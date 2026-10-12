//! Durable, per-installation partner alerts. No credentials or payloads are logged.
use anyhow::{Context, bail};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ring::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Row, SqliteConnection, SqlitePool};
use std::{collections::HashSet, time::Duration};

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub(crate) async fn register(
    pool: &SqlitePool,
    installation: &str,
    token: &str,
    subject: &str,
    session: &str,
    at: i64,
) -> anyhow::Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    // A rotated or transferred token must have exactly one owner.
    sqlx::query("DELETE FROM push_devices WHERE token = ? AND installation_id <> ?")
        .bind(token)
        .bind(installation)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO push_devices (installation_id,subject,session_hash,token,updated_at) VALUES (?,?,?,?,?) ON CONFLICT(installation_id) DO UPDATE SET subject=excluded.subject,session_hash=excluded.session_hash,token=excluded.token,updated_at=excluded.updated_at")
        .bind(installation).bind(subject).bind(session).bind(token).bind(at).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn enqueue(
    tx: &mut SqliteConnection,
    operation: &str,
    transaction: &str,
    actor: &str,
    allowed: &HashSet<String>,
    title: &str,
    body: &str,
    at: i64,
) -> anyhow::Result<()> {
    let recipients: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT d.installation_id,d.subject,d.session_hash FROM push_devices d JOIN pocket_sessions s ON s.id_hash=d.session_hash AND s.subject=d.subject WHERE d.subject <> ? AND s.expires_at > ?")
        .bind(actor).bind(at).fetch_all(&mut *tx).await?;
    for (installation, subject, session) in recipients {
        if !allowed.contains(&subject) {
            continue;
        }
        sqlx::query("INSERT INTO push_outbox (event_id,operation_id,transaction_id,title,body,installation_id,recipient_subject,session_hash,created_at,next_attempt_at) VALUES (?,?,?,?,?,?,?,?,?,?) ON CONFLICT(operation_id,installation_id) DO NOTHING")
            .bind(uuid::Uuid::new_v4().to_string()).bind(operation).bind(transaction)
            .bind(title.chars().take(100).collect::<String>()).bind(body.chars().take(250).collect::<String>()).bind(installation).bind(subject).bind(session).bind(at).bind(at)
            .execute(&mut *tx).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    #[serde(rename = "type")]
    kind: String,
}

pub(crate) struct FcmSender {
    account: ServiceAccount,
    key: RsaKeyPair,
    http: reqwest::Client,
    access_token: Option<(String, i64)>,
    oauth_url: String,
    send_url: String,
}

impl FcmSender {
    pub(crate) fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(path) = std::env::var_os("FCM_SERVICE_ACCOUNT_FILE") else {
            return Ok(None);
        };
        let account: ServiceAccount = serde_json::from_slice(
            &std::fs::read(path).context("Could not read FCM service-account file")?,
        )
        .context("Invalid FCM service-account JSON")?;
        if account.kind != "service_account"
            || account.project_id.is_empty()
            || !account
                .project_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            bail!("Invalid FCM service account project");
        }
        let pem = account
            .private_key
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect::<String>();
        let der = STANDARD
            .decode(pem)
            .context("Invalid FCM private key encoding")?;
        let key = RsaKeyPair::from_pkcs8(&der)
            .map_err(|_| anyhow::anyhow!("Invalid FCM RSA private key"))?;
        let send_url = format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            account.project_id
        );
        Ok(Some(Self {
            account,
            key,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            access_token: None,
            oauth_url: "https://oauth2.googleapis.com/token".into(),
            send_url,
        }))
    }

    async fn token(&mut self) -> anyhow::Result<String> {
        if let Some((token, expiry)) = &self.access_token {
            if *expiry > now() + 60 {
                return Ok(token.clone());
            }
        }
        let at = now();
        let claims = json!({"iss":self.account.client_email,
            "scope":"https://www.googleapis.com/auth/firebase.messaging",
            "aud":"https://oauth2.googleapis.com/token","iat":at,"exp":at+3600});
        let unsigned = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?)
        );
        let mut signature = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                unsigned.as_bytes(),
                &mut signature,
            )
            .map_err(|_| anyhow::anyhow!("Could not sign FCM authentication request"))?;
        let assertion = format!("{unsigned}.{}", URL_SAFE_NO_PAD.encode(signature));
        let response = self
            .http
            .post(&self.oauth_url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &assertion),
            ])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("FCM OAuth transport failed"))?;
        if !response.status().is_success() {
            bail!(
                "FCM OAuth rejected authentication ({})",
                response.status().as_u16()
            );
        }
        let body: Value = response
            .json()
            .await
            .context("Invalid FCM OAuth response")?;
        let token = body["access_token"]
            .as_str()
            .context("Missing FCM access token")?
            .to_owned();
        let lifetime = body["expires_in"].as_i64().unwrap_or(3600).clamp(60, 3600);
        self.access_token = Some((token.clone(), at + lifetime));
        Ok(token)
    }

    async fn send(
        &mut self,
        access_token: &str,
        device: &str,
        event: &str,
        transaction: &str,
        subject: &str,
        session: &str,
        title: &str,
        body: &str,
    ) -> anyhow::Result<Delivery> {
        // Data-only messages let the client enforce opt-out, account identity and deduplication
        // before displaying anything, even while in the background.
        let response = self.http.post(&self.send_url).bearer_auth(access_token).json(&json!({"message":{
            "token":device,"data":{"eventId":event,"transactionId":transaction,"recipientSubject":subject,"recipientSession":session,"title":title,"body":body},
            "android":{"priority":"high","ttl":"86400s"}}}))
            .send().await.map_err(|_| anyhow::anyhow!("FCM send transport failed"))?;
        let status = response.status().as_u16();
        if status == 401 {
            self.access_token = None;
        }
        let minimum = retry_after(
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
            now(),
        );
        if response.status().is_success() {
            return Ok(Delivery::Sent);
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let delivery = classify_failure(status, &body, minimum);
        tracing::warn!(
            status,
            retry = matches!(delivery, Delivery::Retry(_)),
            "FCM delivery rejected"
        );
        Ok(delivery)
    }
}

#[derive(Debug, PartialEq)]
enum Delivery {
    Sent,
    InvalidDevice,
    PermanentFailure,
    Retry(i64),
}

fn retry_after(value: Option<&str>, at: i64) -> i64 {
    value
        .and_then(|v| {
            v.parse::<i64>()
                .ok()
                .filter(|seconds| *seconds >= 0)
                .or_else(|| {
                    chrono::DateTime::parse_from_rfc2822(v)
                        .ok()
                        .map(|date| date.timestamp().saturating_sub(at))
                })
        })
        .unwrap_or(60)
        .max(60)
}

fn retry_delay(minimum: i64, attempts: i64) -> i64 {
    let base = 60 * (1i64 << attempts.clamp(0, 8));
    // Add jitter without shortening either the provider minimum or exponential backoff.
    let random = uuid::Uuid::new_v4().as_u128();
    minimum
        .max(base)
        .saturating_add((random % (base as u128 / 2 + 1)) as i64)
}

fn classify_failure(status: u16, body: &Value, minimum: i64) -> Delivery {
    let has_code = |code: &str| {
        body["error"]["details"].as_array().is_some_and(|details| {
            details.iter().any(|v| {
                v["@type"] == "type.googleapis.com/google.firebase.fcm.v1.FcmError"
                    && v["errorCode"] == code
            })
        })
    };
    if has_code("UNREGISTERED") {
        Delivery::InvalidDevice
    } else if has_code("SENDER_ID_MISMATCH") || status == 400 || status == 403 || status == 404 {
        // Bad payloads and project configuration failures must not delete device registrations.
        Delivery::PermanentFailure
    } else if status == 401 || status == 429 || status >= 500 {
        Delivery::Retry(minimum)
    } else {
        Delivery::PermanentFailure
    }
}

async fn current_device(pool: &SqlitePool, event: &str, at: i64) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar("SELECT d.token FROM push_outbox o JOIN push_devices d ON d.installation_id=o.installation_id AND d.subject=o.recipient_subject AND d.session_hash=o.session_hash JOIN pocket_sessions s ON s.id_hash=d.session_hash AND s.subject=d.subject WHERE o.event_id=? AND s.expires_at>? AND o.created_at>? AND o.attempts<12")
        .bind(event).bind(at).bind(at-86400).fetch_optional(pool).await?)
}

pub(crate) fn start_worker(pool: SqlitePool, mut sender: FcmSender, allowed: HashSet<String>) {
    tokio::spawn(async move {
        loop {
            if drain(&pool, &mut sender, &allowed).await.is_err() {
                // Do not log credentials, device tokens, request bodies or provider responses.
                tracing::warn!("Partner notification worker failed; will retry");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn drain(
    pool: &SqlitePool,
    sender: &mut FcmSender,
    allowed: &HashSet<String>,
) -> anyhow::Result<()> {
    let at = now();
    sqlx::query("DELETE FROM push_outbox WHERE created_at <= ? OR attempts >= 12 OR NOT EXISTS (SELECT 1 FROM push_devices d JOIN pocket_sessions s ON s.id_hash=d.session_hash AND s.subject=d.subject WHERE d.installation_id=push_outbox.installation_id AND d.subject=push_outbox.recipient_subject AND d.session_hash=push_outbox.session_hash AND s.expires_at > ?)")
        .bind(at-86400).bind(at).execute(pool).await?;
    let rows = sqlx::query("SELECT o.* FROM push_outbox o WHERE o.next_attempt_at <= ? ORDER BY o.next_attempt_at, o.created_at LIMIT 20")
        .bind(at).fetch_all(pool).await?;
    for row in rows {
        let event: String = row.try_get("event_id")?;
        let subject: String = row.try_get("recipient_subject")?;
        if !allowed.contains(&subject) {
            sqlx::query("DELETE FROM push_outbox WHERE event_id=?")
                .bind(event)
                .execute(pool)
                .await?;
            continue;
        }
        // Refresh authentication before checking ownership, so OAuth latency cannot stale the check.
        // On an OAuth outage, stop the batch rather than repeating the same request per device.
        let access_token = match sender.token().await {
            Ok(token) => token,
            Err(_) => {
                tracing::warn!("FCM authentication failed; delivery batch deferred");
                sqlx::query("UPDATE push_outbox SET attempts=attempts+CASE WHEN event_id=? THEN 1 ELSE 0 END,next_attempt_at=? WHERE next_attempt_at<=?")
                .bind(&event).bind(now().saturating_add(retry_delay(60, row.try_get("attempts")?))).bind(now()).execute(pool).await?;
                break;
            }
        };
        let Some(token) = current_device(pool, &event, now()).await? else {
            sqlx::query("DELETE FROM push_outbox WHERE event_id=?")
                .bind(&event)
                .execute(pool)
                .await?;
            continue;
        };
        let transaction: String = row.try_get("transaction_id")?;
        let attempts: i64 = row.try_get("attempts")?;
        match sender
            .send(
                &access_token,
                &token,
                &event,
                &transaction,
                &subject,
                &row.try_get::<String, _>("session_hash")?,
                &row.try_get::<String, _>("title")?,
                &row.try_get::<String, _>("body")?,
            )
            .await
            .unwrap_or(Delivery::Retry(60))
        {
            Delivery::Sent | Delivery::PermanentFailure => {
                sqlx::query("DELETE FROM push_outbox WHERE event_id=?")
                    .bind(&event)
                    .execute(pool)
                    .await?;
            }
            Delivery::InvalidDevice => {
                sqlx::query("DELETE FROM push_devices WHERE installation_id=? AND token=?")
                    .bind(row.try_get::<String, _>("installation_id")?)
                    .bind(&token)
                    .execute(pool)
                    .await?;
                sqlx::query("DELETE FROM push_outbox WHERE event_id=?")
                    .bind(&event)
                    .execute(pool)
                    .await?;
            }
            Delivery::Retry(minimum) => {
                let delay = retry_delay(minimum, attempts);
                sqlx::query(
                    "UPDATE push_outbox SET attempts=attempts+1,next_attempt_at=? WHERE event_id=?",
                )
                .bind(now().saturating_add(delay))
                .bind(event)
                .execute(pool)
                .await?;
            }
        }
    }
    sqlx::query("DELETE FROM push_devices WHERE NOT EXISTS (SELECT 1 FROM pocket_sessions s WHERE s.id_hash=push_devices.session_hash AND s.expires_at>?)")
        .bind(at).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_failures_are_classified_without_discarding_good_devices() {
        let error = |code| json!({"error":{"details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":code}]}});
        assert_eq!(
            classify_failure(404, &error("UNREGISTERED"), 60),
            Delivery::InvalidDevice
        );
        assert_eq!(
            classify_failure(400, &error("INVALID_ARGUMENT"), 60),
            Delivery::PermanentFailure
        );
        assert_eq!(
            classify_failure(403, &error("SENDER_ID_MISMATCH"), 60),
            Delivery::PermanentFailure
        );
        for status in [401, 429, 500, 503] {
            assert_eq!(
                classify_failure(status, &Value::Null, 120),
                Delivery::Retry(120)
            );
        }
        assert_eq!(
            classify_failure(
                404,
                &json!({"error":{"details":[{"errorCode":"UNREGISTERED"}]}}),
                60
            ),
            Delivery::PermanentFailure
        );
    }

    #[test]
    fn retry_headers_and_jitter_preserve_provider_minimum() {
        let at = chrono::DateTime::parse_from_rfc2822("Wed, 21 Oct 2015 07:27:00 GMT")
            .unwrap()
            .timestamp();
        assert_eq!(retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT"), at), 60);
        assert_eq!(retry_after(Some("Wed, 21 Oct 2015 07:30:00 GMT"), at), 180);
        assert_eq!(retry_after(Some("172800"), at), 172800);
        for value in [None, Some("invalid"), Some("-1"), Some("0")] {
            assert_eq!(retry_after(value, at), 60);
        }
        for attempts in [-1, 0, 1, 8, 12] {
            let base = 60 * (1i64 << attempts.clamp(0, 8));
            for _ in 0..100 {
                let delay = retry_delay(180, attempts);
                assert!(delay >= base.max(180) && delay <= base.max(180) + base / 2);
            }
        }
    }

    #[tokio::test]
    async fn delivery_rechecks_rotations_transfers_expiry_and_revocation() -> anyhow::Result<()> {
        let pool = database().await?;
        register(&pool, "phone", "old", "jopi", "jopi-session", 100).await?;
        add(&pool, "op", "tim", true).await?;
        let event: String = sqlx::query_scalar("SELECT event_id FROM push_outbox")
            .fetch_one(&pool)
            .await?;
        assert_eq!(
            current_device(&pool, &event, 100).await?.as_deref(),
            Some("old")
        );
        register(&pool, "phone", "new", "jopi", "jopi-session", 101).await?;
        assert_eq!(
            current_device(&pool, &event, 101).await?.as_deref(),
            Some("new")
        );
        register(&pool, "phone", "new", "tim", "tim-session", 102).await?;
        assert!(current_device(&pool, &event, 102).await?.is_none());
        register(&pool, "phone", "new", "jopi", "jopi-session", 103).await?;
        assert!(current_device(&pool, &event, 10_000).await?.is_none());
        sqlx::query("DELETE FROM pocket_sessions WHERE subject='jopi'")
            .execute(&pool)
            .await?;
        assert!(current_device(&pool, &event, 103).await?.is_none());
        Ok(())
    }

    async fn database() -> anyhow::Result<SqlitePool> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!().run(&pool).await?;
        for (id, subject) in [("tim-session", "tim"), ("jopi-session", "jopi")] {
            sqlx::query("INSERT INTO pocket_sessions(id_hash,subject,username,expires_at,created_at) VALUES (?,?,?,?,?)")
                .bind(id).bind(subject).bind(subject).bind(10_000i64).bind(1i64).execute(&pool).await?;
        }
        Ok(pool)
    }

    async fn count(pool: &SqlitePool) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM push_outbox")
            .fetch_one(pool)
            .await?)
    }

    async fn add(
        pool: &SqlitePool,
        operation: &str,
        actor: &str,
        commit: bool,
    ) -> anyhow::Result<()> {
        let mut tx = pool.begin().await?;
        enqueue(
            &mut tx,
            operation,
            "transaction",
            actor,
            &["tim".into(), "jopi".into()].into_iter().collect(),
            "Tim added an expense",
            "24.50 EUR · Groceries",
            100,
        )
        .await?;
        if commit {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn commits_notify_only_partner_devices_and_retry_deduplicates() -> anyhow::Result<()> {
        let pool = database().await?;
        register(&pool, "tim-phone", "tim-token", "tim", "tim-session", 100).await?;
        register(
            &pool,
            "jopi-phone",
            "jopi-token",
            "jopi",
            "jopi-session",
            100,
        )
        .await?;
        register(
            &pool,
            "jopi-tablet",
            "tablet-token",
            "jopi",
            "jopi-session",
            100,
        )
        .await?;
        add(&pool, "failed-op", "tim", false).await?;
        assert_eq!(
            count(&pool).await?,
            0,
            "Rolled-back writes must not send alerts"
        );
        add(&pool, "tim-op", "tim", true).await?;
        add(&pool, "tim-op", "tim", true).await?;
        assert_eq!(
            count(&pool).await?,
            2,
            "One event per partner installation despite lost-ack retries"
        );
        let subjects: Vec<String> = sqlx::query_scalar("SELECT recipient_subject FROM push_outbox")
            .fetch_all(&pool)
            .await?;
        assert!(subjects.iter().all(|s| s == "jopi"));
        add(&pool, "jopi-op", "jopi", true).await?;
        let subject: String = sqlx::query_scalar(
            "SELECT recipient_subject FROM push_outbox WHERE operation_id='jopi-op'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(subject, "tim", "Delivery works in both directions");
        Ok(())
    }

    #[tokio::test]
    async fn token_rotation_and_account_switch_keep_one_owner() -> anyhow::Result<()> {
        let pool = database().await?;
        register(&pool, "phone", "old-token", "jopi", "jopi-session", 100).await?;
        register(&pool, "phone", "new-token", "jopi", "jopi-session", 101).await?;
        let token: String =
            sqlx::query_scalar("SELECT token FROM push_devices WHERE installation_id='phone'")
                .fetch_one(&pool)
                .await?;
        assert_eq!(token, "new-token");
        add(&pool, "before-switch", "tim", true).await?;
        register(&pool, "phone", "new-token", "tim", "tim-session", 102).await?;
        let deliverable: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM push_outbox o JOIN push_devices d ON d.installation_id=o.installation_id AND d.subject=o.recipient_subject AND d.session_hash=o.session_hash").fetch_one(&pool).await?;
        assert_eq!(
            deliverable, 0,
            "Old account alerts must not reach the new owner"
        );
        register(
            &pool,
            "replacement-installation",
            "new-token",
            "jopi",
            "jopi-session",
            103,
        )
        .await?;
        let devices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM push_devices")
            .fetch_one(&pool)
            .await?;
        assert_eq!(
            devices, 1,
            "A reinstalled token cannot remain attached to two installations"
        );
        Ok(())
    }

    #[tokio::test]
    async fn expired_or_revoked_sessions_never_receive_new_events() -> anyhow::Result<()> {
        let pool = database().await?;
        register(&pool, "phone", "token", "jopi", "jopi-session", 100).await?;
        sqlx::query("UPDATE pocket_sessions SET expires_at=100 WHERE subject='jopi'")
            .execute(&pool)
            .await?;
        add(&pool, "expired", "tim", true).await?;
        assert_eq!(count(&pool).await?, 0);
        sqlx::query("DELETE FROM pocket_sessions WHERE subject='jopi'")
            .execute(&pool)
            .await?;
        add(&pool, "revoked", "tim", true).await?;
        assert_eq!(count(&pool).await?, 0);
        Ok(())
    }
}
