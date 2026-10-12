use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL, COOKIE, ORIGIN, SET_COOKIE},
    },
    middleware::{Next, from_fn},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, Column, Row, SqlitePool, TypeInfo, ValueRef};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, warn};
use url::Url;
use uuid::Uuid;

use crate::Config;

const SESSION_COOKIE: &str = "presto_session";
const POCKET_SESSION_COOKIE: &str = "presto_pocket";
const POCKET_OAUTH_COOKIE: &str = "presto_pocket_oauth";
const SESSION_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;
const MOBILE_HANDOFF_TTL_SECONDS: i64 = 90;
const OAUTH_STATE_TTL_SECONDS: i64 = 10 * 60;
const NATIVE_MOBILE_AUTH_REDIRECT_URI: &str = "prestosupuesto-native://pocket-id/callback";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v3/userinfo";
const GATEWAY_URL: &str = "https://script.googleapis.com/v1/scripts";

#[derive(Clone)]
pub(crate) struct AppState {
    config: Config,
    pool: SqlitePool,
    http: reqwest::Client,
    pocket_oidc: PocketOidc,
}

#[derive(Clone)]
struct PocketOidc {
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
}

#[derive(Deserialize)]
struct PocketOidcDiscovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
    code_challenge_methods_supported: Vec<String>,
}

impl AppState {
    pub(crate) async fn new(config: Config, pool: SqlitePool) -> anyhow::Result<Self> {
        if config.finance_db_read_enabled {
            let mut budgets = pool.begin_with("BEGIN IMMEDIATE").await?;
            crate::budget::initialize(&mut budgets).await?;
            crate::budget_ledger::activate(&mut budgets).await?;
            crate::budget_ledger::sync(&mut budgets, None).await?;
            budgets.commit().await?;
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("presto-finance/0.1")
            .build()?;
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            config.pocket_id_issuer
        );
        let discovery = http
            .get(&discovery_url)
            .send()
            .await?
            .error_for_status()?
            .json::<PocketOidcDiscovery>()
            .await?;
        if discovery.issuer.trim_end_matches('/') != config.pocket_id_issuer {
            anyhow::bail!("Pocket ID discovery returned an unexpected issuer.");
        }
        if !discovery
            .code_challenge_methods_supported
            .iter()
            .any(|method| method == "S256")
        {
            anyhow::bail!("Pocket ID must advertise PKCE S256 support.");
        }
        let issuer_url = url::Url::parse(&config.pocket_id_issuer)?;
        for endpoint in [
            &discovery.authorization_endpoint,
            &discovery.token_endpoint,
            &discovery.userinfo_endpoint,
        ] {
            let endpoint_url = url::Url::parse(endpoint)?;
            if endpoint_url.scheme() != "https"
                || endpoint_url.origin() != issuer_url.origin()
                || !endpoint_url.username().is_empty()
                || endpoint_url.password().is_some()
                || endpoint_url.fragment().is_some()
            {
                anyhow::bail!("Pocket ID discovery returned an untrusted endpoint.");
            }
        }
        let pocket_oidc = PocketOidc {
            authorization_endpoint: discovery.authorization_endpoint,
            token_endpoint: discovery.token_endpoint,
            userinfo_endpoint: discovery.userinfo_endpoint,
        };
        Ok(Self {
            config,
            pool,
            http,
            pocket_oidc,
        })
    }
}

pub(crate) fn app(state: AppState, static_dir: &Path) -> anyhow::Result<Router> {
    let index_file = static_dir.join("index.html");
    if !index_file.is_file() {
        anyhow::bail!(
            "frontend build is missing at {}; run npm run web:build",
            static_dir.display()
        );
    }
    let static_files = ServeDir::new(static_dir).not_found_service(ServeFile::new(&index_file));
    Ok(Router::new()
        // Known browser routes serve the SPA with 200; missing assets and unknown routes stay 404.
        .route_service("/budgets", ServeFile::new(&index_file))
        .route_service("/budgets/", ServeFile::new(&index_file))
        .route_service("/history", ServeFile::new(&index_file))
        .route_service("/history/", ServeFile::new(&index_file))
        .route_service("/reconciliation", ServeFile::new(&index_file))
        .route_service("/reconciliation/", ServeFile::new(&index_file))
        .route_service("/transactions", ServeFile::new(&index_file))
        .route_service("/transactions/", ServeFile::new(&index_file))
        .route("/healthz", get(health))
        .route("/auth/pocket-id", get(begin_pocket_id_login))
        .route("/auth/pocket-id/mobile", get(begin_mobile_pocket_id_login))
        .route("/auth/pocket-id/callback", get(pocket_id_callback))
        .route("/auth/google", get(begin_google_login))
        .route("/auth/google/callback", get(google_callback))
        .route("/api/session", get(get_session))
        .route("/api/logout", post(logout))
        .route(
            "/api/mobile/session/exchange",
            post(exchange_mobile_session),
        )
        .route("/api/mobile/logout", post(logout_mobile_session))
        .route("/api/mobile/push-device", post(register_push_device))
        .route("/api/mobile/push-device/remove", post(remove_push_device))
        .route("/api/finance", get(get_finance))
        .route("/api/db/finance", get(get_db_finance))
        .route("/api/db/finance/categories", post(create_budget_category))
        .route("/api/db/finance/monthly-budget", post(save_monthly_budget))
        .route("/api/db/finance/monthly-budget/cancel", post(cancel_monthly_budget))
        .route("/api/db/transactions/history", get(transaction_history))
        .route("/api/db/transactions/detail", get(transaction_detail))
        .route(
            "/api/db/finance/expected-income",
            post(save_expected_income),
        )
        .route("/api/db/finance/backup", get(get_db_finance_backup))
        .route(
            "/api/db/finance/restore",
            post(restore_db_finance).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route("/api/transactions", post(submit_transaction))
        .route("/api/db/transactions", get(list_db_transactions).post(create_db_transaction))
        .route("/api/db/transactions/edit", post(edit_db_transaction))
        .route("/api/db/transactions/delete", post(delete_db_transaction))
        .route("/api/reconciliation", post(reconcile_accounts))
        .route("/api/db/reconciliation", post(record_db_reconciliation))
        .route(
            "/api/db/reconciliation/preview",
            get(get_reconciliation_preview),
        )
        .route(
            "/api/db/category-correction",
            post(record_category_correction),
        )
        .route("/api/budget/close", post(close_budget_month))
        .route("/api/budget/income-complete", post(complete_budget_income))
        .fallback_service(static_files)
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .layer(from_fn(no_store_api_responses))
        .with_state(state))
}

async fn no_store_api_responses(request: Request, next: Next) -> Response {
    let is_api = request.uri().path().starts_with("/api/") || request.uri().path().starts_with("/auth/");
    let mut response = next.run(request).await;
    if is_api {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response.headers_mut().insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert("X-Content-Type-Options", HeaderValue::from_static("nosniff"));
    response.headers_mut().insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    response.headers_mut().insert("Content-Security-Policy", HeaderValue::from_static("frame-ancestors 'none'; base-uri 'self'; object-src 'none'"));
    response
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        error!(error = %error, "finance server request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "The server could not complete this request.",
        )
    }

    fn upstream(error: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, error)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(json!({ "error": self.message }))).into_response();
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionInfo {
    authenticated: bool,
    email: Option<String>,
    pocket_authenticated: bool,
    pocket_username: Option<String>,
    pocket_subject: Option<String>,
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Deserialize)]
struct OAuthCallback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct PocketUserInfo {
    sub: String,
    preferred_username: String,
}

#[derive(Deserialize)]
struct PocketTokenResponse {
    access_token: String,
}

struct PocketSession {
    cookie_id: String,
    username: String,
}

struct PocketIdentity {
    subject: String,
    username: String,
    mobile_redirect_uri: Option<String>,
    mobile_code_challenge: Option<String>,
}

struct ActivePocketSession {
    id_hash: String,
    subject: String,
    username: String,
    expires_at: i64,
}

async fn begin_pocket_id_login(State(state): State<AppState>) -> Result<Response, ApiError> {
    let url = create_pocket_id_authorization(&state, None, None).await?;
    pocket_authorization_response(&state, &url)
}

#[derive(Deserialize)]
struct MobileLoginQuery {
    client: Option<String>,
    code_challenge: Option<String>,
}

fn mobile_client_redirect(client: Option<&str>) -> Result<&'static str, ApiError> {
    match client {
        None | Some("native-android") => Ok(NATIVE_MOBILE_AUTH_REDIRECT_URI),
        _ => Err(ApiError::new(StatusCode::BAD_REQUEST, "Unknown mobile client.")),
    }
}

async fn begin_mobile_pocket_id_login(
    State(state): State<AppState>,
    Query(query): Query<MobileLoginQuery>,
) -> Result<Response, ApiError> {
    let redirect = mobile_client_redirect(query.client.as_deref())?;
    let challenge = query.code_challenge.as_deref().filter(|value| valid_pkce_challenge(value))
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "A valid mobile S256 code challenge is required."))?;
    let url = create_pocket_id_authorization(&state, Some(redirect), Some(challenge)).await?;
    pocket_authorization_response(&state, &url)
}

async fn create_pocket_id_authorization(
    state: &AppState,
    mobile_redirect_uri: Option<&str>,
    mobile_code_challenge: Option<&str>,
) -> Result<String, ApiError> {
    let oauth_state = random_id();
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let expires_at = now_seconds() + OAUTH_STATE_TTL_SECONDS;
    sqlx::query("DELETE FROM pocket_id_states WHERE expires_at <= ?")
        .bind(now_seconds())
        .execute(&state.pool)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO pocket_id_states(state, code_verifier, expires_at, mobile_redirect_uri, mobile_code_challenge) VALUES (?, ?, ?, ?, ?)")
        .bind(&oauth_state)
        .bind(&verifier)
        .bind(expires_at)
        .bind(mobile_redirect_uri)
        .bind(mobile_code_challenge)
        .execute(&state.pool)
        .await
        .map_err(ApiError::internal)?;

    let mut url =
        Url::parse(&state.pocket_oidc.authorization_endpoint).map_err(ApiError::internal)?;
    url.query_pairs_mut()
        .append_pair("client_id", &state.config.pocket_id_client_id)
        .append_pair("redirect_uri", &state.config.pocket_id_redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", "openid profile email")
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "select_account");
    Ok(url.to_string())
}

fn valid_pkce_challenge(value: &str) -> bool {
    value.len() == 43 && URL_SAFE_NO_PAD.decode(value).is_ok_and(|bytes| bytes.len() == 32)
}

fn pocket_authorization_response(state: &AppState, url: &str) -> Result<Response, ApiError> {
    let parsed = Url::parse(url).map_err(ApiError::internal)?;
    let oauth_state = parsed.query_pairs().find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned()).ok_or_else(|| ApiError::internal("missing OAuth state"))?;
    let secure = if state.config.cookie_secure { "; Secure" } else { "" };
    let mut response = Redirect::temporary(url).into_response();
    response.headers_mut().append(SET_COOKIE, HeaderValue::from_str(&format!(
        "{POCKET_OAUTH_COOKIE}={oauth_state}; Path=/; HttpOnly; SameSite=Lax; Max-Age={OAUTH_STATE_TTL_SECONDS}{secure}"
    )).map_err(ApiError::internal)?);
    Ok(response)
}

async fn pocket_id_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallback>,
    headers: HeaderMap,
) -> Response {
    let oauth_state = query.state.clone().unwrap_or_default();
    let mut response = match finish_pocket_id_login(&state, query, &headers).await {
        Ok(identity) => {
            tracing::info!(username = %identity.username, "Pocket ID authentication completed");
            if let Some(redirect_uri) = identity.mobile_redirect_uri.as_deref() {
                match issue_pocket_login(&state, &oauth_state, &identity, &headers).await {
                    Ok(handoff) => {
                        let mut redirect = match Url::parse(redirect_uri) {
                            Ok(redirect) => redirect,
                            Err(_) => {
                                return pocket_callback_error(ApiError::internal(
                                    "invalid mobile redirect URI",
                                ));
                            }
                        };
                        redirect.query_pairs_mut().append_pair("handoff", &handoff);
                        let mut response = Redirect::temporary(redirect.as_str()).into_response();
                        response
                            .headers_mut()
                            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
                        response
                            .headers_mut()
                            .insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
                        response
                    }
                    Err(error) => pocket_callback_error(error),
                }
            } else {
                match issue_pocket_login(&state, &oauth_state, &identity, &headers).await {
                    Ok(session_id) => {
                        let mut response = Redirect::to("/").into_response();
                        set_pocket_session_cookie(&mut response, &state.config, &session_id);
                        response
                    }
                    Err(error) => pocket_callback_error(error),
                }
            }
        }
        Err(error) => pocket_callback_error(error),
    };
    let secure = if state.config.cookie_secure { "; Secure" } else { "" };
    response.headers_mut().append(SET_COOKIE, HeaderValue::from_str(&format!(
        "{POCKET_OAUTH_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}"
    )).expect("static OAuth cookie is valid"));
    response
}

async fn finish_pocket_id_login(
    state: &AppState,
    query: OAuthCallback,
    headers: &HeaderMap,
) -> Result<PocketIdentity, ApiError> {
    let browser_state = cookie_id_from_headers(headers, POCKET_OAUTH_COOKIE);
    if browser_state.is_none() || query.state.as_deref() != browser_state.as_deref() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "This sign-in request was not started in this browser. Please sign in again."));
    }
    if query.error.is_some() {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Pocket ID sign-in was cancelled or denied.",
        ));
    }
    let oauth_state = query
        .state
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "Missing Pocket ID state."))?;
    let code = query
        .code
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "Missing Pocket ID code."))?;
    let state_row = sqlx::query(
        "SELECT code_verifier, mobile_redirect_uri, mobile_code_challenge FROM pocket_id_states WHERE state = ? AND expires_at > ?",
    )
    .bind(&oauth_state)
    .bind(now_seconds())
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "The Pocket ID sign-in request expired. Please try again.",
        )
    })?;
    let verifier: String = state_row
        .try_get("code_verifier")
        .map_err(ApiError::internal)?;
    let mobile_redirect_uri: Option<String> = state_row
        .try_get("mobile_redirect_uri")
        .map_err(ApiError::internal)?;
    let mobile_code_challenge: Option<String> = state_row.try_get("mobile_code_challenge").map_err(ApiError::internal)?;
    let token_response = state
        .http
        .post(&state.pocket_oidc.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", state.config.pocket_id_client_id.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", state.config.pocket_id_redirect_uri.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|_| ApiError::upstream("Could not contact Pocket ID."))?
        .error_for_status()
        .map_err(|_| ApiError::upstream("Pocket ID rejected the sign-in response."))?
        .json::<PocketTokenResponse>()
        .await
        .map_err(|_| ApiError::upstream("Pocket ID returned an invalid token response."))?;
    let info = state
        .http
        .get(&state.pocket_oidc.userinfo_endpoint)
        .bearer_auth(&token_response.access_token)
        .send()
        .await
        .map_err(|_| ApiError::upstream("Could not verify the Pocket ID account."))?
        .error_for_status()
        .map_err(|_| ApiError::upstream("Pocket ID could not verify the account."))?
        .json::<PocketUserInfo>()
        .await
        .map_err(|_| ApiError::upstream("Pocket ID returned invalid account information."))?;
    let subject = info.sub.trim().to_string();
    let username = info.preferred_username.trim().to_ascii_lowercase();
    if subject.is_empty() || !state.config.pocket_id_allowed_subjects.contains(&subject) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This Pocket ID account is not allowed to use the finance workspace.",
        ));
    }
    Ok(PocketIdentity {
        subject,
        username,
        mobile_redirect_uri,
        mobile_code_challenge,
    })
}

// Keep the state cancellable while the provider request is in flight. Consume it
// in the same transaction that issues credentials and revokes the previous session.
async fn issue_pocket_login(
    state: &AppState,
    oauth_state: &str,
    identity: &PocketIdentity,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    let consumed = sqlx::query("DELETE FROM pocket_id_states WHERE state = ? AND expires_at > ?")
        .bind(oauth_state).bind(now_seconds()).execute(&mut *transaction).await.map_err(ApiError::internal)?;
    if consumed.rows_affected() != 1 {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "This sign-in request expired or was cancelled. Please sign in again."));
    }
    let credential = if identity.mobile_redirect_uri.is_some() {
        create_mobile_handoff_in(&mut transaction, identity).await?
    } else {
        if let Some(previous) = cookie_id_from_headers(headers, POCKET_SESSION_COOKIE) {
            revoke_pocket_session_in(&mut transaction, &hash_session_id(&previous)).await?;
        }
        create_pocket_session_in(&mut transaction, &identity.subject, &identity.username).await?.cookie_id
    };
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(credential)
}

async fn create_pocket_session(
    state: &AppState,
    subject: &str,
    username: &str,
) -> Result<PocketSession, ApiError> {
    let mut connection = state.pool.acquire().await.map_err(ApiError::internal)?;
    create_pocket_session_in(&mut connection, subject, username).await
}

async fn create_pocket_session_in(
    connection: &mut sqlx::SqliteConnection,
    subject: &str,
    username: &str,
) -> Result<PocketSession, ApiError> {
    let cookie_id = random_id();
    let now = now_seconds();
    sqlx::query("DELETE FROM pocket_sessions WHERE expires_at <= ?")
        .bind(now)
        .execute(&mut *connection)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO pocket_sessions(id_hash, subject, username, expires_at, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(hash_session_id(&cookie_id))
        .bind(subject)
        .bind(username)
        .bind(now + SESSION_TTL_SECONDS)
        .bind(now)
        .execute(&mut *connection)
        .await
        .map_err(ApiError::internal)?;
    Ok(PocketSession {
        cookie_id,
        username: username.to_string(),
    })
}

#[cfg(test)]
async fn create_mobile_handoff(
    state: &AppState,
    identity: &PocketIdentity,
) -> Result<String, ApiError> {
    let mut connection = state.pool.acquire().await.map_err(ApiError::internal)?;
    create_mobile_handoff_in(&mut connection, identity).await
}

async fn create_mobile_handoff_in(
    connection: &mut sqlx::SqliteConnection,
    identity: &PocketIdentity,
) -> Result<String, ApiError> {
    if !identity.mobile_code_challenge.as_deref().is_some_and(valid_pkce_challenge) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "A mobile S256 code challenge is required."));
    }
    let handoff = random_id();
    let now = now_seconds();
    sqlx::query("DELETE FROM pocket_mobile_handoffs WHERE expires_at <= ?")
        .bind(now)
        .execute(&mut *connection)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO pocket_mobile_handoffs(code_hash, subject, username, expires_at, created_at, code_challenge) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(hash_session_id(&handoff))
        .bind(&identity.subject)
        .bind(&identity.username)
        .bind(now + MOBILE_HANDOFF_TTL_SECONDS)
        .bind(now)
        .bind(&identity.mobile_code_challenge)
        .execute(&mut *connection)
        .await
        .map_err(ApiError::internal)?;
    Ok(handoff)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MobileSessionExchangeRequest {
    handoff_code: String,
    code_verifier: String,
}

async fn exchange_mobile_session(
    State(state): State<AppState>,
    Json(mut payload): Json<MobileSessionExchangeRequest>,
) -> Result<Json<Value>, ApiError> {
    payload.handoff_code = payload.handoff_code.trim().to_ascii_lowercase();
    if payload.handoff_code.len() != 64
        || !payload
            .handoff_code
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "The mobile sign-in handoff is invalid.",
        ));
    }
    if !(43..=128).contains(&payload.code_verifier.len()) || !payload.code_verifier.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "The mobile code verifier is invalid."));
    }
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(payload.code_verifier.as_bytes()));
    let now = now_seconds();
    sqlx::query("DELETE FROM pocket_mobile_handoffs WHERE expires_at <= ?")
        .bind(now)
        .execute(&state.pool)
        .await
        .map_err(ApiError::internal)?;
    let row = sqlx::query("DELETE FROM pocket_mobile_handoffs WHERE code_hash = ? AND expires_at > ? AND code_challenge = ? RETURNING subject, username")
        .bind(hash_session_id(&payload.handoff_code))
        .bind(now)
        .bind(challenge)
        .fetch_optional(&state.pool)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "The mobile sign-in handoff has expired. Please sign in again."))?;
    let subject: String = row.try_get("subject").map_err(ApiError::internal)?;
    let username: String = row.try_get("username").map_err(ApiError::internal)?;
    if !state.config.pocket_id_allowed_subjects.contains(&subject) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This Pocket ID account is not allowed to use the finance workspace.",
        ));
    }
    let session = create_pocket_session(&state, &subject, &username).await?;
    let expires_at = now + SESSION_TTL_SECONDS;
    Ok(Json(json!({
        "accessToken": session.cookie_id,
        "subject": subject,
        "username": session.username,
        "expiresAt": expires_at,
    })))
}

async fn revoke_pocket_session(state: &AppState, id_hash: &str) -> Result<(), ApiError> {
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    revoke_pocket_session_in(&mut transaction, id_hash).await?;
    transaction.commit().await.map_err(ApiError::internal)
}

async fn revoke_pocket_session_in(connection: &mut sqlx::SqliteConnection, id_hash: &str) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM oauth_states WHERE pocket_session_hash = ?")
        .bind(id_hash).execute(&mut *connection).await.map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM sessions WHERE pocket_session_hash = ?")
        .bind(id_hash).execute(&mut *connection).await.map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM push_devices WHERE session_hash = ?")
        .bind(id_hash).execute(&mut *connection).await.map_err(ApiError::internal)?;
    sqlx::query("DELETE FROM pocket_sessions WHERE id_hash = ?")
        .bind(id_hash).execute(&mut *connection).await.map_err(ApiError::internal)?;
    Ok(())
}

async fn logout_mobile_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let token = bearer_session_id_from_headers(&headers)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "A mobile bearer session is required."))?;
    revoke_pocket_session(&state, &hash_session_id(&token)).await?;
    Ok(Json(json!({ "signedOut": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PushDeviceRequest {
    installation_id: String,
    token: String,
}

async fn register_push_device(State(state): State<AppState>, headers: HeaderMap,
    Json(payload): Json<PushDeviceRequest>) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    if bearer_session_id_from_headers(&headers).is_none() {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "A mobile bearer session is required."));
    }
    let session = current_pocket_session(&state, &headers).await?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Please sign in again."))?;
    if Uuid::parse_str(&payload.installation_id).is_err() || payload.token.len() < 20
        || payload.token.len() > 4096 || payload.token.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "Invalid notification device."));
    }
    crate::push::register(&state.pool, &payload.installation_id, &payload.token,
        &session.subject, &session.id_hash, now_seconds()).await.map_err(ApiError::internal)?;
    Ok(Json(json!({"registered": true})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemovePushDeviceRequest { installation_id: String }

async fn remove_push_device(State(state): State<AppState>, headers: HeaderMap,
    Json(payload): Json<RemovePushDeviceRequest>) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let session = current_pocket_session(&state, &headers).await?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Please sign in again."))?;
    sqlx::query("DELETE FROM push_devices WHERE installation_id = ? AND subject = ? AND session_hash = ?")
        .bind(payload.installation_id).bind(session.subject).bind(session.id_hash)
        .execute(&state.pool).await.map_err(ApiError::internal)?;
    Ok(Json(json!({"removed": true})))
}

fn pocket_callback_error(error: ApiError) -> Response {
    warn!(status = %error.status, message = %error.message, "Pocket ID sign-in failed");
    Redirect::to("/?auth_error=pocket_id_login_failed").into_response()
}

async fn begin_google_login(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Some(pocket_session) = current_pocket_session(&state, &headers).await? else {
        return Ok(Redirect::temporary("/auth/pocket-id").into_response());
    };
    let url = create_google_authorization(&state, false, &pocket_session.id_hash).await?;
    Ok(Redirect::temporary(&url).into_response())
}

async fn create_google_authorization(
    state: &AppState,
    force_consent: bool,
    pocket_session_hash: &str,
) -> Result<String, ApiError> {
    let oauth_state = random_id();
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let expires_at = now_seconds() + OAUTH_STATE_TTL_SECONDS;
    sqlx::query("DELETE FROM oauth_states WHERE expires_at <= ?")
        .bind(now_seconds())
        .execute(&state.pool)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO oauth_states(state, code_verifier, force_consent, expires_at, pocket_session_hash) VALUES (?, ?, ?, ?, ?)")
        .bind(&oauth_state)
        .bind(&verifier)
        .bind(force_consent)
        .bind(expires_at)
        .bind(pocket_session_hash)
        .execute(&state.pool)
        .await
        .map_err(ApiError::internal)?;

    let mut url = Url::parse(GOOGLE_AUTH_URL).map_err(ApiError::internal)?;
    url.query_pairs_mut()
        .append_pair("client_id", &state.config.google_client_id)
        .append_pair("redirect_uri", &state.config.google_redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", "https://www.googleapis.com/auth/spreadsheets https://www.googleapis.com/auth/userinfo.email")
        .append_pair("access_type", "offline")
        .append_pair("include_granted_scopes", "true")
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    if force_consent {
        url.query_pairs_mut().append_pair("prompt", "consent");
    } else {
        url.query_pairs_mut()
            .append_pair("prompt", "select_account");
    }
    Ok(url.to_string())
}

async fn google_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallback>,
    headers: HeaderMap,
) -> Response {
    let Ok(Some(pocket_session)) = current_pocket_session(&state, &headers).await else {
        return Redirect::temporary("/auth/pocket-id").into_response();
    };
    match finish_google_login(&state, query, &pocket_session).await {
        Ok(CallbackResult::SignedIn(session_id)) => {
            let mut response = Redirect::to("/").into_response();
            set_session_cookie(&mut response, &state.config, &session_id);
            response
        }
        Ok(CallbackResult::NeedConsent(pocket_session_hash)) => {
            match create_google_authorization(&state, true, &pocket_session_hash).await {
                Ok(url) => Redirect::temporary(&url).into_response(),
                Err(error) => callback_error(error),
            }
        }
        Err(error) => callback_error(error),
    }
}

enum CallbackResult {
    SignedIn(String),
    NeedConsent(String),
}

async fn finish_google_login(
    state: &AppState,
    query: OAuthCallback,
    pocket_session: &ActivePocketSession,
) -> Result<CallbackResult, ApiError> {
    if query.error.is_some() {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Google sign-in was cancelled or denied.",
        ));
    }
    let oauth_state = query
        .state
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "Missing OAuth state."))?;
    let code = query
        .code
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "Missing authorization code."))?;
    let state_row = sqlx::query(
        "DELETE FROM oauth_states WHERE state = ? AND expires_at > ? AND pocket_session_hash = ? RETURNING code_verifier, force_consent, pocket_session_hash",
    )
    .bind(&oauth_state)
    .bind(now_seconds())
    .bind(&pocket_session.id_hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "The sign-in request expired. Please try again."))?;
    let verifier: String = state_row
        .try_get("code_verifier")
        .map_err(ApiError::internal)?;
    let force_consent: bool = state_row
        .try_get("force_consent")
        .map_err(ApiError::internal)?;
    let pocket_session_hash: String = state_row
        .try_get("pocket_session_hash")
        .map_err(ApiError::internal)?;
    if pocket_session_hash != pocket_session.id_hash || pocket_session.expires_at <= now_seconds() {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Pocket ID authentication expired. Sign in again.",
        ));
    }

    let token_response = request_google_token(
        state,
        &[
            ("code", code.as_str()),
            ("client_id", state.config.google_client_id.as_str()),
            ("client_secret", state.config.google_client_secret.as_str()),
            ("redirect_uri", state.config.google_redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ],
    )
    .await?;
    let email = fetch_google_email(state, &token_response.access_token).await?;
    if !state
        .config
        .allowed_emails
        .iter()
        .any(|allowed| allowed == &email)
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This Google account is not on the household allowlist.",
        ));
    }

    let stored_refresh: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT refresh_token FROM google_accounts WHERE email = ?")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await
            .map_err(ApiError::internal)?;
    let refresh_token = if let Some(refresh) = token_response.refresh_token.as_ref() {
        Some(refresh.as_bytes().to_vec())
    } else if let Some(encrypted) = stored_refresh.as_deref() {
        Some(decrypt_secret(state, encrypted)?)
    } else {
        None
    };
    let Some(refresh_token) = refresh_token else {
        if !force_consent {
            return Ok(CallbackResult::NeedConsent(pocket_session_hash));
        }
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Google did not issue an offline refresh token. Check the OAuth consent configuration and try again.",
        ));
    };

    let encrypted_refresh = encrypt_secret(state, &refresh_token)?;
    sqlx::query(
        "INSERT INTO google_accounts(email, refresh_token, updated_at) VALUES (?, ?, ?) \
         ON CONFLICT(email) DO UPDATE SET refresh_token = excluded.refresh_token, updated_at = excluded.updated_at",
    )
    .bind(&email)
    .bind(encrypted_refresh)
    .bind(now_seconds())
    .execute(&state.pool)
    .await
    .map_err(ApiError::internal)?;

    let session_id = insert_google_session(state, pocket_session, &email, &token_response.access_token, token_response.expires_in).await?;
    Ok(CallbackResult::SignedIn(session_id))
}

async fn insert_google_session(state: &AppState, pocket: &ActivePocketSession, email: &str, access_token: &str, expires_in: i64) -> Result<String, ApiError> {
    let session_id = random_id();
    let session_hash = hash_session_id(&session_id);
    let access_token_cipher = encrypt_secret(state, access_token.as_bytes())?;
    let now = now_seconds();
    let inserted = sqlx::query(
        "INSERT INTO sessions(id_hash, email, access_token, access_expires_at, expires_at, created_at, pocket_session_hash) \
         SELECT ?, ?, ?, ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM pocket_sessions WHERE id_hash = ? AND subject = ? AND expires_at > ?)",
    )
    .bind(session_hash)
    .bind(email)
    .bind(access_token_cipher)
    .bind(now + expires_in)
    .bind(now + SESSION_TTL_SECONDS)
    .bind(now)
    .bind(&pocket.id_hash)
    .bind(&pocket.id_hash)
    .bind(&pocket.subject)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(ApiError::internal)?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Pocket ID authentication expired. Sign in again."));
    }
    Ok(session_id)
}

fn callback_error(error: ApiError) -> Response {
    warn!(status = %error.status, message = %error.message, "Google sign-in failed");
    Redirect::to("/?auth_error=login_failed").into_response()
}

#[derive(Debug)]
struct GoogleTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}

async fn request_google_token(
    state: &AppState,
    params: &[(&str, &str)],
) -> Result<GoogleTokenResponse, ApiError> {
    let response = state
        .http
        .post(GOOGLE_TOKEN_URL)
        .form(params)
        .send()
        .await
        .map_err(|error| {
            ApiError::upstream(format!("Could not reach Google's token service: {error}"))
        })?;
    let status = response.status();
    let body = response.json::<Value>().await.map_err(|error| {
        ApiError::upstream(format!(
            "Google returned an unreadable token response: {error}"
        ))
    })?;
    parse_google_token_response(status, body)
}

fn parse_google_token_response(status: StatusCode, body: Value) -> Result<GoogleTokenResponse, ApiError> {
    if !status.is_success() {
        if status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS && body.get("error").and_then(Value::as_str) == Some("invalid_grant") {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Google authorization expired. Sign in again."));
        }
        return Err(ApiError::upstream("Google's token service could not complete authorization. Please retry."));
    }
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::upstream("Google did not return an access token."))?
        .to_string();
    let expires_in = body
        .get("expires_in")
        .and_then(Value::as_i64)
        .unwrap_or(3600)
        .clamp(60, 86_400);
    let refresh_token = body
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(GoogleTokenResponse {
        access_token,
        refresh_token,
        expires_in,
    })
}

#[derive(Deserialize)]
struct GoogleUserInfo {
    email: String,
    #[serde(default, alias = "verified_email")]
    email_verified: bool,
}

async fn fetch_google_email(state: &AppState, access_token: &str) -> Result<String, ApiError> {
    let response = state
        .http
        .get(GOOGLE_USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| {
            ApiError::upstream(format!("Could not reach Google's account service: {error}"))
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Google could not verify the selected account.",
        ));
    }
    let info = response
        .json::<GoogleUserInfo>()
        .await
        .map_err(|error| ApiError::upstream(error.to_string()))?;
    if !info.email_verified {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Google did not verify the selected email address.",
        ));
    }
    Ok(info.email.trim().to_ascii_lowercase())
}

async fn get_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SessionInfo>, ApiError> {
    let Some(pocket_session) = current_pocket_session(&state, &headers).await? else {
        return Ok(Json(SessionInfo {
            authenticated: false,
            email: None,
            pocket_authenticated: false,
            pocket_username: None,
            pocket_subject: None,
        }));
    };
    let Some(session_id) = session_id_from_headers(&headers) else {
        return Ok(Json(SessionInfo {
            authenticated: false,
            email: None,
            pocket_authenticated: true,
            pocket_username: Some(pocket_session.username),
            pocket_subject: Some(pocket_session.subject),
        }));
    };
    let session_hash = hash_session_id(&session_id);
    let row = sqlx::query(
        "SELECT email, expires_at, pocket_session_hash FROM sessions WHERE id_hash = ?",
    )
    .bind(session_hash)
    .fetch_optional(&state.pool)
    .await;
    let row = row.map_err(ApiError::internal)?;
    let Some(row) = row else {
        return Ok(Json(SessionInfo {
            authenticated: false,
            email: None,
            pocket_authenticated: true,
            pocket_username: Some(pocket_session.username),
            pocket_subject: Some(pocket_session.subject),
        }));
    };
    let email = row.try_get::<String, _>("email").ok();
    let expires_at = row.try_get::<i64, _>("expires_at").unwrap_or(0);
    let linked_pocket_hash = row.try_get::<String, _>("pocket_session_hash").ok();
    let email = email.filter(|email| {
        expires_at > now_seconds()
            && linked_pocket_hash.as_deref() == Some(pocket_session.id_hash.as_str())
            && state
                .config
                .allowed_emails
                .iter()
                .any(|allowed| allowed == email)
    });
    Ok(Json(SessionInfo {
        authenticated: email.is_some(),
        email,
        pocket_authenticated: true,
        pocket_username: Some(pocket_session.username),
        pocket_subject: Some(pocket_session.subject),
    }))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    ensure_same_origin(&state, &headers)?;
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    if let Some(session_id) = session_id_from_headers(&headers) {
        sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
            .bind(hash_session_id(&session_id))
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
    }
    if let Some(pocket_session_id) = cookie_id_from_headers(&headers, POCKET_SESSION_COOKIE) {
        revoke_pocket_session_in(&mut transaction, &hash_session_id(&pocket_session_id)).await?;
    }
    if let Some(oauth_state) = cookie_id_from_headers(&headers, POCKET_OAUTH_COOKIE) {
        sqlx::query("DELETE FROM pocket_id_states WHERE state = ?")
            .bind(oauth_state).execute(&mut *transaction).await.map_err(ApiError::internal)?;
    }
    transaction.commit().await.map_err(ApiError::internal)?;
    let mut response = Json(json!({ "signedOut": true })).into_response();
    let secure = if state.config.cookie_secure { "; Secure" } else { "" };
    response.headers_mut().append(SET_COOKIE, HeaderValue::from_str(&format!(
        "{POCKET_OAUTH_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}"
    )).map_err(ApiError::internal)?);
    clear_session_cookie(&mut response, &state.config);
    clear_pocket_session_cookie(&mut response, &state.config);
    Ok(response)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinanceQuery {
    as_of: String,
    recent_count: Option<u16>,
}

async fn get_finance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FinanceQuery>,
) -> Result<Json<Value>, ApiError> {
    if !valid_date(&query.as_of) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "asOf must be a valid YYYY-MM-DD date.",
        ));
    }
    let recent_count = query.recent_count.unwrap_or(200);
    if recent_count > 200 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "recentCount must be between 0 and 200.",
        ));
    }
    let payload = json!({ "asOf": query.as_of, "recentCount": recent_count });
    let result = run_gateway(&state, &headers, "getFinanceSnapshot", payload).await?;
    Ok(Json(result))
}

async fn get_db_finance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FinanceQuery>,
) -> Result<Json<Value>, ApiError> {
    current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_read_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database finance reads are not enabled.",
        ));
    }
    if !valid_date(&query.as_of) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "asOf must be a valid YYYY-MM-DD date.",
        ));
    }
    let recent_count = query.recent_count.unwrap_or(200);
    if recent_count > 200 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "recentCount must be between 0 and 200.",
        ));
    }
    let mut ledger = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::internal)?;
    crate::budget_ledger::sync(&mut ledger, None)
        .await
        .map_err(ApiError::internal)?;
    ledger.commit().await.map_err(ApiError::internal)?;
    let snapshot =
        crate::finance_read_model::load(&state.pool, &query.as_of, u32::from(recent_count))
            .await
            .map_err(ApiError::internal)?;
    Ok(Json(
        serde_json::to_value(snapshot).map_err(ApiError::internal)?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransactionListQuery {
    month: Option<String>,
    #[serde(default)]
    search: String,
    kind: Option<String>,
    category: Option<String>,
    account: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
    sort: Option<String>,
    direction: Option<String>,
}

#[derive(Deserialize)]
struct TransactionDetailQuery { id: String }

async fn transaction_detail(State(state): State<AppState>, headers: HeaderMap,
    Query(query): Query<TransactionDetailQuery>) -> Result<Json<Value>, ApiError> {
    current_pocket_session(&state, &headers).await?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Please sign in again."))?;
    if !state.config.finance_db_read_enabled {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "Database finance reads are not enabled."));
    }
    if Uuid::parse_str(&query.id).is_err() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "Invalid transaction ID."));
    }
    let month = chrono::Utc::now().with_timezone(&chrono_tz::Europe::Madrid).format("%Y-%m").to_string();
    let row = crate::finance_read_model::transaction(&state.pool, &query.id, &month)
        .await.map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "This transaction is no longer available."))?;
    Ok(Json(serde_json::to_value(row).map_err(ApiError::internal)?))
}

async fn list_db_transactions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TransactionListQuery>,
) -> Result<Json<Value>, ApiError> {
    current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_read_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database finance reads are not enabled.",
        ));
    }
    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(50);
    if query
        .month
        .as_deref()
        .is_some_and(|month| !valid_month(month))
        || page == 0
        || !(1..=100).contains(&page_size)
        || query.search.len() > 500
        || query.kind.as_deref().is_some_and(|kind| {
            ![
                "expense",
                "income",
                "transfer",
                "reallocate",
                "reconciliation",
            ]
            .contains(&kind)
        })
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Choose a valid month, type, search (up to 500 characters), and page size (1–100).",
        ));
    }
    let sort = query.sort.as_deref().unwrap_or("date");
    let direction = query.direction.as_deref().unwrap_or("desc");
    if !["date", "description", "type", "amount", "id"].contains(&sort)
        || !["asc", "desc"].contains(&direction)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Choose a valid transaction sort and direction.",
        ));
    }
    let result = crate::finance_read_model::transactions_sorted(
        &state.pool,
        query.month.as_deref(),
        query.search.trim(),
        query.kind.as_deref(),
        query.category.as_deref(),
        query.account.as_deref(),
        page,
        page_size,
        sort,
        direction,
    )
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(
        serde_json::to_value(result).map_err(ApiError::internal)?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MonthlyBudgetRequest {
    effective_start_month: String,
    categories: Vec<MonthlyBudgetAmount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MonthlyBudgetAmount {
    name: String,
    amount: String,
}

async fn transaction_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Sign in to continue."))?;
    if !state.config.finance_db_read_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Finance history is unavailable.",
        ));
    }
    let id = query
        .get("id")
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "Choose a transaction."))?;
    let rows = sqlx::query("SELECT r.revision,r.recorded_at,COALESCE(json_extract(r.snapshot_json,'$.recordedByName'), (SELECT username FROM pocket_sessions WHERE subject=o.actor_subject ORDER BY created_at DESC LIMIT 1),o.actor_subject) AS actor_subject,r.snapshot_json FROM finance_transaction_revisions r LEFT JOIN finance_operations o ON o.operation_id=r.operation_id WHERE r.transaction_id=? ORDER BY r.revision")
        .bind(id).fetch_all(&state.pool).await.map_err(ApiError::internal)?;
    let account_names: std::collections::HashMap<String, String> =
        sqlx::query_as("SELECT id,source_name FROM finance_accounts")
            .fetch_all(&state.pool)
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .collect();
    let category_names: std::collections::HashMap<String, String> =
        sqlx::query_as("SELECT id,name FROM finance_categories")
            .fetch_all(&state.pool)
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .collect();
    let history: Vec<Value> = rows.into_iter().map(|row| -> Result<Value, ApiError> {
        let snapshot: String = row.try_get("snapshot_json").map_err(ApiError::internal)?;
        let mut details = serde_json::from_str::<Value>(&snapshot).map_err(ApiError::internal)?;
        let key = if details.get("request").is_some() { "request" } else { "transaction" };
        if let Some(request) = details.get_mut(key).and_then(Value::as_object_mut) {
            for (id_key,name_key,names) in [("fromAccountId","fromAccountName",&account_names),("toAccountId","toAccountName",&account_names),("fromCategoryId","fromCategoryName",&category_names),("toCategoryId","toCategoryName",&category_names)] {
                if let Some(name) = request.get(id_key).and_then(Value::as_str).and_then(|id| names.get(id)).cloned() { request.insert(name_key.to_owned(), json!(name)); }
            }
        }
        Ok(json!({"revision": row.try_get::<i64,_>("revision").map_err(ApiError::internal)?, "at": row.try_get::<String,_>("recorded_at").map_err(ApiError::internal)?, "by": row.try_get::<Option<String>,_>("actor_subject").map_err(ApiError::internal)?, "details": details}))
    }).collect::<Result<_,_>>()?;
    Ok(Json(json!({"history": history})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateBudgetCategoryRequest {
    name: String,
    monthly_amount: String,
}

async fn create_budget_category(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateBudgetCategoryRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let actor = current_pocket_session(&state, &headers).await?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Sign in to continue."))?;
    if !state.config.finance_db_transaction_writes_enabled {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "Finance changes are unavailable."));
    }
    let name = payload.name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) || payload.monthly_amount.len() > 64 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "Enter a category name of 1–80 characters and a valid monthly amount."));
    }
    let amount = crate::finance_model::round_posted_decimal_text(&payload.monthly_amount, 2)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "Enter a valid monthly amount."))?;
    if amount.starts_with('-') {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "Monthly budgets cannot be negative."));
    }
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM finance_categories")
        .fetch_all(&mut *tx).await.map_err(ApiError::internal)?;
    let duplicate = names.iter().any(|existing| existing.trim().to_lowercase() == name.to_lowercase());
    if duplicate {
        return Err(ApiError::new(StatusCode::CONFLICT, "A category with this name already exists."));
    }
    let month = crate::budget::today()[..7].to_owned();
    crate::budget::require_open(&mut tx, &month).await
        .map_err(|e| ApiError::new(StatusCode::CONFLICT, e.to_string()))?;
    // Establish the existing ledger before adding a zero-opening category.
    crate::budget_ledger::sync(&mut tx, None).await.map_err(ApiError::internal)?;
    let first = crate::budget::first_month(&mut tx).await.map_err(ApiError::internal)?.unwrap();
    let id = Uuid::now_v7().to_string();
    let operation = Uuid::now_v7().to_string();
    let row: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(source_row),0)+1 FROM finance_categories")
        .fetch_one(&mut *tx).await.map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO finance_categories VALUES (?,?,1,?,'{}')")
        .bind(&id).bind(name).bind(row).execute(&mut *tx).await.map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO budget_opening_state VALUES (?,?,'0',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),'{}')")
        .bind(&id).bind(&first).bind(&actor.subject).execute(&mut *tx).await.map_err(ApiError::internal)?;
    // A zero baseline supports every open month without backdating allocations.
    sqlx::query("INSERT INTO category_budget_periods VALUES (?,?,'0',?,'null')")
        .bind(&id).bind(&first).bind(row).execute(&mut *tx).await.map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO category_budget_periods VALUES (?,?,?,?,'null') ON CONFLICT(category_id,effective_start_month) DO UPDATE SET base_monthly_eur=excluded.base_monthly_eur")
        .bind(&id).bind(&month).bind(&amount).bind(row).execute(&mut *tx).await.map_err(ApiError::internal)?;
    let audit = json!({"categoryCreated":true,"categoryId":id,"name":name,"monthlyAmount":amount,"effectiveStartMonth":month});
    sqlx::query("INSERT INTO finance_operations (operation_id,actor_subject,payload_fingerprint,response_json,affected_transaction_ids_json,created_at) VALUES (?,?,?,?,'[]',strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
        .bind(&operation).bind(&actor.subject).bind(hex::encode(Sha256::digest(audit.to_string().as_bytes())))
        .bind(audit.to_string()).execute(&mut *tx).await.map_err(ApiError::internal)?;
    crate::budget_ledger::sync(&mut tx, Some(&operation)).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(audit))
}

async fn cancel_monthly_budget(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<MonthlyBudgetRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let actor = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Sign in to continue."))?;
    if !state.config.finance_db_transaction_writes_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Finance changes are unavailable.",
        ));
    }
    if !valid_month(&payload.effective_start_month)
        || payload.effective_start_month <= crate::budget::today()[..7].to_owned()
        || payload.categories.is_empty()
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Only future scheduled changes can be cancelled.",
        ));
    }
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::internal)?;
    let closed: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month>=?)")
            .bind(&payload.effective_start_month)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
    if closed {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "This plan affects a closed month.",
        ));
    }
    for category in &payload.categories {
        sqlx::query("DELETE FROM category_budget_periods WHERE effective_start_month=? AND category_id IN (SELECT id FROM finance_categories WHERE name=? AND is_real=1 AND id<>'budget-unallocated')")
            .bind(&payload.effective_start_month).bind(&category.name).execute(&mut *tx).await.map_err(ApiError::internal)?;
    }
    let audit = json!({"scheduledBudgetCancelled":true,"month":payload.effective_start_month,"categories":payload.categories.iter().map(|c| &c.name).collect::<Vec<_>>()});
    sqlx::query("INSERT INTO finance_operations (operation_id,actor_subject,payload_fingerprint,response_json,affected_transaction_ids_json,created_at) VALUES (?,?,?,?,'[]',strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
        .bind(Uuid::now_v7().to_string()).bind(actor.subject).bind(hex::encode(Sha256::digest(audit.to_string().as_bytes()))).bind(audit.to_string()).execute(&mut *tx).await.map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(json!({"cancelled":true})))
}

async fn save_monthly_budget(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<MonthlyBudgetRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let pocket_session = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_transaction_writes_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database finance writes are not enabled.",
        ));
    }
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    crate::budget::initialize(&mut tx).await.map_err(ApiError::internal)?;
    let month = crate::budget::today()[..7].to_owned();
    if !valid_month(&payload.effective_start_month) || payload.effective_start_month < month || payload.categories.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Budget changes must start in the current Madrid month or a future month.",
        ));
    }
    let closed: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month >= ?)")
            .bind(&payload.effective_start_month)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
    if closed {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "A current or future budget month is closed.",
        ));
    }
    let mut changes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for category in payload.categories {
        if !seen.insert(category.name.clone()) || category.amount.len() > 64 {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Enter each category once with a valid amount.",
            ));
        }
        let amount =
            crate::finance_model::round_posted_decimal_text(&category.amount, 2).map_err(|_| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Enter a valid monthly budget amount.",
                )
            })?;
        if amount.starts_with('-') {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Monthly budgets cannot be negative.",
            ));
        }
        let row = sqlx::query(
            "SELECT id, source_row FROM finance_categories WHERE name = ? AND is_real = 1 AND id <> 'budget-unallocated'",
        )
        .bind(&category.name)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "Choose a real budget category."))?;
        let id: String = row.try_get("id").map_err(ApiError::internal)?;
        let source_row: i64 = row.try_get("source_row").map_err(ApiError::internal)?;
        let previous = sqlx::query_scalar::<_, Option<String>>("SELECT base_monthly_eur FROM category_budget_periods WHERE category_id=? AND effective_start_month=?")
            .bind(&id).bind(&payload.effective_start_month).fetch_optional(&mut *tx).await.map_err(ApiError::internal)?.flatten();
        changes.push(json!({ "categoryId": id, "previousScheduledAmountEur": previous, "newAmountEur": amount }));
        sqlx::query("DELETE FROM category_budget_periods WHERE category_id = ? AND effective_start_month = ?")
            .bind(&id).bind(&payload.effective_start_month).execute(&mut *tx).await.map_err(ApiError::internal)?;
        sqlx::query("INSERT INTO category_budget_periods (category_id, effective_start_month, base_monthly_eur, source_row, source_cell_json) VALUES (?, ?, ?, ?, 'null')")
            .bind(&id).bind(&payload.effective_start_month).bind(&amount).bind(source_row).execute(&mut *tx).await.map_err(ApiError::internal)?;
    }
    let operation_id = Uuid::now_v7().to_string();
    let audit = json!({ "monthlyBudgetSaved": true, "effectiveStartMonth": payload.effective_start_month, "changes": changes });
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&audit).map_err(ApiError::internal)?,
    ));
    sqlx::query("INSERT INTO finance_operations (operation_id,actor_subject,payload_fingerprint,affected_transaction_ids_json,response_json,created_at) VALUES (?,?,?,'[]',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
        .bind(&operation_id).bind(&pocket_session.subject).bind(fingerprint).bind(audit.to_string())
        .execute(&mut *tx).await.map_err(ApiError::internal)?;
    crate::budget_ledger::sync(&mut tx, Some(&operation_id))
        .await
        .map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(json!({ "saved": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedIncomeRequest {
    effective_start_month: String,
    amount: String,
    currency: String,
    arrival_day: u8,
}

async fn save_expected_income(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ExpectedIncomeRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let session = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_transaction_writes_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database finance writes are not enabled.",
        ));
    }
    if payload.effective_start_month.len() != 7
        || !valid_date(&format!("{}-01", payload.effective_start_month))
        || !(1..=31).contains(&payload.arrival_day)
        || payload.amount.len() > 64
        || !matches!(payload.currency.as_str(), "EUR" | "USD" | "PHP")
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Enter a valid month, non-negative amount and arrival day from 1 to 31.",
        ));
    }
    let amount =
        crate::finance_model::round_posted_decimal_text(&payload.amount, 2).map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "Expected income must be a valid amount.",
            )
        })?;
    if amount.starts_with('-') {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Expected income cannot be negative.",
        ));
    }
    if payload.effective_start_month.as_str() < &crate::budget::today()[..7] {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Expected income settings must start in the current month or a future month.",
        ));
    }
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::internal)?;
    let closed: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month>=?)")
            .bind(&payload.effective_start_month)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
    if closed {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Expected income in closed months is frozen.",
        ));
    }
    let operation_id = Uuid::now_v7().to_string();
    let audit = json!({"expectedIncomeSaved":true,"effectiveStartMonth":payload.effective_start_month,"amount":amount,"currency":payload.currency,"arrivalDay":payload.arrival_day});
    let fingerprint = hex::encode(Sha256::digest(audit.to_string().as_bytes()));
    sqlx::query(
        "INSERT INTO finance_operations VALUES(?,?,?,'[]',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&operation_id)
    .bind(&session.subject)
    .bind(fingerprint)
    .bind(audit.to_string())
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO budget_income_expectations (effective_start_month, amount, currency, arrival_day) VALUES (?, ?, ?, ?) ON CONFLICT(effective_start_month) DO UPDATE SET amount = excluded.amount, currency = excluded.currency, arrival_day = excluded.arrival_day")
        .bind(&payload.effective_start_month).bind(&amount).bind(&payload.currency).bind(payload.arrival_day)
        .execute(&mut *tx).await.map_err(ApiError::internal)?;
    let instances = sqlx::query_scalar::<_,String>("SELECT month FROM budget_plan_income WHERE month>=? AND completed_month IS NULL AND NOT EXISTS(SELECT 1 FROM budget_month_closures WHERE month=budget_plan_income.month) ORDER BY month")
        .bind(&payload.effective_start_month).fetch_all(&mut *tx).await.map_err(ApiError::internal)?;
    let rates = crate::finance_read_model::load_rates(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    for month in instances {
        let setting=sqlx::query("SELECT amount,currency,arrival_day FROM budget_income_expectations WHERE effective_start_month<=? ORDER BY effective_start_month DESC LIMIT 1").bind(&month).fetch_one(&mut *tx).await.map_err(ApiError::internal)?;
        let amount: String = setting.try_get("amount").map_err(ApiError::internal)?;
        let currency: String = setting.try_get("currency").map_err(ApiError::internal)?;
        let day: i64 = setting.try_get("arrival_day").map_err(ApiError::internal)?;
        let eur = crate::finance_model::account_balance_eur(
            &amount,
            &currency,
            &format!("{month}-01"),
            &rates,
        )
        .map_err(ApiError::internal)?;
        sqlx::query("UPDATE budget_plan_income SET amount=?,currency=?,amount_eur=?,arrival_day=? WHERE month=?")
            .bind(amount).bind(currency).bind(eur).bind(day).bind(month).execute(&mut *tx).await.map_err(ApiError::internal)?;
    }
    crate::budget_ledger::sync(&mut tx, Some(&operation_id))
        .await
        .map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(json!({ "saved": true })))
}

const FINANCE_BACKUP_TABLES: &[&str] = &[
    "finance_accounts",
    "finance_account_opening_balances",
    "finance_categories",
    "category_budget_periods",
    "finance_transactions",
    "finance_transaction_revisions",
    "transaction_valuations",
    "transaction_valuation_source_cells",
    "budget_month_snapshots",
    "budget_opening_state",
    "finance_exchange_rate_source_rows",
    "exchange_rates",
    "finance_account_actuals",
    "finance_account_balance_checkpoints",
    "finance_operations",
    "finance_category_corrections",
    "finance_category_correction_items",
    "reconciliation_batches",
    "reconciliation_items",
    "finance_import_runs",
    "finance_quarantined_rows",
    "finance_source_artifacts",
    "budget_income_expectations",
    "budget_ledger_state",
    "budget_plan_income",
    "budget_income_receipts",
    "budget_ledger_movements",
    "budget_month_closures",
    "budget_coverage_closures",
];

async fn restore_db_finance(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_restore_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Mobile snapshot restore is disabled on this server.",
        ));
    }
    let expected_digest = headers
        .get("x-finance-snapshot-sha256")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "A valid finance snapshot digest is required.",
            )
        })?;
    let actual_digest = format!("{:x}", Sha256::digest(&body));
    if !actual_digest.eq_ignore_ascii_case(expected_digest) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Finance snapshot digest verification failed.",
        ));
    }
    let export: Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "Finance snapshot JSON is invalid."))?;
    if export
        .get("interchangeFormatVersion")
        .and_then(Value::as_u64)
        != Some(1)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Finance snapshot format is unsupported.",
        ));
    }
    let snapshot_tables = export
        .get("tables")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "Finance snapshot tables are missing.",
            )
        })?;
    let declared_counts = export
        .get("tableCounts")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "Finance snapshot counts are missing.",
            )
        })?;
    if snapshot_tables.len() != FINANCE_BACKUP_TABLES.len()
        || declared_counts.len() != FINANCE_BACKUP_TABLES.len()
        || FINANCE_BACKUP_TABLES.iter().any(|table| {
            !snapshot_tables.contains_key(*table) || !declared_counts.contains_key(*table)
        })
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Finance snapshot does not contain the complete table set.",
        ));
    }

    let mut transaction = state.pool.begin().await.map_err(ApiError::internal)?;
    let current_schema_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    if export.get("databaseSchemaVersion").and_then(Value::as_i64) != Some(current_schema_version) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Snapshot schema version differs from this server. Deploy the matching database version before recovery.",
        ));
    }

    let mut expected_counts = serde_json::Map::new();
    for table in FINANCE_BACKUP_TABLES {
        let rows = snapshot_tables
            .get(*table)
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Finance snapshot table {table} is invalid."),
                )
            })?;
        if declared_counts.get(*table).and_then(Value::as_u64) != Some(rows.len() as u64) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("Finance snapshot count for {table} does not match its rows."),
            ));
        }
        let sql = format!("SELECT COUNT(*) FROM \"{table}\"");
        let existing_count: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        if existing_count != 0 {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "Recovery is allowed only when all finance tables are empty.",
            ));
        }
        expected_counts.insert((*table).to_string(), json!(rows.len()));
    }

    sqlx::query("PRAGMA defer_foreign_keys = ON")
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    for table in FINANCE_BACKUP_TABLES {
        let rows = snapshot_tables
            .get(*table)
            .and_then(Value::as_array)
            .expect("validated finance snapshot table");
        let columns_sql = format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid");
        let column_rows = sqlx::query(AssertSqlSafe(columns_sql))
            .fetch_all(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        let columns = column_rows
            .iter()
            .map(|row| row.try_get::<String, _>("name"))
            .collect::<Result<Vec<_>, _>>()
            .map_err(ApiError::internal)?;
        let quoted_columns = columns
            .iter()
            .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(", ");
        let select_values = columns
            .iter()
            .map(|column| format!("json_extract(?1, '$.\"{column}\"')"))
            .collect::<Vec<_>>()
            .join(", ");
        let insert = format!("INSERT INTO \"{table}\" ({quoted_columns}) SELECT {select_values}");
        for row in rows {
            let object = row.as_object().ok_or_else(|| {
                ApiError::new(StatusCode::BAD_REQUEST, "Finance snapshot row is invalid.")
            })?;
            if object.len() != columns.len()
                || columns.iter().any(|column| !object.contains_key(column))
            {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    format!("Finance snapshot columns for {table} differ from this server."),
                ));
            }
            sqlx::query(AssertSqlSafe(insert.clone()))
                .bind(serde_json::to_string(row).map_err(ApiError::internal)?)
                .execute(&mut *transaction)
                .await
                .map_err(|_| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        format!("A row in finance snapshot table {table} could not be restored."),
                    )
                })?;
        }
    }

    for (table, expected) in &expected_counts {
        let sql = format!("SELECT COUNT(*) FROM \"{table}\"");
        let actual: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        if Some(actual as u64) != expected.as_u64() {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Restored finance row counts do not match the snapshot.",
            ));
        }
    }
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    let foreign_key_issues = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    if integrity != "ok" || !foreign_key_issues.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Finance snapshot failed database integrity checks.",
        ));
    }
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(Json(json!({
        "restored": true,
        "databaseSchemaVersion": current_schema_version,
        "tableCounts": expected_counts,
        "sha256": actual_digest,
    })))
}

async fn get_db_finance_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_read_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database finance reads are not enabled.",
        ));
    }

    let mut snapshot = state.pool.begin().await.map_err(ApiError::internal)?;
    crate::budget::initialize(&mut snapshot).await.map_err(ApiError::internal)?;
    let mut tables = serde_json::Map::new();
    let mut counts = serde_json::Map::new();
    for table in FINANCE_BACKUP_TABLES {
        // Table names come only from the fixed allowlist above.
        let sql = format!("SELECT * FROM \"{table}\"");
        let rows = sqlx::query(AssertSqlSafe(sql))
            .fetch_all(&mut *snapshot)
            .await
            .map_err(ApiError::internal)?;
        let encoded = rows
            .iter()
            .map(sqlite_row_json_for_backup)
            .collect::<Result<Vec<_>, _>>()?;
        counts.insert((*table).to_string(), json!(encoded.len()));
        tables.insert((*table).to_string(), Value::Array(encoded));
    }
    let database_schema_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(&mut *snapshot)
    .await
    .map_err(ApiError::internal)?;
    let operation_watermark: Option<String> =
        sqlx::query_scalar("SELECT MAX(created_at) FROM finance_operations")
            .fetch_one(&mut *snapshot)
            .await
            .map_err(ApiError::internal)?;
    let exported_at: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')")
        .fetch_one(&mut *snapshot)
        .await
        .map_err(ApiError::internal)?;
    snapshot.commit().await.map_err(ApiError::internal)?;

    let interchange = json!({
        "interchangeFormatVersion": 1,
        "databaseSchemaVersion": database_schema_version,
        "exportedAt": exported_at,
        "operationWatermark": operation_watermark,
        "tableCounts": counts,
        "tables": tables,
    });
    let bytes = serde_json::to_vec(&interchange).map_err(ApiError::internal)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        axum::http::HeaderName::from_static("x-finance-snapshot-sha256"),
        HeaderValue::from_str(&digest).map_err(ApiError::internal)?,
    );
    Ok(response)
}

fn sqlite_row_json_for_backup(row: &sqlx::sqlite::SqliteRow) -> Result<Value, ApiError> {
    let mut object = serde_json::Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(index).map_err(ApiError::internal)?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            match raw.type_info().name() {
                "INTEGER" => json!(row.try_get::<i64, _>(index).map_err(ApiError::internal)?),
                "REAL" => json!(row.try_get::<f64, _>(index).map_err(ApiError::internal)?),
                "BLOB" => {
                    let bytes: Vec<u8> = row.try_get(index).map_err(ApiError::internal)?;
                    json!(hex::encode(bytes))
                }
                _ => json!(
                    row.try_get::<String, _>(index)
                        .map_err(ApiError::internal)?
                ),
            }
        };
        object.insert(column.name().to_string(), value);
    }
    Ok(Value::Object(object))
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransactionInput {
    transaction_id: String,
    kind: String,
    date: String,
    description: String,
    amount: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_account: Option<String>,
}

async fn submit_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TransactionInput>,
) -> Result<Json<Value>, ApiError> {
    ensure_same_origin(&state, &headers)?;
    if !valid_date(&payload.date) || payload.description.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "A valid date and transaction description are required.",
        ));
    }
    if !payload.amount.is_finite() || payload.amount <= 0.0 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Transaction amount must be greater than zero.",
        ));
    }
    if !matches!(
        payload.kind.as_str(),
        "expense" | "income" | "transfer" | "reallocate"
    ) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Unsupported transaction kind.",
        ));
    }
    if !is_uuid_v7(&payload.transaction_id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "transactionId must be a UUIDv7.",
        ));
    }
    let parameters = serde_json::to_value(payload).map_err(ApiError::internal)?;
    let result = run_gateway(&state, &headers, "submitTransaction", parameters).await?;
    Ok(Json(result))
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReconciliationPayload {
    date: String,
    accounts: Vec<ReconciliationAccount>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReconciliationAccount {
    name: String,
    actual_balance: f64,
    transaction_id: String,
}

async fn reconcile_accounts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ReconciliationPayload>,
) -> Result<Json<Value>, ApiError> {
    ensure_same_origin(&state, &headers)?;
    if !valid_date(&payload.date) || payload.accounts.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "A valid closing date and at least one account are required.",
        ));
    }
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    for account in &payload.accounts {
        if account.name.trim().is_empty() || !account.actual_balance.is_finite() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Each account needs a name and a finite actual balance.",
            ));
        }
        if !is_uuid_v7(&account.transaction_id) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Each reconciliation transaction needs a UUIDv7.",
            ));
        }
        if !names.insert(account.name.trim().to_ascii_lowercase())
            || !ids.insert(account.transaction_id.to_ascii_lowercase())
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Each account and transaction ID may appear only once.",
            ));
        }
    }
    let parameters = serde_json::to_value(payload).map_err(ApiError::internal)?;
    let result = run_gateway(&state, &headers, "reconcileAccounts", parameters).await?;
    Ok(Json(result))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DbReconciliationRequest {
    operation_id: String,
    date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observed_date: Option<String>,
    accounts: Vec<DbReconciliationAccount>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DbReconciliationAccount {
    account_id: String,
    actual_balance: String,
    transaction_id: String,
    expected_revision: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReconciliationPreviewQuery {
    observed_date: String,
    closing_month: Option<String>,
}

struct ReconciliationActivity {
    through_observation: String,
    after_observation: String,
    transactions: Vec<Value>,
}

async fn reconciliation_activity(
    connection: &mut sqlx::SqliteConnection,
    account_id: &str,
    closing_date: &str,
    observed_date: &str,
    observed_at: Option<&str>,
) -> Result<ReconciliationActivity, ApiError> {
    let rows = sqlx::query("SELECT t.id, t.revision, t.effective_date, t.occurred_at, (t.occurred_at IS NOT NULL AND julianday(t.occurred_at) > julianday(?)) AS after_cutoff, t.description, t.from_account_id, t.to_account_id, vf.amount_native AS from_native, vt.amount_native AS to_native FROM finance_transactions t LEFT JOIN transaction_valuations vf ON vf.transaction_id = t.id AND vf.side = 'from' AND vf.revision = (SELECT MAX(v.revision) FROM transaction_valuations v WHERE v.transaction_id = t.id AND v.side = 'from') LEFT JOIN transaction_valuations vt ON vt.transaction_id = t.id AND vt.side = 'to' AND vt.revision = (SELECT MAX(v.revision) FROM transaction_valuations v WHERE v.transaction_id = t.id AND v.side = 'to') WHERE t.effective_date > ? AND (t.from_account_id = ? OR t.to_account_id = ?) ORDER BY t.effective_date, t.id")
        .bind(observed_at).bind(closing_date).bind(account_id).bind(account_id).fetch_all(connection).await.map_err(ApiError::internal)?;
    let mut activity = ReconciliationActivity {
        through_observation: "0".into(),
        after_observation: "0".into(),
        transactions: Vec::new(),
    };
    for row in rows {
        let date: String = row.try_get("effective_date").map_err(ApiError::internal)?;
        let from: Option<String> = row.try_get("from_account_id").map_err(ApiError::internal)?;
        let to: Option<String> = row.try_get("to_account_id").map_err(ApiError::internal)?;
        let mut net = "0".to_string();
        if from.as_deref() == Some(account_id) {
            let amount: Option<String> = row.try_get("from_native").map_err(ApiError::internal)?;
            net = crate::finance_model::subtract_decimal_text(
                &net,
                &amount.ok_or_else(|| {
                    ApiError::new(
                        StatusCode::CONFLICT,
                        "An account transaction is missing its native valuation.",
                    )
                })?,
            )
            .map_err(ApiError::internal)?;
        }
        if to.as_deref() == Some(account_id) {
            let amount: Option<String> = row.try_get("to_native").map_err(ApiError::internal)?;
            net = crate::finance_model::add_decimal_text(
                &net,
                &amount.ok_or_else(|| {
                    ApiError::new(
                        StatusCode::CONFLICT,
                        "An account transaction is missing its native valuation.",
                    )
                })?,
            )
            .map_err(ApiError::internal)?;
        }
        if date.as_str() <= observed_date
            && !row
                .try_get::<bool, _>("after_cutoff")
                .map_err(ApiError::internal)?
        {
            activity.through_observation =
                crate::finance_model::add_decimal_text(&activity.through_observation, &net)
                    .map_err(ApiError::internal)?;
            activity.transactions.push(json!({ "id": row.try_get::<String, _>("id").map_err(ApiError::internal)?, "revision": row.try_get::<i64, _>("revision").map_err(ApiError::internal)?, "date": date, "occurredAt": row.try_get::<Option<String>, _>("occurred_at").map_err(ApiError::internal)?, "description": row.try_get::<String, _>("description").map_err(ApiError::internal)?, "netNative": net }));
        } else {
            activity.after_observation =
                crate::finance_model::add_decimal_text(&activity.after_observation, &net)
                    .map_err(ApiError::internal)?;
        }
    }
    Ok(activity)
}

async fn get_reconciliation_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReconciliationPreviewQuery>,
) -> Result<Json<Value>, ApiError> {
    current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_read_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database finance reads are not enabled.",
        ));
    }
    if !valid_date(&query.observed_date) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "The observation date must be valid.",
        ));
    }
    let date_distance: f64 =
        sqlx::query_scalar("SELECT abs(julianday(?) - julianday(?))")
            .bind(&query.observed_date)
            .bind(crate::budget::today())
            .fetch_one(&state.pool)
            .await
            .map_err(ApiError::internal)?;
    if date_distance > 1.0 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Reconciliation uses today's balances.",
        ));
    }
    let closing_date = if let Some(month) = &query.closing_month {
        let day = last_day_of_month(month).ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "Invalid closing month."))?;
        if !valid_month(month) || month.as_str() >= &query.observed_date[..7] {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Reconcile a completed month.",
            ));
        }
        format!("{month}-{day:02}")
    } else { crate::budget::previous_month_end(&query.observed_date).map_err(ApiError::internal)? };
    let rates = crate::finance_read_model::load_rates(&state.pool)
        .await
        .map_err(ApiError::internal)?;
    let mut transaction = state.pool.begin().await.map_err(ApiError::internal)?;
    let observed_at: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    let rows = sqlx::query("SELECT id, source_name, account_type, currency, current_native_balance, ledger_revision FROM finance_accounts WHERE is_real = 1 AND is_active = 1 ORDER BY source_row").fetch_all(&mut *transaction).await.map_err(ApiError::internal)?;
    let mut accounts = Vec::new();
    let mut total_eur = "0".to_string();
    let mut closing_total_eur = "0".to_string();
    let mut conversions = Vec::<Value>::new();
    for row in rows {
        let id: String = row.try_get("id").map_err(ApiError::internal)?;
        let currency: String = row
            .try_get::<Option<String>, _>("currency")
            .map_err(ApiError::internal)?
            .unwrap_or_else(|| "EUR".into());
        let balance = row
            .try_get::<Option<String>, _>("current_native_balance")
            .map_err(ApiError::internal)?
            .ok_or_else(|| {
                ApiError::new(StatusCode::CONFLICT, "An account is missing its balance.")
            })?;
        let activity = reconciliation_activity(
            &mut transaction,
            &id,
            &closing_date,
            &query.observed_date,
            Some(&observed_at),
        )
        .await?;
        let prior_adjustments = sqlx::query_scalar::<_, String>("SELECT amount_decimal FROM finance_transactions WHERE kind = 'reconciliation' AND effective_date = ? AND from_account_id = ?")
            .bind(&closing_date).bind(&id).fetch_all(&mut *transaction).await.map_err(ApiError::internal)?;
        if prior_adjustments.len() > 1 {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "More than one closing adjustment exists for an account.",
            ));
        }
        let prior_adjustment = prior_adjustments.first().map(String::as_str).unwrap_or("0");
        let current =
            crate::finance_model::subtract_decimal_text(&balance, &activity.after_observation)
                .map_err(ApiError::internal)?;
        let closing =
            crate::finance_model::subtract_decimal_text(&current, &activity.through_observation)
                .map_err(ApiError::internal)?;
        let current_eur = crate::finance_model::account_balance_eur(
            &current,
            &currency,
            &query.observed_date,
            &rates,
        )
        .map_err(ApiError::internal)?;
        let closing_before_adjustment =
            crate::finance_model::add_decimal_text(&closing, prior_adjustment)
                .map_err(ApiError::internal)?;
        let closing_eur = crate::finance_model::account_balance_eur(
            &closing_before_adjustment,
            &currency,
            &closing_date,
            &rates,
        )
        .map_err(ApiError::internal)?;
        total_eur = crate::finance_model::add_decimal_text(&total_eur, &current_eur)
            .map_err(ApiError::internal)?;
        closing_total_eur =
            crate::finance_model::add_decimal_text(&closing_total_eur, &closing_eur)
                .map_err(ApiError::internal)?;
        if !conversions.iter().any(|rate| rate["currency"] == currency) {
            let latest =
                crate::finance_model::select_prior_rate(&currency, &query.observed_date, &rates)
                    .map_err(ApiError::internal)?;
            let closing_rate =
                crate::finance_model::select_prior_rate(&currency, &closing_date, &rates)
                    .map_err(ApiError::internal)?;
            conversions.push(json!({ "currency": currency, "unitsPerEur": latest.quote, "date": latest.date, "closingUnitsPerEur": closing_rate.quote, "closingRateDate": closing_rate.date }));
        }
        accounts.push(json!({ "id": id, "name": row.try_get::<String, _>("source_name").map_err(ApiError::internal)?, "type": row.try_get::<Option<String>, _>("account_type").map_err(ApiError::internal)?, "currency": currency, "balanceNative": current, "closingBalanceNative": closing, "activityNative": activity.through_observation, "existingAdjustmentNative": prior_adjustment, "transactions": activity.transactions, "ledgerRevision": row.try_get::<i64, _>("ledger_revision").map_err(ApiError::internal)? }));
    }
    transaction.commit().await.map_err(ApiError::internal)?;
    let reconciliation =
        crate::finance_read_model::load_reconciliation_batch(&state.pool, &closing_date)
            .await
            .map_err(ApiError::internal)?;
    Ok(Json(json!({
        "closingDate": closing_date,
        "observedAt": observed_at,
        "observedDate": query.observed_date,
        "accounts": accounts,
        "totalEur": total_eur,
        "closingTotalEur": closing_total_eur,
        "rates": conversions,
        "reconciliation": reconciliation,
    })))
}

async fn record_db_reconciliation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<DbReconciliationRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let pocket_session = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_reconciliation_writes_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database reconciliation writes are not enabled.",
        ));
    }
    if !is_uuid_v7(&payload.operation_id) || !valid_date(&payload.date) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "operationId must be UUIDv7 and date must be valid.",
        ));
    }
    if payload.accounts.is_empty() || payload.accounts.len() > 100 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Provide one to one hundred accounts to reconcile.",
        ));
    }
    payload.date = payload.date.trim().to_string();
    let mut account_ids = HashSet::new();
    let mut transaction_ids = HashSet::new();
    for account in &mut payload.accounts {
        account.account_id = account.account_id.trim().to_string();
        account.transaction_id = account.transaction_id.to_ascii_lowercase();
        if account.account_id.is_empty()
            || account.account_id.len() > 256
            || !account_ids.insert(account.account_id.clone())
            || !is_uuid_v7(&account.transaction_id)
            || !transaction_ids.insert(account.transaction_id.clone())
            || account.expected_revision < 0
            || account.actual_balance.len() > 64
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Each account needs a unique ID, UUIDv7 transaction ID, nonnegative expected revision, and valid actual balance.",
            ));
        }
        account.actual_balance =
            crate::finance_model::round_posted_decimal_text(&account.actual_balance, 2).map_err(
                |_| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "Actual balances must be valid decimal amounts.",
                    )
                },
            )?;
    }
    payload
        .accounts
        .sort_by(|left, right| left.account_id.cmp(&right.account_id));
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(ApiError::internal)?,
    ));
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    crate::budget::initialize(&mut transaction).await.map_err(ApiError::internal)?;
    let requested_ids = payload
        .accounts
        .iter()
        .map(|account| account.transaction_id.as_str())
        .collect::<Vec<_>>();
    let affected_ids = serde_json::to_string(&requested_ids).map_err(ApiError::internal)?;
    let inserted = sqlx::query(
        "INSERT INTO finance_operations (operation_id, actor_subject, payload_fingerprint, affected_transaction_ids_json, response_json, created_at) VALUES (?, ?, ?, ?, '{}', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) ON CONFLICT(operation_id) DO NOTHING",
    )
    .bind(&payload.operation_id)
    .bind(&pocket_session.subject)
    .bind(&fingerprint)
    .bind(affected_ids)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    if inserted.rows_affected() == 0 {
        let previous = sqlx::query(
            "SELECT o.actor_subject, o.payload_fingerprint, o.response_json, b.operation_id AS reconciliation_operation FROM finance_operations o LEFT JOIN reconciliation_batches b ON b.operation_id = o.operation_id WHERE o.operation_id = ?",
        )
        .bind(&payload.operation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal("finance operation conflict disappeared"))?;
        let actor: String = previous
            .try_get("actor_subject")
            .map_err(ApiError::internal)?;
        let old_fingerprint: String = previous
            .try_get("payload_fingerprint")
            .map_err(ApiError::internal)?;
        let reconciliation_operation: Option<String> = previous
            .try_get("reconciliation_operation")
            .map_err(ApiError::internal)?;
        if actor != pocket_session.subject
            || old_fingerprint != fingerprint
            || reconciliation_operation.as_deref() != Some(payload.operation_id.as_str())
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "operationId was already used for a different finance operation.",
            ));
        }
        let response_text: String = previous
            .try_get("response_json")
            .map_err(ApiError::internal)?;
        let response: Value = serde_json::from_str(&response_text).map_err(ApiError::internal)?;
        transaction.commit().await.map_err(ApiError::internal)?;
        return Ok(Json(response));
    }

    let closed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month=?)")
        .bind(&payload.date[..7]).fetch_one(&mut *transaction).await.map_err(ApiError::internal)?;
    if closed || payload.date[..7] > crate::budget::today()[..7] {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Reconciliation requires a period that is not closed or in the future.",
        ));
    }
    if payload.observed_at.is_some() != payload.observed_date.is_some() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Provide both the observation timestamp and local date.",
        ));
    }
    if let (Some(observed_at), Some(observed_date)) = (&payload.observed_at, &payload.observed_date)
    {
        if !valid_date(observed_date)
            || observed_at.len() > 40
            || !observed_at.is_ascii()
            || !observed_at.contains('T')
            || !observed_at.ends_with('Z')
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "The balance observation must have a valid date and UTC timestamp.",
            ));
        }
        let age: Option<f64> =
            sqlx::query_scalar("SELECT (julianday('now') - julianday(?)) * 86400")
                .bind(observed_at)
                .fetch_one(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        let distance: Option<f64> = sqlx::query_scalar("SELECT abs(julianday(?) - julianday(?))")
            .bind(observed_date)
            .bind(&observed_at[..10.min(observed_at.len())])
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        let closing: Option<String> =
            sqlx::query_scalar("SELECT date(?, 'start of month', '-1 day')")
                .bind(observed_date)
                .fetch_one(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        if age.is_none_or(|seconds| !(-60.0..=86400.0).contains(&seconds))
            || distance.is_none_or(|days| days > 1.0)
            || closing.as_deref().is_none_or(|latest| payload.date.as_str() > latest)
            || last_day_of_month(&payload.date[..7]).is_none_or(|day| payload.date != format!("{}-{day:02}", &payload.date[..7]))
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "Refresh reconciliation to check current balances for the previous month.",
            ));
        }
    }

    // Multiple submissions for the same date are allowed; each one is an
    // immutable batch, while each account keeps one replaceable ledger row.
    let recorded_at: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query("INSERT INTO reconciliation_batches (id, operation_id, effective_date, actor_subject, submitted_at, observed_at, observed_date) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&payload.operation_id)
        .bind(&payload.operation_id)
        .bind(&payload.date)
        .bind(&pocket_session.subject)
        .bind(&recorded_at)
        .bind(&payload.observed_at)
        .bind(&payload.observed_date)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;

    let reconciliation_account = load_system_db_account(&mut transaction, "Reconciliation").await?;
    let reconciliation_category =
        load_system_db_category(&mut transaction, "Reconciliation").await?;
    let mut observations = Vec::new();
    for row in sqlx::query(
        "SELECT quote_currency, rate_date, rate_decimal FROM exchange_rates WHERE provider = 'ECB' AND base_currency = 'EUR' AND quote_currency IN ('PHP', 'USD') AND rate_date IS NOT NULL AND rate_decimal IS NOT NULL",
    )
    .fetch_all(&mut *transaction)
    .await
    .map_err(ApiError::internal)?
    {
        observations.push(crate::finance_model::RateObservation {
            currency: row.try_get("quote_currency").map_err(ApiError::internal)?,
            date: row.try_get("rate_date").map_err(ApiError::internal)?,
            currency_units_per_eur: row.try_get("rate_decimal").map_err(ApiError::internal)?,
        });
    }
    let current_as_of = crate::budget::today();
    let mut results = Vec::with_capacity(payload.accounts.len());
    let mut actual_transaction_ids = Vec::with_capacity(payload.accounts.len());
    for requested in &payload.accounts {
        let account = load_db_account(&mut transaction, &requested.account_id).await?;
        if !account.is_real
            || !account.is_active
            || !matches!(account.currency.as_str(), "EUR" | "PHP" | "USD")
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Only active real accounts can be reconciled.",
            ));
        }
        let account_row = sqlx::query(
            "SELECT current_native_balance, ledger_revision FROM finance_accounts WHERE id = ?",
        )
        .bind(&account.id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
        let current_balance: Option<String> = account_row
            .try_get("current_native_balance")
            .map_err(ApiError::internal)?;
        let current_balance = current_balance.ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "A reconciled account has no canonical native balance.",
            )
        })?;
        let current_revision: i64 = account_row
            .try_get("ledger_revision")
            .map_err(ApiError::internal)?;
        if current_revision != requested.expected_revision {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "An account changed after its balance was loaded. Refresh the account balances and reconcile again.",
            ));
        }
        let existing_rows = sqlx::query(
            "SELECT t.id, t.amount_decimal, t.revision FROM finance_transactions t JOIN finance_accounts ta ON ta.id = t.to_account_id JOIN finance_categories fc ON fc.id = t.from_category_id WHERE t.kind = 'reconciliation' AND t.effective_date = ? AND t.from_account_id = ? AND ta.source_name = 'Reconciliation' COLLATE NOCASE AND fc.name = 'Reconciliation' COLLATE NOCASE",
        )
        .bind(&payload.date)
        .bind(&account.id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
        if existing_rows.len() > 1 {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "More than one reconciliation adjustment exists for an account and date.",
            ));
        }
        let existing = existing_rows.first();
        let transaction_id = if let Some(row) = existing {
            row.try_get::<String, _>("id").map_err(ApiError::internal)?
        } else {
            requested.transaction_id.clone()
        };
        if transaction_id != requested.transaction_id {
            let requested_id_exists: i64 = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM finance_transactions WHERE id = ?)",
            )
            .bind(&requested.transaction_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
            if requested_id_exists != 0 {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "transactionId is already used by another transaction.",
                ));
            }
        }
        let prior_adjustment = if let Some(row) = existing {
            row.try_get::<String, _>("amount_decimal")
                .map_err(ApiError::internal)?
        } else {
            "0".to_string()
        };
        let base_balance =
            crate::finance_model::add_decimal_text(&current_balance, &prior_adjustment)
                .map_err(ApiError::internal)?;
        let activity = if let Some(observed_date) = &payload.observed_date {
            reconciliation_activity(
                &mut transaction,
                &account.id,
                &payload.date,
                observed_date,
                payload.observed_at.as_deref(),
            )
            .await?
        } else {
            ReconciliationActivity {
                through_observation: "0".into(),
                after_observation: "0".into(),
                transactions: Vec::new(),
            }
        };
        let target_current = crate::finance_model::add_decimal_text(
            &requested.actual_balance,
            &activity.after_observation,
        )
        .map_err(ApiError::internal)?;
        let closing_actual = crate::finance_model::subtract_decimal_text(
            &requested.actual_balance,
            &activity.through_observation,
        )
        .map_err(ApiError::internal)?;
        let raw_adjustment =
            crate::finance_model::subtract_decimal_text(&base_balance, &target_current)
                .map_err(ApiError::internal)?;
        let adjustment = crate::finance_model::round_posted_decimal_text(&raw_adjustment, 2)
            .map_err(ApiError::internal)?;
        let actual_balance =
            crate::finance_model::round_posted_decimal_text(&requested.actual_balance, 2)
                .map_err(ApiError::internal)?;
        let next_revision = existing
            .map(|row| row.try_get::<i64, _>("revision"))
            .transpose()
            .map_err(ApiError::internal)?
            .unwrap_or(0)
            + 1;
        let valuation = crate::finance_model::calculate_valuation(
            &adjustment,
            &account.currency,
            &account.currency,
            "0",
            &reconciliation_account.currency,
            &payload.date,
            &observations,
            crate::finance_model::POSTING_ROUNDING_MODE,
        )
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
        let row_json = json!({
            "operationId": payload.operation_id,
            "transactionId": transaction_id,
            "date": payload.date,
            "kind": "reconciliation",
            "accountId": account.id,
            "amount": adjustment,
            "actualBalance": actual_balance,
            "observedAt": payload.observed_at,
            "observedDate": payload.observed_date,
            "closingBalance": closing_actual,
            "postMonthActivity": activity.through_observation,
            "includedTransactions": activity.transactions,
        });
        let row_json_text = serde_json::to_string(&row_json).map_err(ApiError::internal)?;
        if let Some(_row) = existing {
            sqlx::query("UPDATE finance_transactions SET amount_decimal = ?, input_currency = ?, revision = ? WHERE id = ?")
                .bind(&adjustment)
                .bind(&account.currency)
                .bind(next_revision)
                .bind(&transaction_id)
                .execute(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        } else {
            let source_row: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(source_row), 0) + 1 FROM finance_transactions WHERE source_sheet = 'Transactions'")
                .fetch_one(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
            sqlx::query("INSERT INTO finance_transactions (id, kind, effective_date, description, amount_decimal, input_currency, to_amount_decimal, to_currency, from_category_id, to_category_id, from_account_id, to_account_id, currency_is_override, to_amount_is_override, created_at, created_by, revision, source_workbook_id, source_sheet, source_row, original_row_fingerprint, source_row_json) VALUES (?, 'reconciliation', ?, 'reconciliation', ?, ?, NULL, NULL, ?, ?, ?, ?, 0, 0, ?, ?, 1, 'canonical-live', 'Transactions', ?, ?, ?)")
                .bind(&transaction_id)
                .bind(&payload.date)
                .bind(&adjustment)
                .bind(&account.currency)
                .bind(&reconciliation_category.id)
                .bind(&reconciliation_category.id)
                .bind(&account.id)
                .bind(&reconciliation_account.id)
                .bind(&recorded_at)
                .bind(&pocket_session.subject)
                .bind(source_row)
                .bind(&fingerprint)
                .bind(&row_json_text)
                .execute(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        }
        let revision_snapshot = json!({
            "transaction": row_json,
            "operationId": payload.operation_id,
            "expectedAccountRevision": requested.expected_revision,
            "accountLedgerRevision": current_revision + 1,
            "recordedBy": pocket_session.subject,
            "actualBalance": actual_balance,
            "balanceBeforeReplacement": base_balance,
            "replacedAdjustment": prior_adjustment,
            "newAdjustment": adjustment,
            "valuationPolicy": "ecb-prior-rate-fx-uplift-1.02-half-even-v1",
            "fromValuation": valuation_side_json(&valuation.from),
            "toValuation": valuation_side_json(&valuation.to),
        });
        sqlx::query("INSERT INTO finance_transaction_revisions (transaction_id, revision, operation_id, snapshot_json, recorded_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&transaction_id)
            .bind(next_revision)
            .bind(&payload.operation_id)
            .bind(serde_json::to_string(&revision_snapshot).map_err(ApiError::internal)?)
            .bind(&recorded_at)
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        for (side, side_valuation, currency) in [
            ("from", &valuation.from, account.currency.as_str()),
            (
                "to",
                &valuation.to,
                reconciliation_account.currency.as_str(),
            ),
        ] {
            let audit_json = serde_json::to_string(&valuation_side_json(side_valuation))
                .map_err(ApiError::internal)?;
            let exchange_to_eur = format!(
                "{}/{}",
                side_valuation.exchange_to_eur_numerator,
                side_valuation.exchange_to_eur_denominator
            );
            let exchange_to_native = format!(
                "{}/{}",
                side_valuation.exchange_to_native_numerator,
                side_valuation.exchange_to_native_denominator
            );
            sqlx::query("INSERT INTO transaction_valuations (transaction_id, side, account_currency, exchange_to_eur, exchange_to_native, amount_native, amount_eur, effective_date, valuation_source, rate_set_version, revision, source_cells_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'canonical-db-policy', 'ecb-prior-rate-fx-uplift-1.02-half-even-v1', ?, ?)")
                .bind(&transaction_id)
                .bind(side)
                .bind(currency)
                .bind(exchange_to_eur)
                .bind(exchange_to_native)
                .bind(&side_valuation.amount_native)
                .bind(&side_valuation.amount_eur)
                .bind(&payload.date)
                .bind(next_revision)
                .bind(audit_json)
                .execute(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        }
        let current_eur = crate::finance_model::account_balance_eur(
            &target_current,
            &account.currency,
            &current_as_of,
            &observations,
        )
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
        let actual_eur = crate::finance_model::account_balance_eur(
            &actual_balance,
            &account.currency,
            &current_as_of,
            &observations,
        )
        .map_err(ApiError::internal)?;
        sqlx::query("UPDATE finance_accounts SET current_native_balance = ?, current_eur_balance = ?, actual_native_balance = ?, actual_eur_balance = ?, reconciliation_native = ?, ledger_revision = ledger_revision + 1 WHERE id = ?")
            .bind(&target_current)
            .bind(&current_eur)
            .bind(&actual_balance)
            .bind(&actual_eur)
            .bind(&adjustment)
            .bind(&account.id)
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        let account_revision: i64 =
            sqlx::query_scalar("SELECT ledger_revision FROM finance_accounts WHERE id = ?")
                .bind(&account.id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        sqlx::query("INSERT INTO reconciliation_items (batch_id, account_id, transaction_id, actual_native_balance, variance_native, account_revision, observed_native_balance, post_month_activity_native, included_transactions_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&payload.operation_id)
            .bind(&account.id)
            .bind(&transaction_id)
            .bind(&closing_actual)
            .bind(&adjustment)
            .bind(account_revision)
            .bind(if payload.observed_at.is_some() { Some(&actual_balance) } else { None })
            .bind(&activity.through_observation)
            .bind(serde_json::to_string(&activity.transactions).map_err(ApiError::internal)?)
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        actual_transaction_ids.push(transaction_id.clone());
        results.push(json!({
            "accountId": account.id,
            "transactionId": transaction_id,
            "actualBalance": actual_balance,
            "adjustment": adjustment,
            "balanceNative": actual_balance,
            "balanceEur": current_eur,
            "ledgerRevision": account_revision,
        }));
    }
    sqlx::query(
        "UPDATE finance_operations SET affected_transaction_ids_json = ? WHERE operation_id = ?",
    )
    .bind(serde_json::to_string(&actual_transaction_ids).map_err(ApiError::internal)?)
    .bind(&payload.operation_id)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    let response = json!({
        "recorded": true,
        "operationId": payload.operation_id,
        "date": payload.date,
        "submittedAt": recorded_at,
        "accounts": results,
    });
    sqlx::query("UPDATE finance_operations SET response_json = ? WHERE operation_id = ?")
        .bind(serde_json::to_string(&response).map_err(ApiError::internal)?)
        .bind(&payload.operation_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(Json(response))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloseBudgetMonthRequest {
    operation_id: String,
    month: String,
}



#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CategoryCorrectionRequest {
    operation_id: String,
    original_transaction_id: String,
    reason: String,
    category_deltas: Vec<CategoryExpenseDelta>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CategoryExpenseDelta {
    category_id: String,
    expense_delta_eur: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DbTransactionWriteRequest {
    operation_id: String,
    transaction_id: String,
    kind: String,
    date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    occurred_at: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    date_only: bool,
    description: String,
    amount: String,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    to_amount: Option<String>,
    #[serde(default)]
    from_category_id: Option<String>,
    #[serde(default)]
    to_category_id: Option<String>,
    #[serde(default)]
    from_account_id: Option<String>,
    #[serde(default)]
    to_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    income_treatment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    corrects_transaction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_income_month: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DbTransactionEditRequest {
    #[serde(flatten)]
    transaction: DbTransactionWriteRequest,
    expected_revision: i64,
    expected_account_revisions: Vec<DbExpectedAccountRevision>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DbTransactionDeleteRequest {
    operation_id: String,
    transaction_id: String,
    expected_revision: i64,
    expected_account_revisions: Vec<DbExpectedAccountRevision>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DbExpectedAccountRevision {
    account_id: String,
    revision: i64,
}

#[derive(Clone)]
struct DbWriteAccount {
    id: String,
    currency: String,
    is_real: bool,
    is_active: bool,
}

#[derive(Clone)]
struct DbWriteCategory {
    id: String,
    is_real: bool,
}

async fn create_db_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<DbTransactionWriteRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let pocket_session = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_transaction_writes_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database transaction writes are not enabled.",
        ));
    }
    normalize_db_transaction_write(&mut payload)?;
    if !is_uuid_v7(&payload.operation_id) || !is_uuid_v7(&payload.transaction_id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "operationId and transactionId must be UUIDv7 values.",
        ));
    }

    let normalized_fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(ApiError::internal)?,
    ));
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    crate::budget::initialize(&mut transaction).await.map_err(ApiError::internal)?;
    let affected_ids =
        serde_json::to_string(&[&payload.transaction_id]).map_err(ApiError::internal)?;
    let inserted = sqlx::query(
        "INSERT INTO finance_operations (operation_id, actor_subject, payload_fingerprint, affected_transaction_ids_json, response_json, created_at) VALUES (?, ?, ?, ?, '{}', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) ON CONFLICT(operation_id) DO NOTHING",
    )
    .bind(&payload.operation_id)
    .bind(&pocket_session.subject)
    .bind(&normalized_fingerprint)
    .bind(affected_ids)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    if inserted.rows_affected() == 0 {
        let previous = sqlx::query(
            "SELECT o.actor_subject, o.payload_fingerprint, o.response_json, EXISTS (SELECT 1 FROM finance_transaction_revisions r WHERE r.operation_id = o.operation_id) AS transaction_operation FROM finance_operations o WHERE o.operation_id = ?",
        )
        .bind(&payload.operation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal("finance operation conflict disappeared"))?;
        let actor: String = previous
            .try_get("actor_subject")
            .map_err(ApiError::internal)?;
        let old_fingerprint: String = previous
            .try_get("payload_fingerprint")
            .map_err(ApiError::internal)?;
        let is_transaction_operation: i64 = previous
            .try_get("transaction_operation")
            .map_err(ApiError::internal)?;
        if actor != pocket_session.subject
            || old_fingerprint != normalized_fingerprint
            || is_transaction_operation == 0
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "operationId was already used for a different finance operation.",
            ));
        }
        let response_text: String = previous
            .try_get("response_json")
            .map_err(ApiError::internal)?;
        let response: Value = serde_json::from_str(&response_text).map_err(ApiError::internal)?;
        transaction.commit().await.map_err(ApiError::internal)?;
        return Ok(Json(response));
    }

    let from_account_id = payload.from_account_id.as_deref();
    let to_account_id = payload.to_account_id.as_deref();
    let from_category_id = payload.from_category_id.as_deref();
    let to_category_id = payload.to_category_id.as_deref();
    let mut account_ids = Vec::<String>::new();
    let mut category_ids = Vec::<String>::new();
    match payload.kind.as_str() {
        "expense" => {
            if to_account_id.is_some() || to_category_id.is_some() || payload.to_amount.is_some() {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Expense transactions accept only a from account and category.",
                ));
            }
            account_ids.push(required_id(from_account_id, "fromAccountId")?);
            category_ids.push(required_id(from_category_id, "fromCategoryId")?);
        }
        "income" => {
            if from_account_id.is_some()
                || from_category_id.is_some()
                || payload.to_amount.is_some()
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Income transactions accept only a to account and category.",
                ));
            }
            account_ids.push(required_id(to_account_id, "toAccountId")?);
            category_ids.push(required_id(to_category_id, "toCategoryId")?);
        }
        "transfer" => {
            if from_category_id.is_some() || to_category_id.is_some() {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Transfer categories are assigned automatically.",
                ));
            }
            account_ids.push(required_id(from_account_id, "fromAccountId")?);
            account_ids.push(required_id(to_account_id, "toAccountId")?);
        }
        "reallocate" => {
            if from_account_id.is_some()
                || to_account_id.is_some()
                || payload.currency.is_some()
                || payload.to_amount.is_some()
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Reallocations accept only two categories and a EUR amount.",
                ));
            }
            category_ids.push(required_id(from_category_id, "fromCategoryId")?);
            category_ids.push(required_id(to_category_id, "toCategoryId")?);
        }
        _ => unreachable!(),
    }
    if payload.kind == "transfer" && account_ids[0] == account_ids[1] {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Transfer accounts must differ.",
        ));
    }
    if payload.kind == "reallocate" && category_ids[0] == category_ids[1] {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Reallocation categories must differ.",
        ));
    }

    let mut accounts = HashMap::<String, DbWriteAccount>::new();
    for account_id in &account_ids {
        let account = load_db_account(&mut transaction, account_id).await?;
        if !account.is_real
            || !account.is_active
            || !matches!(account.currency.as_str(), "EUR" | "PHP" | "USD")
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Transactions must use active real accounts with a supported currency.",
            ));
        }
        accounts.insert(account_id.clone(), account);
    }
    let mut categories = HashMap::<String, DbWriteCategory>::new();
    for category_id in &category_ids {
        let category = load_db_category(&mut transaction, category_id).await?;
        if !category.is_real {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Transactions must use real budget categories.",
            ));
        }
        categories.insert(category_id.clone(), category);
    }
    let from_account = from_account_id.and_then(|id| accounts.get(id)).cloned();
    let to_account = to_account_id.and_then(|id| accounts.get(id)).cloned();
    let from_category = from_category_id.and_then(|id| categories.get(id)).cloned();
    let to_category = to_category_id.and_then(|id| categories.get(id)).cloned();
    let (from_account, to_account, from_category, to_category) = match payload.kind.as_str() {
        "expense" => (
            from_account,
            Some(load_system_db_account(&mut transaction, "Expense").await?),
            from_category,
            Some(load_system_db_category(&mut transaction, "Expense").await?),
        ),
        "income" => (
            Some(load_system_db_account(&mut transaction, "Income").await?),
            to_account,
            Some(load_system_db_category(&mut transaction, "Income").await?),
            to_category,
        ),
        "transfer" => (
            from_account,
            to_account,
            Some(load_system_db_category(&mut transaction, "Balance Transfer").await?),
            Some(load_system_db_category(&mut transaction, "Balance Transfer").await?),
        ),
        "reallocate" => (
            Some(load_system_db_account(&mut transaction, "Reallocation").await?),
            Some(load_system_db_account(&mut transaction, "Reallocation").await?),
            from_category,
            to_category,
        ),
        _ => unreachable!(),
    };
    let from_account =
        from_account.ok_or_else(|| ApiError::internal("normalized from account is missing"))?;
    let to_account =
        to_account.ok_or_else(|| ApiError::internal("normalized to account is missing"))?;
    let from_category =
        from_category.ok_or_else(|| ApiError::internal("normalized from category is missing"))?;
    let to_category =
        to_category.ok_or_else(|| ApiError::internal("normalized to category is missing"))?;

    let default_input_currency = match payload.kind.as_str() {
        "income" => to_account.currency.as_str(),
        "reallocate" => "EUR",
        _ => from_account.currency.as_str(),
    };
    let input_currency = payload
        .currency
        .as_deref()
        .unwrap_or(default_input_currency)
        .to_string();
    if payload.kind == "reallocate" && input_currency != "EUR" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Reallocation amounts use EUR.",
        ));
    }
    let cross_currency_transfer =
        payload.kind == "transfer" && from_account.currency != to_account.currency;
    if payload.kind == "transfer" && cross_currency_transfer != payload.to_amount.is_some() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            if cross_currency_transfer {
                "Cross-currency transfers require toAmount."
            } else {
                "Same-currency transfers must not include toAmount."
            },
        ));
    }
    let valuation_to_amount = payload.to_amount.as_deref().unwrap_or(&payload.amount);
    let mut observations = Vec::new();
    for row in sqlx::query(
        "SELECT quote_currency, rate_date, rate_decimal FROM exchange_rates WHERE provider = 'ECB' AND base_currency = 'EUR' AND quote_currency IN ('PHP', 'USD') AND rate_date IS NOT NULL AND rate_decimal IS NOT NULL",
    )
    .fetch_all(&mut *transaction)
    .await
    .map_err(ApiError::internal)?
    {
        observations.push(crate::finance_model::RateObservation {
            currency: row.try_get("quote_currency").map_err(ApiError::internal)?,
            date: row.try_get("rate_date").map_err(ApiError::internal)?,
            currency_units_per_eur: row.try_get("rate_decimal").map_err(ApiError::internal)?,
        });
    }
    let valuation = crate::finance_model::calculate_valuation(
        &payload.amount,
        &input_currency,
        &from_account.currency,
        valuation_to_amount,
        &to_account.currency,
        &payload.date,
        &observations,
        crate::finance_model::POSTING_ROUNDING_MODE,
    )
    .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
    let projection =
        crate::finance_model::project_transaction(&crate::finance_model::TransactionEffect {
            from_account_id: Some(from_account.id.clone()),
            to_account_id: Some(to_account.id.clone()),
            from_category_id: Some(from_category.id.clone()),
            to_category_id: Some(to_category.id.clone()),
            kind: payload.kind.clone(),
            amount: payload.amount.clone(),
            valuation: valuation.clone(),
        })
        .map_err(ApiError::internal)?;

    let effective_month = payload
        .date
        .get(..7)
        .ok_or_else(|| ApiError::internal("stored transaction date is invalid"))?;
    crate::budget::require_open(&mut transaction, effective_month).await
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
    let existing_transaction: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM finance_transactions WHERE id = ?)")
        .bind(&payload.transaction_id).fetch_one(&mut *transaction).await.map_err(ApiError::internal)?;
    if existing_transaction {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "transactionId already exists.",
        ));
    }
    let source_row: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(source_row), 0) + 1 FROM finance_transactions WHERE source_sheet = 'Transactions'",
    )
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    let input_override = input_currency != default_input_currency;
    let to_amount_override = payload.kind == "transfer" && payload.to_amount.is_some();
    let stored_to_amount = if to_amount_override {
        payload.to_amount.clone()
    } else {
        None
    };
    let stored_to_currency = if payload.kind == "transfer" {
        Some(to_account.currency.clone())
    } else {
        None
    };
    let occurred_at = resolve_transaction_timestamp(
        &mut transaction,
        &payload.date,
        payload.occurred_at.as_deref(),
        payload.date_only,
    )
    .await?;
    validate_db_correction(&mut transaction, &payload).await?;
    let source_request = json!({
        "operationId": payload.operation_id,
        "transactionId": payload.transaction_id,
        "kind": payload.kind,
        "incomeTreatment": payload.income_treatment,
        "correctsTransactionId": payload.corrects_transaction_id,
        "expectedIncomeMonth": payload.expected_income_month,
        "date": payload.date,
        "occurredAt": occurred_at,
        "description": payload.description,
        "amount": payload.amount,
        "inputCurrency": input_currency,
        "toAmount": stored_to_amount,
        "toCurrency": stored_to_currency,
        "fromAccountId": from_account.id,
        "toAccountId": to_account.id,
        "fromCategoryId": from_category.id,
        "toCategoryId": to_category.id,
    });
    let source_json = serde_json::to_string(&source_request).map_err(ApiError::internal)?;
    let created_at: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query(
        "INSERT INTO finance_transactions (id, kind, effective_date, description, amount_decimal, input_currency, to_amount_decimal, to_currency, from_category_id, to_category_id, from_account_id, to_account_id, currency_is_override, to_amount_is_override, created_at, created_by, source_workbook_id, source_sheet, source_row, original_row_fingerprint, source_row_json, occurred_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'canonical-live', 'Transactions', ?, ?, ?, ?)",
    )
    .bind(&payload.transaction_id)
    .bind(&payload.kind)
    .bind(&payload.date)
    .bind(&payload.description)
    .bind(&payload.amount)
    .bind(&input_currency)
    .bind(&stored_to_amount)
    .bind(&stored_to_currency)
    .bind(&from_category.id)
    .bind(&to_category.id)
    .bind(&from_account.id)
    .bind(&to_account.id)
    .bind(input_override)
    .bind(to_amount_override)
    .bind(&created_at)
    .bind(&pocket_session.subject)
    .bind(source_row)
    .bind(&normalized_fingerprint)
    .bind(&source_json)
    .bind(&occurred_at)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    let revision_snapshot = json!({
        "transaction": source_request,
        "recordedByName": pocket_session.username,
        "operationId": payload.operation_id,
        "valuationPolicy": "ecb-prior-rate-fx-uplift-1.02-half-even-v1",
        "fromValuation": valuation_side_json(&valuation.from),
        "toValuation": valuation_side_json(&valuation.to),
    });
    sqlx::query("INSERT INTO finance_transaction_revisions (transaction_id, revision, operation_id, snapshot_json, recorded_at) VALUES (?, 1, ?, ?, ?)")
        .bind(&payload.transaction_id)
        .bind(&payload.operation_id)
        .bind(serde_json::to_string(&revision_snapshot).map_err(ApiError::internal)?)
        .bind(&created_at)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    for (side, side_valuation, currency) in [
        ("from", &valuation.from, from_account.currency.as_str()),
        ("to", &valuation.to, to_account.currency.as_str()),
    ] {
        let audit_json = serde_json::to_string(&valuation_side_json(side_valuation))
            .map_err(ApiError::internal)?;
        let exchange_to_eur = format!(
            "{}/{}",
            side_valuation.exchange_to_eur_numerator, side_valuation.exchange_to_eur_denominator
        );
        let exchange_to_native = format!(
            "{}/{}",
            side_valuation.exchange_to_native_numerator,
            side_valuation.exchange_to_native_denominator
        );
        sqlx::query("INSERT INTO transaction_valuations (transaction_id, side, account_currency, exchange_to_eur, exchange_to_native, amount_native, amount_eur, effective_date, valuation_source, rate_set_version, revision, source_cells_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'canonical-db-policy', 'ecb-prior-rate-fx-uplift-1.02-half-even-v1', 1, ?)")
            .bind(&payload.transaction_id)
            .bind(side)
            .bind(currency)
            .bind(exchange_to_eur)
            .bind(exchange_to_native)
            .bind(&side_valuation.amount_native)
            .bind(&side_valuation.amount_eur)
            .bind(&payload.date)
            .bind(audit_json)
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
    }

    let current_as_of = crate::budget::today();
    let mut updated_accounts = Vec::new();
    for (account_id, delta) in &projection.account_deltas {
        let account = load_db_account(&mut transaction, account_id).await?;
        if !account.is_real || !account.is_active {
            continue;
        }
        let row = sqlx::query("SELECT current_native_balance FROM finance_accounts WHERE id = ?")
            .bind(account_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        let balance: Option<String> = row
            .try_get("current_native_balance")
            .map_err(ApiError::internal)?;
        let balance = balance.ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "A touched account has no canonical native balance.",
            )
        })?;
        let new_balance =
            crate::finance_model::add_decimal_text(&balance, delta).map_err(ApiError::internal)?;
        let new_eur = crate::finance_model::account_balance_eur(
            &new_balance,
            &account.currency,
            &current_as_of,
            &observations,
        )
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
        sqlx::query("UPDATE finance_accounts SET current_native_balance = ?, current_eur_balance = ?, ledger_revision = ledger_revision + 1 WHERE id = ?")
            .bind(&new_balance)
            .bind(&new_eur)
            .bind(account_id)
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        let ledger_revision: i64 =
            sqlx::query_scalar("SELECT ledger_revision FROM finance_accounts WHERE id = ?")
                .bind(account_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        updated_accounts.push(
            json!({ "accountId": account_id, "balanceNative": new_balance, "balanceEur": new_eur, "ledgerRevision": ledger_revision }),
        );
    }

    crate::budget_ledger::receipt(
        &mut transaction,
        &payload.transaction_id,
        payload.income_treatment.as_deref(),
        payload.expected_income_month.as_deref(),
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::budget_ledger::sync(&mut transaction, Some(&payload.operation_id))
        .await
        .map_err(ApiError::internal)?;
    let updated_categories = crate::budget::categories(&mut transaction, effective_month)
        .await
        .map_err(ApiError::internal)?;
    let response = json!({
        "recorded": true,
        "operationId": payload.operation_id,
        "transactionId": payload.transaction_id,
        "kind": payload.kind,
        "month": effective_month,
        "valuationStale": valuation.from.stale || valuation.to.stale,
        "accounts": updated_accounts,
        "categories": updated_categories,
    });
    sqlx::query("UPDATE finance_operations SET response_json = ? WHERE operation_id = ?")
        .bind(serde_json::to_string(&response).map_err(ApiError::internal)?)
        .bind(&payload.operation_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    crate::push::enqueue(&mut transaction, &payload.operation_id, &payload.transaction_id,
        &pocket_session.subject, &state.config.pocket_id_allowed_subjects,
        &format!("{} added {}", pocket_session.username, match payload.kind.as_str() { "expense" => "an expense", "income" => "income", "transfer" => "a transfer", "reallocate" => "a budget move", _ => "a transaction" }),
        &format!("{} {} · {}", payload.amount, input_currency, payload.description.lines().next().unwrap_or("").chars().take(160).collect::<String>()),
        now_seconds())
        .await.map_err(ApiError::internal)?;
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(Json(response))
}

async fn edit_db_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<DbTransactionEditRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let pocket_session = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_transaction_writes_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database transaction writes are not enabled.",
        ));
    }
    normalize_db_transaction_write(&mut payload.transaction)?;
    if !is_uuid_v7(&payload.transaction.operation_id)
        || !is_uuid_v7(&payload.transaction.transaction_id)
        || payload.expected_revision < 1
        || payload.expected_account_revisions.len() > 100
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "operationId and transactionId must be UUIDv7 values and valid revisions are required.",
        ));
    }
    let mut expected_revisions = BTreeMap::new();
    for expected in &mut payload.expected_account_revisions {
        expected.account_id = expected.account_id.trim().to_string();
        if expected.account_id.is_empty()
            || expected.account_id.len() > 256
            || expected.revision < 0
            || expected_revisions
                .insert(expected.account_id.clone(), expected.revision)
                .is_some()
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Expected account revisions must have unique account IDs and nonnegative values.",
            ));
        }
    }
    payload
        .expected_account_revisions
        .sort_by(|left, right| left.account_id.cmp(&right.account_id));
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(ApiError::internal)?,
    ));
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    crate::budget::initialize(&mut transaction).await.map_err(ApiError::internal)?;
    let inserted = sqlx::query("INSERT INTO finance_operations (operation_id, actor_subject, payload_fingerprint, affected_transaction_ids_json, response_json, created_at) VALUES (?, ?, ?, ?, '{}', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) ON CONFLICT(operation_id) DO NOTHING")
        .bind(&payload.transaction.operation_id)
        .bind(&pocket_session.subject)
        .bind(&fingerprint)
        .bind(serde_json::to_string(&[&payload.transaction.transaction_id]).map_err(ApiError::internal)?)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    if inserted.rows_affected() == 0 {
        let previous = sqlx::query("SELECT o.actor_subject, o.payload_fingerprint, o.response_json, r.transaction_id FROM finance_operations o LEFT JOIN finance_transaction_revisions r ON r.operation_id = o.operation_id WHERE o.operation_id = ?")
            .bind(&payload.transaction.operation_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::internal("finance operation conflict disappeared"))?;
        let actor: String = previous
            .try_get("actor_subject")
            .map_err(ApiError::internal)?;
        let previous_fingerprint: String = previous
            .try_get("payload_fingerprint")
            .map_err(ApiError::internal)?;
        let previous_transaction: Option<String> = previous
            .try_get("transaction_id")
            .map_err(ApiError::internal)?;
        if actor != pocket_session.subject
            || previous_fingerprint != fingerprint
            || previous_transaction.as_deref() != Some(payload.transaction.transaction_id.as_str())
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "operationId was already used for a different finance operation.",
            ));
        }
        let response_text: String = previous
            .try_get("response_json")
            .map_err(ApiError::internal)?;
        let response: Value = serde_json::from_str(&response_text).map_err(ApiError::internal)?;
        transaction.commit().await.map_err(ApiError::internal)?;
        return Ok(Json(response));
    }

    let old = sqlx::query("SELECT id, kind, effective_date, occurred_at, description, amount_decimal, input_currency, to_amount_decimal, to_currency, from_category_id, to_category_id, from_account_id, to_account_id, currency_is_override, to_amount_is_override, revision, source_workbook_id FROM finance_transactions WHERE id = ?")
        .bind(&payload.transaction.transaction_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "The transaction to edit was not found."))?;
    let source_workbook_id: String = old
        .try_get("source_workbook_id")
        .map_err(ApiError::internal)?;
    if source_workbook_id != "canonical-live" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Imported source transactions are not editable through this route.",
        ));
    }
    let old_revision: i64 = old.try_get("revision").map_err(ApiError::internal)?;
    if old_revision != payload.expected_revision {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "The transaction changed after it was loaded. Refresh and retry the edit.",
        ));
    }
    let old_kind: String = old.try_get("kind").map_err(ApiError::internal)?;
    if !matches!(
        old_kind.as_str(),
        "expense" | "income" | "transfer" | "reallocate"
    ) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "This transaction kind cannot be edited.",
        ));
    }
    let old_date: String = old.try_get("effective_date").map_err(ApiError::internal)?;
    let old_month = old_date
        .get(..7)
        .ok_or_else(|| ApiError::internal("stored transaction date is invalid"))?;
    let new_month = payload
        .transaction
        .date
        .get(..7)
        .ok_or_else(|| ApiError::internal("transaction date is invalid"))?;
    if old_month != new_month {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "An edit must stay in its original month.",
        ));
    }
    crate::budget::require_open(&mut transaction, old_month).await
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
    let is_closed: i64 =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM budget_month_closures WHERE month = ?)")
            .bind(old_month)
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
    if is_closed != 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Closed-month transactions cannot be edited. Use the category-only correction operation.",
        ));
    }
    let has_correction: i64 = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM finance_category_corrections WHERE original_transaction_id = ?)")
        .bind(&payload.transaction.transaction_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    if has_correction != 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "A transaction with a recorded category correction cannot be edited.",
        ));
    }

    let write = &payload.transaction;
    let mut account_ids = Vec::<String>::new();
    let mut category_ids = Vec::<String>::new();
    match write.kind.as_str() {
        "expense" => {
            if write.to_account_id.is_some()
                || write.to_category_id.is_some()
                || write.to_amount.is_some()
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Expense transactions accept only a from account and category.",
                ));
            }
            account_ids.push(required_id(
                write.from_account_id.as_deref(),
                "fromAccountId",
            )?);
            category_ids.push(required_id(
                write.from_category_id.as_deref(),
                "fromCategoryId",
            )?);
        }
        "income" => {
            if write.from_account_id.is_some()
                || write.from_category_id.is_some()
                || write.to_amount.is_some()
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Income transactions accept only a to account and category.",
                ));
            }
            account_ids.push(required_id(write.to_account_id.as_deref(), "toAccountId")?);
            category_ids.push(required_id(
                write.to_category_id.as_deref(),
                "toCategoryId",
            )?);
        }
        "transfer" => {
            if write.from_category_id.is_some() || write.to_category_id.is_some() {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Transfer categories are assigned automatically.",
                ));
            }
            account_ids.push(required_id(
                write.from_account_id.as_deref(),
                "fromAccountId",
            )?);
            account_ids.push(required_id(write.to_account_id.as_deref(), "toAccountId")?);
        }
        "reallocate" => {
            if write.from_account_id.is_some()
                || write.to_account_id.is_some()
                || write.currency.is_some()
                || write.to_amount.is_some()
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Reallocations accept only two categories and a EUR amount.",
                ));
            }
            category_ids.push(required_id(
                write.from_category_id.as_deref(),
                "fromCategoryId",
            )?);
            category_ids.push(required_id(
                write.to_category_id.as_deref(),
                "toCategoryId",
            )?);
        }
        _ => unreachable!(),
    }
    if write.kind == "transfer" && account_ids[0] == account_ids[1] {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Transfer accounts must differ.",
        ));
    }
    if write.kind == "reallocate" && category_ids[0] == category_ids[1] {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Reallocation categories must differ.",
        ));
    }
    let mut accounts = HashMap::<String, DbWriteAccount>::new();
    for account_id in &account_ids {
        let account = load_db_account(&mut transaction, account_id).await?;
        if !account.is_real
            || !account.is_active
            || !matches!(account.currency.as_str(), "EUR" | "PHP" | "USD")
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Transactions must use active real accounts with a supported currency.",
            ));
        }
        accounts.insert(account_id.clone(), account);
    }
    let mut categories = HashMap::<String, DbWriteCategory>::new();
    for category_id in &category_ids {
        let category = load_db_category(&mut transaction, category_id).await?;
        if !category.is_real {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Transactions must use real budget categories.",
            ));
        }
        categories.insert(category_id.clone(), category);
    }
    let user_from_account = write
        .from_account_id
        .as_deref()
        .and_then(|id| accounts.get(id))
        .cloned();
    let user_to_account = write
        .to_account_id
        .as_deref()
        .and_then(|id| accounts.get(id))
        .cloned();
    let user_from_category = write
        .from_category_id
        .as_deref()
        .and_then(|id| categories.get(id))
        .cloned();
    let user_to_category = write
        .to_category_id
        .as_deref()
        .and_then(|id| categories.get(id))
        .cloned();
    let (from_account, to_account, from_category, to_category) = match write.kind.as_str() {
        "expense" => (
            user_from_account,
            Some(load_system_db_account(&mut transaction, "Expense").await?),
            user_from_category,
            Some(load_system_db_category(&mut transaction, "Expense").await?),
        ),
        "income" => (
            Some(load_system_db_account(&mut transaction, "Income").await?),
            user_to_account,
            Some(load_system_db_category(&mut transaction, "Income").await?),
            user_to_category,
        ),
        "transfer" => (
            user_from_account,
            user_to_account,
            Some(load_system_db_category(&mut transaction, "Balance Transfer").await?),
            Some(load_system_db_category(&mut transaction, "Balance Transfer").await?),
        ),
        "reallocate" => (
            Some(load_system_db_account(&mut transaction, "Reallocation").await?),
            Some(load_system_db_account(&mut transaction, "Reallocation").await?),
            user_from_category,
            user_to_category,
        ),
        _ => unreachable!(),
    };
    let from_account =
        from_account.ok_or_else(|| ApiError::internal("normalized from account is missing"))?;
    let to_account =
        to_account.ok_or_else(|| ApiError::internal("normalized to account is missing"))?;
    let from_category =
        from_category.ok_or_else(|| ApiError::internal("normalized from category is missing"))?;
    let to_category =
        to_category.ok_or_else(|| ApiError::internal("normalized to category is missing"))?;
    let default_input_currency = match write.kind.as_str() {
        "income" => to_account.currency.as_str(),
        "reallocate" => "EUR",
        _ => from_account.currency.as_str(),
    };
    let input_currency = write
        .currency
        .as_deref()
        .unwrap_or(default_input_currency)
        .to_string();
    if write.kind == "reallocate" && input_currency != "EUR" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Reallocation amounts use EUR.",
        ));
    }
    let cross_currency_transfer =
        write.kind == "transfer" && from_account.currency != to_account.currency;
    if write.kind == "transfer" && cross_currency_transfer != write.to_amount.is_some() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            if cross_currency_transfer {
                "Cross-currency transfers require toAmount."
            } else {
                "Same-currency transfers must not include toAmount."
            },
        ));
    }
    let valuation_to_amount = write.to_amount.as_deref().unwrap_or(&write.amount);

    let old_kind = old_kind;
    let old_amount: String = old.try_get("amount_decimal").map_err(ApiError::internal)?;
    let old_input_currency: Option<String> =
        old.try_get("input_currency").map_err(ApiError::internal)?;
    let old_to_amount: Option<String> = old
        .try_get("to_amount_decimal")
        .map_err(ApiError::internal)?;
    let old_from_account_id: String = old.try_get("from_account_id").map_err(ApiError::internal)?;
    let old_to_account_id: String = old.try_get("to_account_id").map_err(ApiError::internal)?;
    let old_from_category_id: String = old
        .try_get("from_category_id")
        .map_err(ApiError::internal)?;
    let old_to_category_id: String = old.try_get("to_category_id").map_err(ApiError::internal)?;
    let old_from_account = load_db_account(&mut transaction, &old_from_account_id).await?;
    let old_to_account = load_db_account(&mut transaction, &old_to_account_id).await?;
    let old_input_currency = old_input_currency.unwrap_or_else(|| match old_kind.as_str() {
        "income" => old_to_account.currency.clone(),
        "reallocate" => "EUR".into(),
        _ => old_from_account.currency.clone(),
    });
    let old_valuation_to_amount = if old_kind == "transfer" {
        old_to_amount.unwrap_or_else(|| old_amount.clone())
    } else {
        old_amount.clone()
    };
    let old_from_valuation =
        load_stored_side_valuation(&mut transaction, &write.transaction_id, "from").await?;
    let old_to_valuation =
        load_stored_side_valuation(&mut transaction, &write.transaction_id, "to").await?;
    let old_valuation = crate::finance_model::TransactionValuation {
        from: old_from_valuation,
        to: old_to_valuation,
    };
    let old_projection =
        crate::finance_model::project_transaction(&crate::finance_model::TransactionEffect {
            from_account_id: Some(old_from_account_id.clone()),
            to_account_id: Some(old_to_account_id.clone()),
            from_category_id: Some(old_from_category_id.clone()),
            to_category_id: Some(old_to_category_id.clone()),
            kind: old_kind.clone(),
            amount: old_amount.clone(),
            valuation: old_valuation.clone(),
        })
        .map_err(ApiError::internal)?;

    let inputs_unchanged = old_amount == write.amount
        && old_date == write.date
        && old_input_currency == input_currency
        && old_from_account.currency == from_account.currency
        && old_to_account.currency == to_account.currency
        && old_valuation_to_amount == valuation_to_amount;
    let mut observations = Vec::new();
    for row in sqlx::query("SELECT quote_currency, rate_date, rate_decimal FROM exchange_rates WHERE provider = 'ECB' AND base_currency = 'EUR' AND quote_currency IN ('PHP', 'USD') AND rate_date IS NOT NULL AND rate_decimal IS NOT NULL")
        .fetch_all(&mut *transaction)
        .await
        .map_err(ApiError::internal)?
    {
        observations.push(crate::finance_model::RateObservation {
            currency: row.try_get("quote_currency").map_err(ApiError::internal)?,
            date: row.try_get("rate_date").map_err(ApiError::internal)?,
            currency_units_per_eur: row.try_get("rate_decimal").map_err(ApiError::internal)?,
        });
    }
    let valuation = if inputs_unchanged {
        old_valuation.clone()
    } else {
        crate::finance_model::calculate_valuation(
            &write.amount,
            &input_currency,
            &from_account.currency,
            valuation_to_amount,
            &to_account.currency,
            &write.date,
            &observations,
            crate::finance_model::POSTING_ROUNDING_MODE,
        )
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?
    };
    let new_projection =
        crate::finance_model::project_transaction(&crate::finance_model::TransactionEffect {
            from_account_id: Some(from_account.id.clone()),
            to_account_id: Some(to_account.id.clone()),
            from_category_id: Some(from_category.id.clone()),
            to_category_id: Some(to_category.id.clone()),
            kind: write.kind.clone(),
            amount: write.amount.clone(),
            valuation: valuation.clone(),
        })
        .map_err(ApiError::internal)?;

    let account_deltas = difference_maps(
        &new_projection.account_deltas,
        &old_projection.account_deltas,
    )
    .map_err(ApiError::internal)?;

    let mut involved_account_ids = HashSet::new();
    for account in [
        &old_from_account,
        &old_to_account,
        &from_account,
        &to_account,
    ] {
        if account.is_real {
            involved_account_ids.insert(account.id.clone());
        }
    }
    if expected_revisions
        .keys()
        .any(|id| !involved_account_ids.contains(id))
        || involved_account_ids
            .iter()
            .any(|id| !expected_revisions.contains_key(id))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Provide the current ledger revision for every real account affected by this edit.",
        ));
    }
    for account_id in &involved_account_ids {
        let revision: i64 =
            sqlx::query_scalar("SELECT ledger_revision FROM finance_accounts WHERE id = ?")
                .bind(account_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        if Some(&revision) != expected_revisions.get(account_id) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "An account changed after it was loaded. Refresh and retry the edit.",
            ));
        }
    }

    let next_revision = old_revision + 1;
    let to_amount_override = write.kind == "transfer" && write.to_amount.is_some();
    let stored_to_amount = if to_amount_override {
        write.to_amount.clone()
    } else {
        None
    };
    let stored_to_currency = if write.kind == "transfer" {
        Some(to_account.currency.clone())
    } else {
        None
    };
    let currency_override = input_currency != default_input_currency;
    let old_occurred_at: Option<String> = old.try_get("occurred_at").map_err(ApiError::internal)?;
    let occurred_at = if !write.date_only && write.occurred_at.is_none() && old_date == write.date {
        old_occurred_at
    } else {
        resolve_transaction_timestamp(
            &mut transaction,
            &write.date,
            write.occurred_at.as_deref(),
            write.date_only,
        )
        .await?
    };
    validate_db_correction(&mut transaction, write).await?;
    let new_source_json = json!({
        "operationId": write.operation_id,
        "transactionId": write.transaction_id,
        "kind": write.kind,
        "incomeTreatment": write.income_treatment,
        "correctsTransactionId": write.corrects_transaction_id,
        "expectedIncomeMonth": write.expected_income_month,
        "date": write.date,
        "occurredAt": occurred_at,
        "description": write.description,
        "amount": write.amount,
        "inputCurrency": input_currency,
        "toAmount": stored_to_amount,
        "toCurrency": stored_to_currency,
        "fromAccountId": from_account.id,
        "toAccountId": to_account.id,
        "fromCategoryId": from_category.id,
        "toCategoryId": to_category.id,
    });
    let source_json_text = serde_json::to_string(&new_source_json).map_err(ApiError::internal)?;
    sqlx::query("UPDATE finance_transactions SET kind = ?, effective_date = ?, description = ?, amount_decimal = ?, input_currency = ?, to_amount_decimal = ?, to_currency = ?, from_category_id = ?, to_category_id = ?, from_account_id = ?, to_account_id = ?, currency_is_override = ?, to_amount_is_override = ?, revision = ?, source_row_json = ?, occurred_at = ? WHERE id = ?")
        .bind(&write.kind)
        .bind(&write.date)
        .bind(&write.description)
        .bind(&write.amount)
        .bind(&input_currency)
        .bind(&stored_to_amount)
        .bind(&stored_to_currency)
        .bind(&from_category.id)
        .bind(&to_category.id)
        .bind(&from_account.id)
        .bind(&to_account.id)
        .bind(currency_override)
        .bind(to_amount_override)
        .bind(next_revision)
        .bind(&source_json_text)
        .bind(&occurred_at)
        .bind(&write.transaction_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    if !inputs_unchanged {
        for (side, side_valuation, currency) in [
            ("from", &valuation.from, from_account.currency.as_str()),
            ("to", &valuation.to, to_account.currency.as_str()),
        ] {
            let audit_json = serde_json::to_string(&valuation_side_json(side_valuation))
                .map_err(ApiError::internal)?;
            let exchange_to_eur = format!(
                "{}/{}",
                side_valuation.exchange_to_eur_numerator,
                side_valuation.exchange_to_eur_denominator
            );
            let exchange_to_native = format!(
                "{}/{}",
                side_valuation.exchange_to_native_numerator,
                side_valuation.exchange_to_native_denominator
            );
            sqlx::query("INSERT INTO transaction_valuations (transaction_id, side, account_currency, exchange_to_eur, exchange_to_native, amount_native, amount_eur, effective_date, valuation_source, rate_set_version, revision, source_cells_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'canonical-db-policy', 'ecb-prior-rate-fx-uplift-1.02-half-even-v1', ?, ?)")
                .bind(&write.transaction_id)
                .bind(side)
                .bind(currency)
                .bind(exchange_to_eur)
                .bind(exchange_to_native)
                .bind(&side_valuation.amount_native)
                .bind(&side_valuation.amount_eur)
                .bind(&write.date)
                .bind(next_revision)
                .bind(audit_json)
                .execute(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        }
    }
    let mut changed_accounts = Vec::new();
    let current_as_of = crate::budget::today();
    for (account_id, delta) in &account_deltas {
        if delta == "0" {
            continue;
        }
        let account = load_db_account(&mut transaction, account_id).await?;
        if !account.is_real || !account.is_active {
            continue;
        }
        let old_balance: Option<String> =
            sqlx::query_scalar("SELECT current_native_balance FROM finance_accounts WHERE id = ?")
                .bind(account_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        let old_balance = old_balance.ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "A touched account has no canonical native balance.",
            )
        })?;
        let new_balance = crate::finance_model::add_decimal_text(&old_balance, delta)
            .map_err(ApiError::internal)?;
        let new_eur = crate::finance_model::account_balance_eur(
            &new_balance,
            &account.currency,
            &current_as_of,
            &observations,
        )
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
        sqlx::query("UPDATE finance_accounts SET current_native_balance = ?, current_eur_balance = ?, ledger_revision = ledger_revision + 1 WHERE id = ?")
            .bind(&new_balance)
            .bind(&new_eur)
            .bind(account_id)
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
        let revision: i64 =
            sqlx::query_scalar("SELECT ledger_revision FROM finance_accounts WHERE id = ?")
                .bind(account_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        changed_accounts.push(json!({ "accountId": account_id, "balanceNative": new_balance, "balanceEur": new_eur, "ledgerRevision": revision }));
    }
    crate::budget_ledger::receipt(
        &mut transaction,
        &write.transaction_id,
        write.income_treatment.as_deref(),
        write.expected_income_month.as_deref(),
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::budget_ledger::sync(&mut transaction, Some(&write.operation_id))
        .await
        .map_err(ApiError::internal)?;
    let changed_categories = crate::budget::categories(&mut transaction, old_month)
        .await
        .map_err(ApiError::internal)?;
    let recorded_at: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    let revision_snapshot = json!({
        "operationId": write.operation_id,
        "recordedBy": pocket_session.subject,
        "recordedByName": pocket_session.username,
        "transactionId": write.transaction_id,
        "previousRevision": old_revision,
        "revision": next_revision,
        "request": new_source_json,
        "valuationPreserved": inputs_unchanged,
        "fromValuation": valuation_side_json(&valuation.from),
        "toValuation": valuation_side_json(&valuation.to),
        "expectedAccountRevisions": payload.expected_account_revisions,
    });
    sqlx::query("INSERT INTO finance_transaction_revisions (transaction_id, revision, operation_id, snapshot_json, recorded_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&write.transaction_id)
        .bind(next_revision)
        .bind(&write.operation_id)
        .bind(serde_json::to_string(&revision_snapshot).map_err(ApiError::internal)?)
        .bind(&recorded_at)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    let response = json!({
        "updated": true,
        "operationId": write.operation_id,
        "transactionId": write.transaction_id,
        "revision": next_revision,
        "valuationStale": valuation.from.stale || valuation.to.stale,
        "accounts": changed_accounts,
        "categories": changed_categories,
    });
    sqlx::query("UPDATE finance_operations SET response_json = ? WHERE operation_id = ?")
        .bind(serde_json::to_string(&response).map_err(ApiError::internal)?)
        .bind(&write.operation_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(Json(response))
}

async fn delete_db_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<DbTransactionDeleteRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let pocket_session = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_transaction_writes_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database transaction writes are not enabled.",
        ));
    }
    payload.operation_id = payload.operation_id.trim().to_string();
    payload.transaction_id = payload.transaction_id.trim().to_string();
    if !is_uuid_v7(&payload.operation_id)
        || !is_uuid_v7(&payload.transaction_id)
        || payload.expected_revision < 1
        || payload.expected_account_revisions.len() > 100
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "operationId and transactionId must be UUIDv7 values and valid revisions are required.",
        ));
    }
    let mut expected_revisions = BTreeMap::new();
    for expected in &mut payload.expected_account_revisions {
        expected.account_id = expected.account_id.trim().to_string();
        if expected.account_id.is_empty()
            || expected.account_id.len() > 256
            || expected.revision < 0
            || expected_revisions.insert(expected.account_id.clone(), expected.revision).is_some()
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Expected account revisions must have unique account IDs and nonnegative values.",
            ));
        }
    }
    payload
        .expected_account_revisions
        .sort_by(|left, right| left.account_id.cmp(&right.account_id));
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(ApiError::internal)?,
    ));
    let mut transaction = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::internal)?;
    crate::budget::initialize(&mut transaction)
        .await
        .map_err(ApiError::internal)?;
    let inserted = sqlx::query("INSERT INTO finance_operations (operation_id, actor_subject, payload_fingerprint, affected_transaction_ids_json, response_json, created_at) VALUES (?, ?, ?, ?, '{}', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) ON CONFLICT(operation_id) DO NOTHING")
        .bind(&payload.operation_id)
        .bind(&pocket_session.subject)
        .bind(&fingerprint)
        .bind(serde_json::to_string(&[&payload.transaction_id]).map_err(ApiError::internal)?)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    if inserted.rows_affected() == 0 {
        let previous = sqlx::query("SELECT actor_subject, payload_fingerprint, response_json FROM finance_operations WHERE operation_id = ?")
            .bind(&payload.operation_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::internal("finance operation conflict disappeared"))?;
        let actor: String = previous.try_get("actor_subject").map_err(ApiError::internal)?;
        let previous_fingerprint: String = previous.try_get("payload_fingerprint").map_err(ApiError::internal)?;
        let response_text: String = previous.try_get("response_json").map_err(ApiError::internal)?;
        let response: Value = serde_json::from_str(&response_text).map_err(ApiError::internal)?;
        if actor != pocket_session.subject
            || previous_fingerprint != fingerprint
            || response["deleted"] != true
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "operationId was already used for a different finance operation.",
            ));
        }
        transaction.commit().await.map_err(ApiError::internal)?;
        return Ok(Json(response));
    }

    let old = sqlx::query("SELECT id, kind, effective_date, description, amount_decimal, input_currency, from_category_id, to_category_id, from_account_id, to_account_id, revision, source_workbook_id, source_sheet, source_row, original_row_fingerprint, source_row_json FROM finance_transactions WHERE id = ?")
        .bind(&payload.transaction_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "The transaction to delete was not found."))?;
    let source_workbook_id: String = old.try_get("source_workbook_id").map_err(ApiError::internal)?;
    if source_workbook_id != "canonical-live" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Imported source transactions cannot be deleted through this route.",
        ));
    }
    let old_revision: i64 = old.try_get("revision").map_err(ApiError::internal)?;
    if old_revision != payload.expected_revision {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "The transaction changed after it was loaded. Refresh and retry the deletion.",
        ));
    }
    let kind: String = old.try_get("kind").map_err(ApiError::internal)?;
    if !matches!(
        kind.as_str(),
        "expense" | "income" | "transfer" | "reallocate"
    ) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "This transaction kind cannot be deleted.",
        ));
    }
    let has_dependent_audit: i64 = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM finance_category_corrections WHERE original_transaction_id = ?) OR EXISTS (SELECT 1 FROM reconciliation_items WHERE transaction_id = ?)")
        .bind(&payload.transaction_id).bind(&payload.transaction_id)
        .fetch_one(&mut *transaction).await.map_err(ApiError::internal)?;
    if has_dependent_audit != 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "This transaction is referenced by a reconciliation or category correction and cannot be deleted.",
        ));
    }
    let effective_date: String = old.try_get("effective_date").map_err(ApiError::internal)?;
    let month = effective_date.get(..7).ok_or_else(|| ApiError::internal("stored transaction date is invalid"))?;
    crate::budget::require_open(&mut transaction, month).await
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
    let closed: i64 = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM budget_month_closures WHERE month = ?)")
        .bind(month).fetch_one(&mut *transaction).await.map_err(ApiError::internal)?;
    if closed != 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Transactions in a closed budget month cannot be deleted.",
        ));
    }

    let from_account_id: String = old.try_get("from_account_id").map_err(ApiError::internal)?;
    let to_account_id: String = old.try_get("to_account_id").map_err(ApiError::internal)?;
    let from_category_id: String = old.try_get("from_category_id").map_err(ApiError::internal)?;
    let to_category_id: String = old.try_get("to_category_id").map_err(ApiError::internal)?;
    let old_amount: String = old.try_get("amount_decimal").map_err(ApiError::internal)?;
    let from_account = load_db_account(&mut transaction, &from_account_id).await?;
    let to_account = load_db_account(&mut transaction, &to_account_id).await?;
    let from_valuation =
        load_stored_side_valuation(&mut transaction, &payload.transaction_id, "from").await?;
    let to_valuation =
        load_stored_side_valuation(&mut transaction, &payload.transaction_id, "to").await?;
    let valuation = crate::finance_model::TransactionValuation {
        from: from_valuation,
        to: to_valuation,
    };
    let projection =
        crate::finance_model::project_transaction(&crate::finance_model::TransactionEffect {
            from_account_id: Some(from_account_id.clone()),
            to_account_id: Some(to_account_id.clone()),
            from_category_id: Some(from_category_id.clone()),
            to_category_id: Some(to_category_id.clone()),
            kind: kind.clone(),
            amount: old_amount.clone(),
            valuation: valuation.clone(),
        })
        .map_err(ApiError::internal)?;
    let involved_account_ids: HashSet<String> = [
        (&from_account_id, &from_account),
        (&to_account_id, &to_account),
    ]
    .into_iter()
    .filter(|(_, account)| account.is_real)
    .map(|(id, _)| id.clone())
    .collect();
    if expected_revisions
        .keys()
        .any(|id| !involved_account_ids.contains(id))
        || involved_account_ids
            .iter()
            .any(|id| !expected_revisions.contains_key(id))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Provide the current ledger revision for every real account affected by this deletion.",
        ));
    }
    for account_id in &involved_account_ids {
        let revision: i64 = sqlx::query_scalar("SELECT ledger_revision FROM finance_accounts WHERE id = ?")
            .bind(account_id).fetch_one(&mut *transaction).await.map_err(ApiError::internal)?;
        if Some(&revision) != expected_revisions.get(account_id) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "An account changed after it was loaded. Refresh and retry the deletion.",
            ));
        }
    }

    let revision_audit: Vec<String> = sqlx::query_scalar("SELECT snapshot_json FROM finance_transaction_revisions WHERE transaction_id = ? ORDER BY revision")
        .bind(&payload.transaction_id).fetch_all(&mut *transaction).await.map_err(ApiError::internal)?;
    let valuation_rows = sqlx::query("SELECT side, account_currency, exchange_to_eur, exchange_to_native, amount_native, amount_eur, effective_date, valuation_source, rate_set_version, revision, source_cells_json FROM transaction_valuations WHERE transaction_id = ? ORDER BY side, revision")
        .bind(&payload.transaction_id).fetch_all(&mut *transaction).await.map_err(ApiError::internal)?;
    let valuation_audit: Vec<Value> = valuation_rows.into_iter().map(|row| Ok(json!({
        "side": row.try_get::<String, _>("side").map_err(ApiError::internal)?,
        "accountCurrency": row.try_get::<Option<String>, _>("account_currency").map_err(ApiError::internal)?,
        "exchangeToEur": row.try_get::<Option<String>, _>("exchange_to_eur").map_err(ApiError::internal)?,
        "exchangeToNative": row.try_get::<Option<String>, _>("exchange_to_native").map_err(ApiError::internal)?,
        "amountNative": row.try_get::<Option<String>, _>("amount_native").map_err(ApiError::internal)?,
        "amountEur": row.try_get::<Option<String>, _>("amount_eur").map_err(ApiError::internal)?,
        "effectiveDate": row.try_get::<String, _>("effective_date").map_err(ApiError::internal)?,
        "valuationSource": row.try_get::<String, _>("valuation_source").map_err(ApiError::internal)?,
        "rateSetVersion": row.try_get::<Option<String>, _>("rate_set_version").map_err(ApiError::internal)?,
        "revision": row.try_get::<i64, _>("revision").map_err(ApiError::internal)?,
        "sourceCells": row.try_get::<String, _>("source_cells_json").map_err(ApiError::internal)?,
    }))).collect::<Result<_, ApiError>>()?;

    let mut observations = Vec::new();
    for row in sqlx::query("SELECT quote_currency, rate_date, rate_decimal FROM exchange_rates WHERE provider = 'ECB' AND base_currency = 'EUR' AND quote_currency IN ('PHP', 'USD') AND rate_date IS NOT NULL AND rate_decimal IS NOT NULL")
        .fetch_all(&mut *transaction).await.map_err(ApiError::internal)?
    {
        observations.push(crate::finance_model::RateObservation {
            currency: row.try_get("quote_currency").map_err(ApiError::internal)?,
            date: row.try_get("rate_date").map_err(ApiError::internal)?,
            currency_units_per_eur: row.try_get("rate_decimal").map_err(ApiError::internal)?,
        });
    }
    let current_as_of = crate::budget::today();
    let mut changed_accounts = Vec::new();
    for (account_id, delta) in &projection.account_deltas {
        if delta == "0" { continue; }
        let account = load_db_account(&mut transaction, account_id).await?;
        if !account.is_real || !account.is_active {
            continue;
        }
        let old_balance: Option<String> =
            sqlx::query_scalar("SELECT current_native_balance FROM finance_accounts WHERE id = ?")
                .bind(account_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        let old_balance = old_balance.ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "A touched account has no canonical native balance.",
            )
        })?;
        let new_balance = crate::finance_model::subtract_decimal_text(&old_balance, delta)
            .map_err(ApiError::internal)?;
        let new_eur = crate::finance_model::account_balance_eur(
            &new_balance,
            &account.currency,
            &current_as_of,
            &observations,
        )
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
        sqlx::query("UPDATE finance_accounts SET current_native_balance = ?, current_eur_balance = ?, ledger_revision = ledger_revision + 1 WHERE id = ?")
            .bind(&new_balance).bind(&new_eur).bind(account_id).execute(&mut *transaction).await.map_err(ApiError::internal)?;
        let revision: i64 = sqlx::query_scalar("SELECT ledger_revision FROM finance_accounts WHERE id = ?")
            .bind(account_id).fetch_one(&mut *transaction).await.map_err(ApiError::internal)?;
        changed_accounts.push(json!({ "accountId": account_id, "balanceNative": new_balance, "balanceEur": new_eur, "ledgerRevision": revision }));
    }

    let deleted_snapshot = json!({
        "id": payload.transaction_id,
        "kind": kind,
        "effectiveDate": effective_date,
        "description": old.try_get::<String, _>("description").map_err(ApiError::internal)?,
        "amount": old_amount,
        "inputCurrency": old.try_get::<Option<String>, _>("input_currency").map_err(ApiError::internal)?,
        "fromCategoryId": from_category_id,
        "toCategoryId": to_category_id,
        "fromAccountId": from_account_id,
        "toAccountId": to_account_id,
        "sourceSheet": old.try_get::<String, _>("source_sheet").map_err(ApiError::internal)?,
        "originalRowFingerprint": old.try_get::<String, _>("original_row_fingerprint").map_err(ApiError::internal)?,
        "sourceRow": old.try_get::<i64, _>("source_row").map_err(ApiError::internal)?,
        "sourceRowJson": old.try_get::<String, _>("source_row_json").map_err(ApiError::internal)?,
        "revision": old_revision,
        "revisionAudit": revision_audit,
        "valuationAudit": valuation_audit,
        "deletedBy": pocket_session.subject,
        "operationId": payload.operation_id,
    });
    sqlx::query("DELETE FROM finance_transactions WHERE id = ?")
        .bind(&payload.transaction_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    crate::budget_ledger::sync(&mut transaction, Some(&payload.operation_id))
        .await
        .map_err(ApiError::internal)?;
    let changed_categories = crate::budget::categories(&mut transaction, month)
        .await
        .map_err(ApiError::internal)?;
    let recorded_at: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(&mut *transaction).await.map_err(ApiError::internal)?;
    let response = json!({
        "deleted": true,
        "operationId": payload.operation_id,
        "transactionId": payload.transaction_id,
        "recordedAt": recorded_at,
        "accounts": changed_accounts,
        "categories": changed_categories,
        "deletedTransaction": deleted_snapshot,
    });
    sqlx::query("UPDATE finance_operations SET response_json = ? WHERE operation_id = ?")
        .bind(serde_json::to_string(&response).map_err(ApiError::internal)?)
        .bind(&payload.operation_id).execute(&mut *transaction).await.map_err(ApiError::internal)?;
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(Json(response))
}

fn valid_transaction_timestamp(value: &str) -> bool {
    if !value.is_ascii()
        || !matches!(value.len(), 20 | 24)
        || !valid_date(&value[..10])
        || &value[10..11] != "T"
        || &value[13..14] != ":"
        || &value[16..17] != ":"
        || !value.ends_with('Z')
    {
        return false;
    }
    let valid_time = [(11..13, 23), (14..16, 59), (17..19, 59)]
        .into_iter()
        .all(|(range, max)| {
            value[range.clone()]
                .bytes()
                .all(|byte| byte.is_ascii_digit())
                && value[range]
                    .parse::<u32>()
                    .is_ok_and(|number| number <= max)
        });
    valid_time
        && (value.len() == 20
            || (&value[19..20] == "." && value[20..23].bytes().all(|byte| byte.is_ascii_digit())))
}

async fn resolve_transaction_timestamp(
    connection: &mut sqlx::SqliteConnection,
    date: &str,
    supplied: Option<&str>,
    date_only: bool,
) -> Result<Option<String>, ApiError> {
    if date_only {
        return Ok(None);
    }
    if let Some(timestamp) = supplied {
        let age: Option<f64> =
            sqlx::query_scalar("SELECT (julianday('now') - julianday(?)) * 86400")
                .bind(timestamp)
                .fetch_one(&mut *connection)
                .await
                .map_err(ApiError::internal)?;
        let date_distance: Option<f64> =
            sqlx::query_scalar("SELECT abs(julianday(?) - julianday(?))")
                .bind(date)
                .bind(&timestamp[..10])
                .fetch_one(connection)
                .await
                .map_err(ApiError::internal)?;
        if age.is_none_or(|seconds| seconds < -60.0) || date_distance.is_none_or(|days| days > 1.0)
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "The transaction timestamp must match its posting date and cannot be in the future.",
            ));
        }
        return Ok(Some(timestamp.to_string()));
    }
    let now: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(connection)
        .await
        .map_err(ApiError::internal)?;
    Ok((date == crate::budget::today()).then_some(now))
}

fn normalize_db_transaction_write(payload: &mut DbTransactionWriteRequest) -> Result<(), ApiError> {
    payload.kind = payload.kind.trim().to_ascii_lowercase();
    payload.date = payload.date.trim().to_string();
    payload.description = payload.description.trim().to_string();
    if payload
        .occurred_at
        .as_deref()
        .is_some_and(|value| !valid_transaction_timestamp(value))
        || (payload.date_only && payload.occurred_at.is_some())
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Use a full UTC timestamp, or a date-only transaction.",
        ));
    }

    if !valid_date(&payload.date)
        || payload.description.is_empty()
        || payload.description.chars().count() > 500
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "A valid date and a description of at most 500 characters are required.",
        ));
    }
    if !matches!(
        payload.kind.as_str(),
        "expense" | "income" | "transfer" | "reallocate"
    ) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Unsupported transaction kind.",
        ));
    }
    let round_positive = |value: &str, field: &'static str| {
        if value.len() > 64 {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("{field} must be a valid positive amount."),
            ));
        }
        let rounded = crate::finance_model::round_posted_decimal_text(value, 2).map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("{field} must be a valid positive amount."),
            )
        })?;
        if rounded == "0" || rounded.starts_with('-') {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("{field} must be greater than zero."),
            ));
        }
        Ok(rounded)
    };
    payload.amount = round_positive(&payload.amount, "amount")?;
    payload.to_amount = payload
        .to_amount
        .as_deref()
        .map(|amount| round_positive(amount, "toAmount"))
        .transpose()?;
    payload.currency = payload
        .currency
        .take()
        .map(|currency| currency.trim().to_ascii_uppercase())
        .filter(|currency| !currency.is_empty());
    if payload
        .currency
        .as_deref()
        .is_some_and(|currency| !matches!(currency, "EUR" | "PHP" | "USD"))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "currency must be EUR, PHP, or USD.",
        ));
    }
    if payload
        .income_treatment
        .as_deref()
        .is_some_and(|t| !matches!(t, "salary" | "unexpected" | "refund"))
        || (payload
            .expected_income_month
            .as_deref()
            .is_some_and(|m| !valid_month(m)))
        || (payload.kind != "income"
            && (payload.income_treatment.is_some() || payload.expected_income_month.is_some()))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Choose valid income treatment and expected-income month.",
        ));
    }
    for value in [
        &mut payload.from_category_id,
        &mut payload.to_category_id,
        &mut payload.from_account_id,
        &mut payload.to_account_id,
    ] {
        *value = value
            .take()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());
    }
    if payload.kind == "income"
        && payload.to_category_id.is_none()
        && payload.income_treatment.as_deref() != Some("refund")
    {
        payload.to_category_id = Some(crate::budget_ledger::UNALLOCATED.into());
    }
    if payload.kind == "expense"
        && payload.from_category_id.as_deref() == Some(crate::budget_ledger::UNALLOCATED)
        || payload.income_treatment.as_deref() == Some("refund")
            && payload.to_category_id.as_deref() == Some(crate::budget_ledger::UNALLOCATED)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Allocate Unallocated funds to a spending category first; refunds restore a spending category.",
        ));
    }
    Ok(())
}

async fn validate_db_correction(c:&mut sqlx::SqliteConnection,payload:&DbTransactionWriteRequest)->Result<(),ApiError> {
    let Some(original)=payload.corrects_transaction_id.as_deref() else {return Ok(());};
    if original==payload.transaction_id || !matches!(payload.kind.as_str(),"expense"|"income") || payload.date[..7]!=crate::budget::today()[..7] {
        return Err(ApiError::new(StatusCode::BAD_REQUEST,"Record account corrections as expenses or income in the current open month."));
    }
    let date:Option<String>=sqlx::query_scalar("SELECT effective_date FROM finance_transactions WHERE id=?").bind(original).fetch_optional(&mut *c).await.map_err(ApiError::internal)?;
    let date=date.ok_or_else(||ApiError::new(StatusCode::NOT_FOUND,"Original transaction not found."))?;
    let first=crate::budget::first_month(c).await.map_err(ApiError::internal)?;
    let closed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month=?)").bind(&date[..7]).fetch_one(&mut *c).await.map_err(ApiError::internal)?;
    if !closed && first.as_deref().is_none_or(|m|&date[..7]>=m){return Err(ApiError::new(StatusCode::CONFLICT,"Edit the original transaction while its month is open."));}
    Ok(())
}

fn required_id(value: Option<&str>, field: &'static str) -> Result<String, ApiError> {
    value
        .filter(|value| value.len() <= 256)
        .map(str::to_string)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, format!("{field} is required.")))
}

async fn load_db_account(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: &str,
) -> Result<DbWriteAccount, ApiError> {
    let row =
        sqlx::query("SELECT id, currency, is_real, is_active FROM finance_accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "An account was not found."))?;
    Ok(DbWriteAccount {
        id: row.try_get("id").map_err(ApiError::internal)?,
        currency: row
            .try_get::<Option<String>, _>("currency")
            .map_err(ApiError::internal)?
            .unwrap_or_else(|| "EUR".into()),
        is_real: row
            .try_get::<i64, _>("is_real")
            .map_err(ApiError::internal)?
            != 0,
        is_active: row
            .try_get::<i64, _>("is_active")
            .map_err(ApiError::internal)?
            != 0,
    })
}

async fn load_system_db_account(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    name: &str,
) -> Result<DbWriteAccount, ApiError> {
    let rows = sqlx::query("SELECT id, currency, is_real, is_active FROM finance_accounts WHERE source_name = ? COLLATE NOCASE")
        .bind(name)
        .fetch_all(&mut **transaction)
        .await
        .map_err(ApiError::internal)?;
    if rows.len() != 1 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "A required system account is missing or ambiguous.",
        ));
    }
    let row = &rows[0];
    let account = DbWriteAccount {
        id: row.try_get("id").map_err(ApiError::internal)?,
        currency: row
            .try_get::<Option<String>, _>("currency")
            .map_err(ApiError::internal)?
            .unwrap_or_else(|| "EUR".into()),
        is_real: row
            .try_get::<i64, _>("is_real")
            .map_err(ApiError::internal)?
            != 0,
        is_active: row
            .try_get::<i64, _>("is_active")
            .map_err(ApiError::internal)?
            != 0,
    };
    if account.is_real {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "A required system account is configured as a real account.",
        ));
    }
    Ok(account)
}

async fn load_db_category(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    category_id: &str,
) -> Result<DbWriteCategory, ApiError> {
    let row = sqlx::query("SELECT id, is_real FROM finance_categories WHERE id = ?")
        .bind(category_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(ApiError::internal)?;
    let row =
        row.ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "A category was not found."))?;
    Ok(DbWriteCategory {
        id: row.try_get("id").map_err(ApiError::internal)?,
        is_real: row
            .try_get::<i64, _>("is_real")
            .map_err(ApiError::internal)?
            != 0,
    })
}

async fn load_system_db_category(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    name: &str,
) -> Result<DbWriteCategory, ApiError> {
    let rows =
        sqlx::query("SELECT id, is_real FROM finance_categories WHERE name = ? COLLATE NOCASE")
            .bind(name)
            .fetch_all(&mut **transaction)
            .await
            .map_err(ApiError::internal)?;
    if rows.len() != 1 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "A required system category is missing or ambiguous.",
        ));
    }
    let category = DbWriteCategory {
        id: rows[0].try_get("id").map_err(ApiError::internal)?,
        is_real: rows[0]
            .try_get::<i64, _>("is_real")
            .map_err(ApiError::internal)?
            != 0,
    };
    if category.is_real {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "A required system category is configured as a real category.",
        ));
    }
    Ok(category)
}

fn valuation_side_json(side: &crate::finance_model::SideValuation) -> Value {
    json!({
        "amountNative": side.amount_native,
        "amountEur": side.amount_eur,
        "exchangeToEur": { "numerator": side.exchange_to_eur_numerator, "denominator": side.exchange_to_eur_denominator },
        "exchangeToNative": { "numerator": side.exchange_to_native_numerator, "denominator": side.exchange_to_native_denominator },
        "appliedRateDates": side.applied_rate_dates,
        "stale": side.stale,
    })
}

async fn load_stored_side_valuation(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    transaction_id: &str,
    side: &str,
) -> Result<crate::finance_model::SideValuation, ApiError> {
    let row = sqlx::query("SELECT amount_native, amount_eur, source_cells_json FROM transaction_valuations WHERE transaction_id = ? AND side = ? ORDER BY revision DESC LIMIT 1")
        .bind(transaction_id)
        .bind(side)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "The transaction has no saved valuation to revise."))?;
    let amount_native: Option<String> = row.try_get("amount_native").map_err(ApiError::internal)?;
    let amount_eur: Option<String> = row.try_get("amount_eur").map_err(ApiError::internal)?;
    let source_cells: String = row
        .try_get("source_cells_json")
        .map_err(ApiError::internal)?;
    let audit: Value = serde_json::from_str(&source_cells).map_err(ApiError::internal)?;
    let ratio = |name: &str| {
        let value = audit.get(name)?;
        Some((
            value.get("numerator")?.as_str()?.to_string(),
            value.get("denominator")?.as_str()?.to_string(),
        ))
    };
    let (exchange_to_eur_numerator, exchange_to_eur_denominator) =
        ratio("exchangeToEur").unwrap_or_else(|| ("1".into(), "1".into()));
    let (exchange_to_native_numerator, exchange_to_native_denominator) =
        ratio("exchangeToNative").unwrap_or_else(|| ("1".into(), "1".into()));
    let applied_rate_dates = audit
        .get("appliedRateDates")
        .and_then(Value::as_object)
        .map(|dates| {
            dates
                .iter()
                .map(|(currency, date)| (currency.clone(), date.as_str().map(str::to_string)))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    Ok(crate::finance_model::SideValuation {
        amount_native: amount_native.ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "The saved transaction valuation is incomplete.",
            )
        })?,
        amount_eur: amount_eur.ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "The saved transaction valuation is incomplete.",
            )
        })?,
        exchange_to_eur_numerator,
        exchange_to_eur_denominator,
        exchange_to_native_numerator,
        exchange_to_native_denominator,
        applied_rate_dates,
        stale: audit.get("stale").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn difference_maps(
    newer: &BTreeMap<String, String>,
    older: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    let keys = newer
        .keys()
        .chain(older.keys())
        .cloned()
        .collect::<HashSet<_>>();
    let mut differences = BTreeMap::new();
    for key in keys {
        let newer_value = newer.get(&key).map(String::as_str).unwrap_or("0");
        let older_value = older.get(&key).map(String::as_str).unwrap_or("0");
        let difference = crate::finance_model::subtract_decimal_text(newer_value, older_value)?;
        if difference != "0" {
            differences.insert(key, difference);
        }
    }
    Ok(differences)
}

async fn record_category_correction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<CategoryCorrectionRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let pocket_session = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_category_corrections_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database category corrections are not enabled.",
        ));
    }
    if !is_uuid_v7(&payload.operation_id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "operationId must be a UUIDv7.",
        ));
    }
    payload.original_transaction_id = payload.original_transaction_id.trim().to_string();
    payload.reason = payload.reason.trim().to_string();
    if payload.original_transaction_id.is_empty()
        || payload.original_transaction_id.len() > 256
        || payload.reason.is_empty()
        || payload.reason.chars().count() > 500
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "A source transaction and a reason of at most 500 characters are required.",
        ));
    }
    if payload.category_deltas.is_empty() || payload.category_deltas.len() > 50 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Provide one to fifty category expense adjustments.",
        ));
    }
    let mut category_ids = HashSet::new();
    for delta in &mut payload.category_deltas {
        delta.category_id = delta.category_id.trim().to_string();
        if delta.category_id.is_empty()
            || delta.category_id.len() > 200
            || !category_ids.insert(delta.category_id.clone())
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Each real category may appear only once.",
            ));
        }
        if delta.expense_delta_eur.len() > 64 {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Category expense adjustments must be valid decimal amounts.",
            ));
        }
        delta.expense_delta_eur =
            crate::finance_model::round_posted_decimal_text(&delta.expense_delta_eur, 2).map_err(
                |_| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "Category expense adjustments must be valid decimal amounts.",
                    )
                },
            )?;
        if delta.expense_delta_eur == "0" {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Category expense adjustments must be nonzero after rounding.",
            ));
        }
    }
    payload
        .category_deltas
        .sort_by(|left, right| left.category_id.cmp(&right.category_id));
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(ApiError::internal)?,
    ));
    let affected_transaction_ids =
        serde_json::to_string(&[&payload.original_transaction_id]).map_err(ApiError::internal)?;
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    crate::budget::initialize(&mut transaction).await.map_err(ApiError::internal)?;

    // Take the SQLite writer lock before selecting the current open month or
    // changing its category snapshots.
    let inserted = sqlx::query(
        "INSERT INTO finance_operations (operation_id, actor_subject, payload_fingerprint, affected_transaction_ids_json, response_json, created_at) VALUES (?, ?, ?, ?, '{}', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) ON CONFLICT(operation_id) DO NOTHING",
    )
    .bind(&payload.operation_id)
    .bind(&pocket_session.subject)
    .bind(&fingerprint)
    .bind(affected_transaction_ids)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    if inserted.rows_affected() == 0 {
        let previous = sqlx::query(
            "SELECT o.actor_subject, o.payload_fingerprint, o.response_json, c.operation_id AS correction_operation FROM finance_operations o LEFT JOIN finance_category_corrections c ON c.operation_id = o.operation_id WHERE o.operation_id = ?",
        )
        .bind(&payload.operation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal("finance operation conflict disappeared"))?;
        let actor: String = previous
            .try_get("actor_subject")
            .map_err(ApiError::internal)?;
        let previous_fingerprint: String = previous
            .try_get("payload_fingerprint")
            .map_err(ApiError::internal)?;
        let correction_operation: Option<String> = previous
            .try_get("correction_operation")
            .map_err(ApiError::internal)?;
        if actor != pocket_session.subject
            || previous_fingerprint != fingerprint
            || correction_operation.as_deref() != Some(payload.operation_id.as_str())
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "operationId was already used for a different finance operation.",
            ));
        }
        let response_text: String = previous
            .try_get("response_json")
            .map_err(ApiError::internal)?;
        let response: Value = serde_json::from_str(&response_text).map_err(ApiError::internal)?;
        transaction.commit().await.map_err(ApiError::internal)?;
        return Ok(Json(response));
    }

    let original_effective_date: String =
        sqlx::query_scalar("SELECT effective_date FROM finance_transactions WHERE id = ?")
            .bind(&payload.original_transaction_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    "The transaction selected for correction was not found.",
                )
            })?;
    let original_month = original_effective_date
        .get(..7)
        .ok_or_else(|| ApiError::internal("stored transaction date is invalid"))?;
    let original_is_closed: i64 =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM budget_month_closures WHERE month = ?)")
            .bind(original_month)
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
    let first = crate::budget::first_month(&mut transaction).await.map_err(ApiError::internal)?;
    let historical = first.as_deref().is_some_and(|first| original_month < first);
    if original_is_closed == 0 && !historical {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Category-only correction is reserved for transactions in closed months.",
        ));
    }
    let applied_month = crate::budget::today()[..7].to_owned();
    crate::budget::require_open(&mut transaction, &applied_month).await
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, error.to_string()))?;
    let mut updated_categories = Vec::with_capacity(payload.category_deltas.len());
    for delta in &payload.category_deltas {
        let is_real: Option<i64> =
            sqlx::query_scalar("SELECT is_real FROM finance_categories WHERE id = ?")
                .bind(&delta.category_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(ApiError::internal)?;
        if is_real != Some(1) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Category adjustments must target real budget categories.",
            ));
        }
        updated_categories.push(json!({
            "categoryId": delta.category_id,
            "expenseDeltaEur": delta.expense_delta_eur,
        }));
    }
    let category_deltas_json =
        serde_json::to_string(&updated_categories).map_err(ApiError::internal)?;
    let recorded_at: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    sqlx::query(
        "INSERT INTO finance_category_corrections (operation_id, original_transaction_id, original_effective_date, applied_month, reason, category_deltas_json, recorded_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&payload.operation_id)
    .bind(&payload.original_transaction_id)
    .bind(&original_effective_date)
    .bind(&applied_month)
    .bind(&payload.reason)
    .bind(category_deltas_json)
    .bind(&recorded_at)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    for delta in &payload.category_deltas {
        sqlx::query(
            "INSERT INTO finance_category_correction_items (operation_id, category_id, expense_delta_eur) VALUES (?, ?, ?)",
        )
        .bind(&payload.operation_id)
        .bind(&delta.category_id)
        .bind(&delta.expense_delta_eur)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    }
    crate::budget_ledger::sync(&mut transaction, Some(&payload.operation_id))
        .await
        .map_err(ApiError::internal)?;
    let response = json!({
        "recorded": true,
        "operationId": payload.operation_id,
        "originalTransactionId": payload.original_transaction_id,
        "originalEffectiveDate": original_effective_date,
        "appliedMonth": applied_month,
        "accountBalancesChanged": false,
        "originalTransactionFlagged": true,
        "categoryCount": payload.category_deltas.len(),
        "categoryDeltas": updated_categories,
        "recordedAt": recorded_at,
    });
    sqlx::query("UPDATE finance_operations SET response_json = ? WHERE operation_id = ?")
        .bind(serde_json::to_string(&response).map_err(ApiError::internal)?)
        .bind(&payload.operation_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(Json(response))
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompleteBudgetIncomeRequest {
    operation_id: String,
    expected_month: String,
}
async fn complete_budget_income(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CompleteBudgetIncomeRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let session = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !state.config.finance_db_transaction_writes_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database finance writes are not enabled.",
        ));
    }
    if !is_uuid_v7(&payload.operation_id) || !valid_month(&payload.expected_month) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Choose a valid expectation and operation ID.",
        ));
    }
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(ApiError::internal)?,
    ));
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(ApiError::internal)?;
    if let Some(row)=sqlx::query("SELECT actor_subject,payload_fingerprint,response_json FROM finance_operations WHERE operation_id=?").bind(&payload.operation_id).fetch_optional(&mut *tx).await.map_err(ApiError::internal)? {
        if row.try_get::<String,_>("actor_subject").map_err(ApiError::internal)?!=session.subject || row.try_get::<String,_>("payload_fingerprint").map_err(ApiError::internal)?!=fingerprint {return Err(ApiError::new(StatusCode::CONFLICT,"operationId was already used for another operation."));}
        let response:String=row.try_get("response_json").map_err(ApiError::internal)?;
        return Ok(Json(serde_json::from_str(&response).map_err(ApiError::internal)?));
    }
    crate::budget_ledger::sync(&mut tx, None)
        .await
        .map_err(ApiError::internal)?;
    let current = crate::budget::today()[..7].to_owned();
    crate::budget::require_open(&mut tx, &current)
        .await
        .map_err(|e| ApiError::new(StatusCode::CONFLICT, e.to_string()))?;
    if payload.expected_month > current {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "A future expectation cannot be completed.",
        ));
    }
    let expectation = crate::budget_ledger::income(&mut tx, &payload.expected_month, &current)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Expected income not found."))?;
    if expectation.completed_month.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "This expectation is already complete.",
        ));
    }
    let response = json!({"completed":true,"expectedMonth":payload.expected_month,"appliedMonth":current,"shortfallEur":expectation.outstanding_eur});
    sqlx::query(
        "INSERT INTO finance_operations VALUES(?,?,?,'[]',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&payload.operation_id)
    .bind(&session.subject)
    .bind(fingerprint)
    .bind(response.to_string())
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal)?;
    sqlx::query("UPDATE budget_plan_income SET completed_month=?,completion_shortfall_eur=?,completion_operation_id=? WHERE month=?").bind(&current).bind(&expectation.outstanding_eur).bind(&payload.operation_id).bind(&payload.expected_month).execute(&mut *tx).await.map_err(ApiError::internal)?;
    crate::budget_ledger::sync(&mut tx, Some(&payload.operation_id))
        .await
        .map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;
    Ok(Json(response))
}

async fn close_budget_month(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CloseBudgetMonthRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_db_request_origin(&state, &headers)?;
    let pocket_session = current_pocket_session(&state, &headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    if !valid_month(&payload.month) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "month must be a valid YYYY-MM month.",
        ));
    }
    if !is_uuid_v7(&payload.operation_id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "operationId must be a UUIDv7.",
        ));
    }

    let fingerprint = hex::encode(Sha256::digest(
        format!("close-budget-month\n{}", payload.month).as_bytes(),
    ));
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await.map_err(ApiError::internal)?;
    crate::budget::initialize(&mut transaction).await.map_err(ApiError::internal)?;

    // This first write takes SQLite's writer lock before reading the values to
    // be frozen, so concurrent finance operations cannot close a stale view.
    let inserted = sqlx::query(
        "INSERT INTO finance_operations (operation_id, actor_subject, payload_fingerprint, affected_transaction_ids_json, response_json, created_at) VALUES (?, ?, ?, '[]', '{}', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) ON CONFLICT(operation_id) DO NOTHING",
    )
    .bind(&payload.operation_id)
    .bind(&pocket_session.subject)
    .bind(&fingerprint)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;

    if inserted.rows_affected() == 0 {
        let previous = sqlx::query(
            "SELECT o.actor_subject, o.payload_fingerprint, o.response_json, c.month FROM finance_operations o LEFT JOIN budget_month_closures c ON c.operation_id = o.operation_id WHERE o.operation_id = ?",
        )
        .bind(&payload.operation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal("operation conflict disappeared"))?;
        let actor: String = previous
            .try_get("actor_subject")
            .map_err(ApiError::internal)?;
        let previous_fingerprint: String = previous
            .try_get("payload_fingerprint")
            .map_err(ApiError::internal)?;
        let previous_month: Option<String> =
            previous.try_get("month").map_err(ApiError::internal)?;
        if actor != pocket_session.subject
            || previous_fingerprint != fingerprint
            || previous_month.as_deref() != Some(payload.month.as_str())
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "operationId was already used for a different finance operation.",
            ));
        }
        let response_text: String = previous
            .try_get("response_json")
            .map_err(ApiError::internal)?;
        let response: Value = serde_json::from_str(&response_text).map_err(ApiError::internal)?;
        transaction.commit().await.map_err(ApiError::internal)?;
        return Ok(Json(response));
    }

    if !state.config.finance_db_close_enabled {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Database month closing is not enabled.",
        ));
    }
    let current_month = crate::budget::today()[..7].to_owned();
    if payload.month >= current_month {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Only a completed budget month can be closed.",
        ));
    }

    let last_day = last_day_of_month(&payload.month)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "Invalid budget month."))?;
    let reconciliation_date = format!("{}-{last_day:02}", payload.month);
    let fully_reconciled: i64 = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM reconciliation_batches rb WHERE rb.effective_date = ? AND (SELECT COUNT(DISTINCT ri.account_id) FROM reconciliation_items ri JOIN finance_accounts a ON a.id = ri.account_id WHERE ri.batch_id = rb.id AND a.is_real = 1 AND a.is_active = 1) = (SELECT COUNT(*) FROM finance_accounts WHERE is_real = 1 AND is_active = 1))",
    )
    .bind(&reconciliation_date)
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    if fully_reconciled == 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Reconcile every active real account at month end before closing this budget month.",
        ));
    }

    let existing_close: Option<String> =
        sqlx::query_scalar("SELECT operation_id FROM budget_month_closures WHERE month = ?")
            .bind(&payload.month)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
    if existing_close.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "This budget month is already closed.",
        ));
    }

    let reconciliation_id: String = sqlx::query_scalar("SELECT rb.id FROM reconciliation_batches rb JOIN finance_operations operation ON operation.operation_id=rb.operation_id WHERE rb.effective_date=? AND (SELECT COUNT(DISTINCT ri.account_id) FROM reconciliation_items ri JOIN finance_accounts a ON a.id=ri.account_id WHERE ri.batch_id=rb.id AND a.is_real=1 AND a.is_active=1)=(SELECT COUNT(*) FROM finance_accounts WHERE is_real=1 AND is_active=1) ORDER BY operation.rowid DESC LIMIT 1")
        .bind(&reconciliation_date).fetch_one(&mut *transaction).await.map_err(ApiError::internal)?;
    if !crate::budget::reconciliation_current(
        &mut transaction,
        &reconciliation_date,
        &reconciliation_id,
    )
    .await
    .map_err(ApiError::internal)?
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Account activity changed after reconciliation. Reconcile again before closing.",
        ));
    }
    let first = crate::budget::first_month(&mut transaction).await.map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "No canonical opening balance."))?;
    let previous_open = crate::budget::history(&mut transaction, &payload.month).await.map_err(ApiError::internal)?
        .into_iter().any(|month| month.status == "open" && month.month < payload.month);
    if payload.month < first || previous_open {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "Close budget months in chronological order.",
        ));
    }
    crate::budget_ledger::sync(&mut transaction, Some(&payload.operation_id))
        .await
        .map_err(ApiError::internal)?;
    let categories = crate::budget::categories(&mut transaction, &payload.month)
        .await
        .map_err(ApiError::internal)?;
    for category in &categories {
        sqlx::query("INSERT INTO budget_month_snapshots (category_id,month,expense_eur,remaining_eur,reallocation_impact_eur,source_expense_state,source_remaining_state,source_row,source_cells_json) VALUES (?,?,?,?,?,'literal','literal',0,'{}') ON CONFLICT(category_id,month) DO UPDATE SET expense_eur=excluded.expense_eur, remaining_eur=excluded.remaining_eur, reallocation_impact_eur=excluded.reallocation_impact_eur")
            .bind(&category.category_id).bind(&payload.month).bind(&category.expense_eur).bind(&category.remaining_eur).bind(&category.reallocation_impact_eur)
            .execute(&mut *transaction).await.map_err(ApiError::internal)?;
    }
    if categories.is_empty() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "There are no real budget categories to close.",
        ));
    }

    let coverage = crate::budget_ledger::coverage(&mut transaction, &payload.month, &categories)
        .await
        .map_err(ApiError::internal)?;
    let snapshot_json = serde_json::to_string(&categories).map_err(ApiError::internal)?;
    let snapshot_sha256 = hex::encode(Sha256::digest(snapshot_json.as_bytes()));
    let closed_at: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    let response = json!({
        "closed": true,
        "month": payload.month,
        "closedAt": closed_at,
        "snapshotSha256": snapshot_sha256,
        "categoryCount": categories.len(),
    });
    let response_json = serde_json::to_string(&response).map_err(ApiError::internal)?;

    sqlx::query(
        "INSERT INTO budget_month_closures (month, operation_id, closed_at, snapshot_json, snapshot_sha256) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&payload.month)
    .bind(&payload.operation_id)
    .bind(&closed_at)
    .bind(&snapshot_json)
    .bind(&snapshot_sha256)
    .execute(&mut *transaction)
    .await
    .map_err(|error| {
        if let sqlx::Error::Database(database_error) = &error
            && database_error.is_unique_violation()
        {
            ApiError::new(StatusCode::CONFLICT, "This budget month is already closed.")
        } else {
            ApiError::internal(error)
        }
    })?;
    sqlx::query("UPDATE finance_operations SET response_json = ? WHERE operation_id = ?")
        .bind(response_json)
        .bind(&payload.operation_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::internal)?;
    if let Some(coverage) = coverage {
        let frozen = serde_json::to_string(&coverage).map_err(ApiError::internal)?;
        let digest = hex::encode(Sha256::digest(frozen.as_bytes()));
        sqlx::query("INSERT INTO budget_coverage_closures VALUES(?,?,?)")
            .bind(&payload.month)
            .bind(frozen)
            .bind(digest)
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::internal)?;
    }
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(Json(response))
}

async fn run_gateway(
    state: &AppState,
    headers: &HeaderMap,
    function: &str,
    parameters: Value,
) -> Result<Value, ApiError> {
    let access_token = current_access_token(state, headers).await?;
    let url = format!(
        "{}/{}/:run",
        GATEWAY_URL, state.config.apps_script_deployment_id
    );
    let response = state
        .http
        .post(url)
        .bearer_auth(access_token)
        .json(&json!({ "function": function, "parameters": [parameters] }))
        .send()
        .await
        .map_err(|error| {
            ApiError::upstream(format!("Could not reach the spreadsheet gateway: {error}"))
        })?;
    let status = response.status();
    let operation = response.json::<Value>().await.map_err(|error| {
        ApiError::upstream(format!(
            "The spreadsheet gateway returned an unreadable response: {error}"
        ))
    })?;
    if !status.is_success() || operation.get("error").is_some() {
        let message = operation
            .pointer("/error/details/0/errorMessage")
            .and_then(Value::as_str)
            .or_else(|| operation.pointer("/error/message").and_then(Value::as_str))
            .unwrap_or("The spreadsheet gateway could not complete the request.");
        return Err(ApiError::upstream(message.to_string()));
    }
    operation
        .pointer("/response/result")
        .cloned()
        .ok_or_else(|| ApiError::upstream("The spreadsheet gateway did not return a result."))
}

async fn current_access_token(state: &AppState, headers: &HeaderMap) -> Result<String, ApiError> {
    let pocket_session = current_pocket_session(state, headers)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Sign in with Pocket ID to continue.",
            )
        })?;
    let session_id = session_id_from_headers(headers).ok_or_else(|| {
        ApiError::new(StatusCode::UNAUTHORIZED, "Sign in with Google to continue.")
    })?;
    let id_hash = hash_session_id(&session_id);
    let row = sqlx::query(
        "SELECT email, access_token, access_expires_at, expires_at, pocket_session_hash FROM sessions WHERE id_hash = ?",
    )
    .bind(&id_hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::internal)?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Your session expired. Sign in again.",
        )
    })?;
    let email: String = row.try_get("email").map_err(ApiError::internal)?;
    let expires_at: i64 = row.try_get("expires_at").map_err(ApiError::internal)?;
    let linked_pocket_hash: String = row
        .try_get("pocket_session_hash")
        .map_err(ApiError::internal)?;
    if linked_pocket_hash != pocket_session.id_hash
        || expires_at <= now_seconds()
        || !state
            .config
            .allowed_emails
            .iter()
            .any(|allowed| allowed == &email)
    {
        sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
            .bind(&id_hash)
            .execute(&state.pool)
            .await
            .map_err(ApiError::internal)?;
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Your session expired. Sign in again.",
        ));
    }
    let access_expires_at: i64 = row
        .try_get("access_expires_at")
        .map_err(ApiError::internal)?;
    let access_cipher: Vec<u8> = row.try_get("access_token").map_err(ApiError::internal)?;
    if access_expires_at > now_seconds() + 60 {
        return String::from_utf8(decrypt_secret(state, &access_cipher)?)
            .map_err(ApiError::internal);
    }

    let refresh_cipher: Vec<u8> =
        sqlx::query_scalar("SELECT refresh_token FROM google_accounts WHERE email = ?")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "Google authorization expired. Sign in again.",
                )
            })?;
    let refresh_token =
        String::from_utf8(decrypt_secret(state, &refresh_cipher)?).map_err(ApiError::internal)?;
    let token_response = request_google_token(
        state,
        &[
            ("client_id", state.config.google_client_id.as_str()),
            ("client_secret", state.config.google_client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ],
    )
    .await;
    let token_response = match token_response {
        Ok(response) => response,
        Err(error) => {
            if error.status == StatusCode::UNAUTHORIZED {
                sqlx::query("DELETE FROM sessions WHERE email = ?")
                    .bind(&email).execute(&state.pool).await.map_err(ApiError::internal)?;
            }
            return Err(error);
        }
    };
    let encrypted_access = encrypt_secret(state, token_response.access_token.as_bytes())?;
    let updated = sqlx::query("UPDATE sessions SET access_token = ?, access_expires_at = ? WHERE id_hash = ? AND expires_at > ? AND EXISTS (SELECT 1 FROM pocket_sessions p WHERE p.id_hash = sessions.pocket_session_hash AND p.expires_at > ?)")
        .bind(encrypted_access)
        .bind(now_seconds() + token_response.expires_in)
        .bind(id_hash)
        .bind(now_seconds())
        .bind(now_seconds())
        .execute(&state.pool)
        .await
        .map_err(ApiError::internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Your session expired. Sign in again."));
    }
    if let Some(refresh) = token_response.refresh_token {
        let encrypted_refresh = encrypt_secret(state, refresh.as_bytes())?;
        sqlx::query("UPDATE google_accounts SET refresh_token = ?, updated_at = ? WHERE email = ?")
            .bind(encrypted_refresh)
            .bind(now_seconds())
            .bind(&email)
            .execute(&state.pool)
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(token_response.access_token)
}

fn ensure_same_origin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = Url::parse(&(state.config.public_origin.clone() + "/"));
    let actual = headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Url::parse(value).ok());
    let same_origin = match (expected, actual) {
        (Ok(expected), Some(actual)) => {
            actual.origin() == expected.origin()
                && actual.username().is_empty()
                && actual.password().is_none()
                && actual.path() == "/"
                && actual.query().is_none()
                && actual.fragment().is_none()
        }
        _ => false,
    };
    if !same_origin {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This request did not come from the finance workspace.",
        ));
    }
    Ok(())
}

fn ensure_db_request_origin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.contains_key(ORIGIN) {
        return ensure_same_origin(state, headers);
    }
    if bearer_session_id_from_headers(headers).is_some() {
        return Ok(());
    }
    ensure_same_origin(state, headers)
}

fn session_id_from_headers(headers: &HeaderMap) -> Option<String> {
    cookie_id_from_headers(headers, SESSION_COOKIE)
}

fn pocket_session_id_from_headers(headers: &HeaderMap) -> Option<String> {
    if headers.contains_key(AUTHORIZATION) {
        bearer_session_id_from_headers(headers)
    } else {
        cookie_id_from_headers(headers, POCKET_SESSION_COOKIE)
    }
}

fn bearer_session_id_from_headers(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?;
    (token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| token.to_string())
}

fn cookie_id_from_headers(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    let cookies = headers.get(COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|cookie| {
        let (name, value) = cookie.trim().split_once('=')?;
        if name == cookie_name
            && value.len() == 64
            && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            Some(value.to_string())
        } else {
            None
        }
    })
}

async fn current_pocket_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<ActivePocketSession>, ApiError> {
    let Some(cookie_id) = pocket_session_id_from_headers(headers) else {
        return Ok(None);
    };
    let id_hash = hash_session_id(&cookie_id);
    let row =
        sqlx::query("SELECT subject, username, expires_at FROM pocket_sessions WHERE id_hash = ?")
            .bind(&id_hash)
            .fetch_optional(&state.pool)
            .await
            .map_err(ApiError::internal)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let subject: String = row.try_get("subject").map_err(ApiError::internal)?;
    let username: String = row.try_get("username").map_err(ApiError::internal)?;
    let expires_at: i64 = row.try_get("expires_at").map_err(ApiError::internal)?;
    if expires_at <= now_seconds()
        || subject.trim().is_empty()
        || !state.config.pocket_id_allowed_subjects.contains(&subject)
    {
        revoke_pocket_session(state, &id_hash).await?;
        return Ok(None);
    }
    Ok(Some(ActivePocketSession {
        id_hash,
        subject,
        username,
        expires_at,
    }))
}

fn set_session_cookie(response: &mut Response, config: &Config, session_id: &str) {
    let secure = if config.cookie_secure { "; Secure" } else { "" };
    let cookie = format!(
        "{SESSION_COOKIE}={session_id}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECONDS}{secure}"
    );
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append(SET_COOKIE, value);
    }
}

fn set_pocket_session_cookie(response: &mut Response, config: &Config, session_id: &str) {
    let secure = if config.cookie_secure { "; Secure" } else { "" };
    let cookie = format!(
        "{POCKET_SESSION_COOKIE}={session_id}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECONDS}{secure}"
    );
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append(SET_COOKIE, value);
    }
}

fn clear_session_cookie(response: &mut Response, config: &Config) {
    let secure = if config.cookie_secure { "; Secure" } else { "" };
    let cookie = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}");
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append(SET_COOKIE, value);
    }
}

fn clear_pocket_session_cookie(response: &mut Response, config: &Config) {
    let secure = if config.cookie_secure { "; Secure" } else { "" };
    let cookie =
        format!("{POCKET_SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}");
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append(SET_COOKIE, value);
    }
}

fn random_id() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn hash_session_id(session_id: &str) -> String {
    hex::encode(Sha256::digest(session_id.as_bytes()))
}

fn is_uuid_v7(value: &str) -> bool {
    let bytes = value.as_bytes();
    let variant = bytes.get(19).copied().map(|byte| byte.to_ascii_lowercase());
    Uuid::parse_str(value).is_ok()
        && bytes.get(14) == Some(&b'7')
        && matches!(variant, Some(b'8' | b'9' | b'a' | b'b'))
}

fn encrypt_secret(state: &AppState, plaintext: &[u8]) -> Result<Vec<u8>, ApiError> {
    let cipher = XChaCha20Poly1305::new_from_slice(&state.config.encryption_key)
        .map_err(|_| ApiError::internal("invalid token encryption key"))?;
    let mut nonce = [0_u8; 24];
    nonce[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    nonce[16..].copy_from_slice(&Uuid::new_v4().as_bytes()[..8]);
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext)
        .map_err(|_| ApiError::internal("token encryption failed"))?;
    let mut encrypted = nonce.to_vec();
    encrypted.extend(ciphertext);
    Ok(encrypted)
}

fn decrypt_secret(state: &AppState, encrypted: &[u8]) -> Result<Vec<u8>, ApiError> {
    if encrypted.len() < 24 {
        return Err(ApiError::internal(
            "encrypted token is shorter than its nonce",
        ));
    }
    let cipher = XChaCha20Poly1305::new_from_slice(&state.config.encryption_key)
        .map_err(|_| ApiError::internal("invalid token encryption key"))?;
    cipher
        .decrypt(XNonce::from_slice(&encrypted[..24]), &encrypted[24..])
        .map_err(|_| ApiError::internal("token decryption failed"))
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn valid_month(value: &str) -> bool {
    let mut parts = value.split('-');
    let (Some(year), Some(month), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if year.len() != 4 || month.len() != 2 {
        return false;
    }
    match (year.parse::<u32>(), month.parse::<u32>()) {
        (Ok(year), Ok(month)) => year > 0 && (1..=12).contains(&month),
        _ => false,
    }
}

fn last_day_of_month(value: &str) -> Option<u32> {
    if !valid_month(value) {
        return None;
    }
    let year = value.get(..4)?.parse::<u32>().ok()?;
    let month = value.get(5..)?.parse::<u32>().ok()?;
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return None,
    })
}

fn valid_date(value: &str) -> bool {
    let mut parts = value.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<u32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return false;
    };
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return false,
    };
    year > 0 && day > 0 && day <= days_in_month
}

#[cfg(test)]
mod finance_api_drill {
    #[tokio::test]
    async fn browser_deep_links_return_success_without_hiding_missing_assets() -> anyhow::Result<()> {
        let directory = std::env::temp_dir().join(format!("presto-spa-{}", Uuid::new_v4()));
        std::fs::create_dir_all(directory.join("assets"))?;
        let index = "<!doctype html><html>synthetic browser workspace</html>";
        std::fs::write(directory.join("index.html"), index)?;
        std::fs::write(directory.join("assets/app.js"), "console.log('fixture');")?;
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        sqlx::migrate!().run(&pool).await?;
        let router = app(restore_test_state(pool, directory.join("unused.sqlite"), false), &directory)?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build()?;
        for path in ["/", "/budgets", "/history", "/reconciliation", "/transactions?month=2026-10", "/transactions/"] {
            let response = client.get(format!("{origin}{path}")).send().await?;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(response.text().await?, index, "{path}");
        }
        assert_eq!(client.get(format!("{origin}/assets/app.js")).send().await?.status(), StatusCode::OK);
        for path in ["/assets/missing.js", "/api/missing", "/unknown-page"] {
            assert_eq!(client.get(format!("{origin}{path}")).send().await?.status(), StatusCode::NOT_FOUND, "{path}");
        }
        for path in ["/auth/pocket-id", "/api/session"] {
            let response = client.get(format!("{origin}{path}")).send().await?;
            assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
            assert_eq!(response.headers().get("Referrer-Policy").unwrap(), "no-referrer");
            assert_eq!(response.headers().get("X-Frame-Options").unwrap(), "DENY");
        }
        server.abort();
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    use super::*;
    use axum::http::{HeaderValue, header};
    use sqlx::{Row, sqlite::SqliteConnectOptions, sqlite::SqlitePoolOptions};
    use std::{env, path::PathBuf, str::FromStr};

    #[test]
    fn mobile_callbacks_are_allowlisted_and_native_is_default() {
        assert_eq!(
            mobile_client_redirect(None).unwrap(),
            NATIVE_MOBILE_AUTH_REDIRECT_URI
        );
        assert_eq!(
            mobile_client_redirect(Some("native-android")).unwrap(),
            NATIVE_MOBILE_AUTH_REDIRECT_URI
        );
        for client in [
            "https://attacker.example/callback",
            "native",
            "",
            "../callback",
        ] {
            assert_eq!(
                mobile_client_redirect(Some(client)).unwrap_err().status,
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[test]
    fn category_ids_are_normalized_before_budget_rules() {
        let payload = |kind: &str, from: Option<String>, to: Option<String>, treatment: Option<&str>| {
            serde_json::from_value::<DbTransactionWriteRequest>(json!({
                "operationId": uuid7(1), "transactionId": uuid7(2), "kind": kind,
                "date": "2026-09-01", "description": "Test", "amount": "1",
                "fromCategoryId": from, "toCategoryId": to, "incomeTreatment": treatment,
            })).unwrap()
        };
        let padded = format!(" {} ", crate::budget_ledger::UNALLOCATED);
        let mut expense = payload("expense", Some(padded.clone()), None, None);
        assert_eq!(normalize_db_transaction_write(&mut expense).unwrap_err().status, StatusCode::BAD_REQUEST);
        let mut refund = payload("income", None, Some(padded), Some("refund"));
        assert_eq!(normalize_db_transaction_write(&mut refund).unwrap_err().status, StatusCode::BAD_REQUEST);
        let mut income = payload("income", None, Some("  ".into()), Some("unexpected"));
        normalize_db_transaction_write(&mut income).unwrap();
        assert_eq!(income.to_category_id.as_deref(), Some(crate::budget_ledger::UNALLOCATED));
    }

    const SUBJECT: &str = "00000000-0000-4000-8000-000000000001";
    const SESSION_ID: &str = "abababababababababababababababababababababababababababababababab";
    const EUR_ACCOUNT: &str = "01a0f36d-fe19-7352-b728-fd24607c9b20";
    const USD_ACCOUNT: &str = "01a0f36d-fe19-7352-b728-fd25098bb754";
    const GROCERIES: &str = "01a0f36d-fe19-7352-b728-fd2a0f136df1";
    const MONTH: &str = "2026-09";

    fn uuid7(suffix: u16) -> String {
        format!("01995d7a-0000-7000-8000-{suffix:012x}")
    }

    #[tokio::test]
    async fn mobile_login_stores_only_the_selected_allowlisted_callback() -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!().run(&pool).await?;
        let state = restore_test_state(
            pool.clone(),
            PathBuf::from("/tmp/unused-mobile-login.sqlite"),
            false,
        );
        for (client, expected) in [
            (None, NATIVE_MOBILE_AUTH_REDIRECT_URI),
            (Some("native-android"), NATIVE_MOBILE_AUTH_REDIRECT_URI),
        ] {
            let response = begin_mobile_pocket_id_login(
                State(state.clone()),
                Query(MobileLoginQuery {
                    client: client.map(str::to_owned),
                    code_challenge: Some(URL_SAFE_NO_PAD.encode(Sha256::digest(b"a".repeat(64)))),
                }),
            )
            .await
            .unwrap()
            .into_response();
            let location = response.headers().get(header::LOCATION).unwrap().to_str()?;
            let url = Url::parse(location)?;
            let oauth_state = url
                .query_pairs()
                .find(|(name, _)| name == "state")
                .unwrap()
                .1
                .into_owned();
            let stored: String = sqlx::query_scalar(
                "SELECT mobile_redirect_uri FROM pocket_id_states WHERE state = ?",
            )
            .bind(oauth_state)
            .fetch_one(&pool)
            .await?;
            assert_eq!(stored, expected);
            assert_eq!(url.host_str(), Some("pocket-id.test.invalid"));
        }
        let error = begin_mobile_pocket_id_login(
            State(state),
            Query(MobileLoginQuery {
                client: Some("https://attacker.example".into()),
                code_challenge: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pocket_id_states")
            .fetch_one(&pool)
            .await?;
        assert_eq!(count, 2);
        Ok(())
    }

    async fn auth_test_state() -> anyhow::Result<AppState> {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(restore_test_state(pool, PathBuf::from("/tmp/unused-auth.sqlite"), false))
    }

    #[tokio::test]
    async fn pocket_callbacks_require_the_initiating_browser_and_expire() -> anyhow::Result<()> {
        let state = auth_test_state().await?;
        let url = create_pocket_id_authorization(&state, None, None).await?;
        let nonce = Url::parse(&url)?.query_pairs().find(|(key, _)| key == "state").unwrap().1.into_owned();
        let query = || OAuthCallback { state: Some(nonce.clone()), code: Some("test-code".into()), error: None };
        for cookie in [None, Some(format!("{POCKET_OAUTH_COOKIE}={}", "a".repeat(64)))] {
            let mut headers = HeaderMap::new();
            if let Some(cookie) = cookie { headers.insert(COOKIE, HeaderValue::from_str(&cookie)?); }
            assert_eq!(finish_pocket_id_login(&state, query(), &headers).await.err().unwrap().status, StatusCode::BAD_REQUEST);
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pocket_id_states").fetch_one(&state.pool).await?;
        assert_eq!(count, 1, "A callback from another browser must not consume the request");
        sqlx::query("UPDATE pocket_id_states SET expires_at = ?").bind(now_seconds()).execute(&state.pool).await?;
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(&format!("{POCKET_OAUTH_COOKIE}={nonce}"))?);
        assert_eq!(finish_pocket_id_login(&state, query(), &headers).await.err().unwrap().status, StatusCode::BAD_REQUEST);
        let response = pocket_authorization_response(&state, &url)?;
        let cookie = response.headers().get(SET_COOKIE).unwrap().to_str()?;
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax") && cookie.contains("Max-Age=600"));
        Ok(())
    }

    #[tokio::test]
    async fn pocket_login_issuance_rejects_logout_expiry_and_replay() -> anyhow::Result<()> {
        for mobile in [false, true] {
            for cancellation in ["logout", "expiry", "replay"] {
                let state = auth_test_state().await?;
                insert_drill_pocket_session(&state.pool).await?;
                let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest("v".repeat(64).as_bytes()));
                let redirect = mobile.then_some(NATIVE_MOBILE_AUTH_REDIRECT_URI);
                let url = create_pocket_id_authorization(&state, redirect, mobile.then_some(challenge.as_str())).await?;
                let nonce = Url::parse(&url)?.query_pairs().find(|(key, _)| key == "state").unwrap().1.into_owned();
                // Model a verified identity whose provider request completed after cancellation.
                let identity = PocketIdentity { subject: SUBJECT.into(), username: "test".into(),
                    mobile_redirect_uri: redirect.map(str::to_string), mobile_code_challenge: mobile.then_some(challenge) };
                let mut headers = HeaderMap::new();
                headers.insert(COOKIE, HeaderValue::from_str(&format!("{POCKET_SESSION_COOKIE}={SESSION_ID}; {POCKET_OAUTH_COOKIE}={nonce}"))?);
                headers.insert(ORIGIN, HeaderValue::from_static("http://localhost:3000"));
                match cancellation {
                    "logout" => { logout(State(state.clone()), headers.clone()).await?; }
                    "expiry" => { sqlx::query("UPDATE pocket_id_states SET expires_at = ?").bind(now_seconds()).execute(&state.pool).await?; }
                    _ => {
                        let issued = issue_pocket_login(&state, &nonce, &identity, &headers).await?;
                        assert_eq!(issued.len(), 64);
                    }
                }
                let before: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM pocket_sessions), (SELECT count(*) FROM pocket_mobile_handoffs)").fetch_one(&state.pool).await?;
                assert_eq!(issue_pocket_login(&state, &nonce, &identity, &headers).await.unwrap_err().status, StatusCode::UNAUTHORIZED);
                let after: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM pocket_sessions), (SELECT count(*) FROM pocket_mobile_handoffs)").fetch_one(&state.pool).await?;
                assert_eq!(before, after, "Cancelled callbacks must not issue credentials");
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn browser_logout_revokes_its_cookie_even_with_a_bearer_header() -> anyhow::Result<()> {
        for bearer in ["Bearer invalid".to_string(), format!("Bearer {}", "c".repeat(64))] {
            let state = auth_test_state().await?;
            insert_drill_pocket_session(&state.pool).await?;
            let mut headers = HeaderMap::new();
            headers.insert(COOKIE, HeaderValue::from_str(&format!("{POCKET_SESSION_COOKIE}={SESSION_ID}"))?);
            headers.insert(ORIGIN, HeaderValue::from_static("http://localhost:3000"));
            headers.insert(AUTHORIZATION, HeaderValue::from_str(&bearer)?);
            logout(State(state.clone()), headers).await?;
            assert!(current_pocket_session(&state, &authenticated_headers()).await?.is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn google_callback_from_another_session_does_not_consume_state() -> anyhow::Result<()> {
        let state = auth_test_state().await?;
        insert_drill_pocket_session(&state.pool).await?;
        let pocket = current_pocket_session(&state, &authenticated_headers()).await?.unwrap();
        let url = create_google_authorization(&state, false, &pocket.id_hash).await?;
        let nonce = Url::parse(&url)?.query_pairs().find(|(key, _)| key == "state").unwrap().1.into_owned();
        let other = ActivePocketSession { id_hash: "different-session".into(), ..pocket };
        let query = OAuthCallback { state: Some(nonce.clone()), code: Some("unused-code".into()), error: None };
        assert_eq!(finish_google_login(&state, query, &other).await.err().unwrap().status, StatusCode::BAD_REQUEST);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_states WHERE state = ?").bind(nonce).fetch_one(&state.pool).await?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn mobile_handoffs_require_proof_are_single_use_and_expire() -> anyhow::Result<()> {
        let state = auth_test_state().await?;
        let verifier = "v".repeat(64);
        let identity = PocketIdentity { subject: SUBJECT.into(), username: "test".into(),
            mobile_redirect_uri: Some(NATIVE_MOBILE_AUTH_REDIRECT_URI.into()),
            mobile_code_challenge: Some(URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))) };
        let handoff = create_mobile_handoff(&state, &identity).await?;
        let payload = |code: &str, proof: &str| Json(MobileSessionExchangeRequest { handoff_code: code.into(), code_verifier: proof.into() });
        assert_eq!(exchange_mobile_session(State(state.clone()), payload(&handoff, &"x".repeat(64))).await.unwrap_err().status, StatusCode::UNAUTHORIZED);
        let Json(session) = exchange_mobile_session(State(state.clone()), payload(&handoff, &verifier)).await?;
        assert!(session["expiresAt"].as_i64().unwrap() > now_seconds());
        assert_eq!(exchange_mobile_session(State(state.clone()), payload(&handoff, &verifier)).await.unwrap_err().status, StatusCode::UNAUTHORIZED);
        let expired = create_mobile_handoff(&state, &identity).await?;
        sqlx::query("UPDATE pocket_mobile_handoffs SET expires_at = ?").bind(now_seconds()).execute(&state.pool).await?;
        assert_eq!(exchange_mobile_session(State(state.clone()), payload(&expired, &verifier)).await.unwrap_err().status, StatusCode::UNAUTHORIZED);
        let removed = create_mobile_handoff(&state, &identity).await?;
        let mut denied = state.clone();
        denied.config.pocket_id_allowed_subjects.clear();
        assert_eq!(exchange_mobile_session(State(denied), payload(&removed, &verifier)).await.unwrap_err().status, StatusCode::FORBIDDEN);
        Ok(())
    }

    #[tokio::test]
    async fn expired_and_removed_household_sessions_are_rejected() -> anyhow::Result<()> {
        let mut state = auth_test_state().await?;
        insert_drill_pocket_session(&state.pool).await?;
        assert!(current_pocket_session(&state, &authenticated_headers()).await?.is_some());
        state.config.pocket_id_allowed_subjects.clear();
        assert!(current_pocket_session(&state, &authenticated_headers()).await?.is_none());
        state.config.pocket_id_allowed_subjects.insert(SUBJECT.into());
        insert_drill_pocket_session(&state.pool).await?;
        sqlx::query("UPDATE pocket_sessions SET expires_at = ?").bind(now_seconds()).execute(&state.pool).await?;
        assert!(current_pocket_session(&state, &authenticated_headers()).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn bearer_headers_never_fall_back_to_a_valid_cookie() -> anyhow::Result<()> {
        let state = auth_test_state().await?;
        insert_drill_pocket_session(&state.pool).await?;
        for bearer in ["Bearer invalid".into(), format!("Bearer {}", "c".repeat(64))] {
            let mut headers = HeaderMap::new();
            headers.insert(COOKIE, HeaderValue::from_str(&format!("{POCKET_SESSION_COOKIE}={SESSION_ID}"))?);
            headers.insert(AUTHORIZATION, HeaderValue::from_str(&bearer)?);
            assert!(current_pocket_session(&state, &headers).await?.is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn cookie_writes_require_same_origin_including_budget_restore_and_logout() -> anyhow::Result<()> {
        let state = auth_test_state().await?;
        insert_drill_pocket_session(&state.pool).await?;
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(&format!("{POCKET_SESSION_COOKIE}={SESSION_ID}"))?);
        let budget = || Json(MonthlyBudgetRequest { effective_start_month: "2099-01".into(), categories: vec![] });
        assert_eq!(save_monthly_budget(State(state.clone()), headers.clone(), budget()).await.unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(cancel_monthly_budget(State(state.clone()), headers.clone(), budget()).await.unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(restore_db_finance(State(state.clone()), headers.clone(), axum::body::Bytes::new()).await.unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(logout_mobile_session(State(state.clone()), headers.clone()).await.unwrap_err().status, StatusCode::FORBIDDEN);
        headers.insert(ORIGIN, HeaderValue::from_static("https://attacker.example"));
        assert!(ensure_db_request_origin(&state, &headers).is_err());
        assert!(ensure_db_request_origin(&state, &authenticated_headers()).is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn logout_revokes_linked_google_sessions_and_pending_oauth_and_is_idempotent() -> anyhow::Result<()> {
        let state = auth_test_state().await?;
        insert_drill_pocket_session(&state.pool).await?;
        let hash = hash_session_id(SESSION_ID);
        sqlx::query("INSERT INTO google_accounts VALUES ('test@example.invalid', X'00', 0)").execute(&state.pool).await?;
        sqlx::query("INSERT INTO sessions VALUES ('google-session', 'test@example.invalid', X'00', 0, ?, 0, ?)")
            .bind(now_seconds()+3600).bind(&hash).execute(&state.pool).await?;
        create_google_authorization(&state, false, &hash).await?;
        let _ = logout_mobile_session(State(state.clone()), authenticated_headers()).await?;
        let _ = logout_mobile_session(State(state.clone()), authenticated_headers()).await?;
        for query in ["SELECT count(*) FROM sessions", "SELECT count(*) FROM oauth_states", "SELECT count(*) FROM pocket_sessions"] {
            let count: i64 = sqlx::query_scalar(query).fetch_one(&state.pool).await?;
            assert_eq!(count, 0, "{query}");
        }
        assert!(current_pocket_session(&state, &authenticated_headers()).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn browser_logout_cancels_pending_login_and_clears_every_cookie() -> anyhow::Result<()> {
        let state = auth_test_state().await?;
        insert_drill_pocket_session(&state.pool).await?;
        let url = create_pocket_id_authorization(&state, None, None).await?;
        let nonce = Url::parse(&url)?.query_pairs().find(|(key, _)| key == "state").unwrap().1.into_owned();
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static("http://localhost:3000"));
        headers.insert(COOKIE, HeaderValue::from_str(&format!("{POCKET_SESSION_COOKIE}={SESSION_ID}; {POCKET_OAUTH_COOKIE}={nonce}"))?);
        let response = logout(State(state.clone()), headers).await?;
        assert_eq!(response.headers().get_all(SET_COOKIE).iter().count(), 3);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pocket_id_states").fetch_one(&state.pool).await?;
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn google_outages_do_not_invalidate_authentication_and_token_lifetimes_are_bounded() {
        for status in [StatusCode::SERVICE_UNAVAILABLE, StatusCode::TOO_MANY_REQUESTS, StatusCode::BAD_REQUEST] {
            assert_eq!(parse_google_token_response(status, json!({"error":"temporarily_unavailable"})).unwrap_err().status, StatusCode::BAD_GATEWAY);
        }
        assert_eq!(parse_google_token_response(StatusCode::BAD_REQUEST, json!({"error":"invalid_grant"})).unwrap_err().status, StatusCode::UNAUTHORIZED);
        assert_eq!(parse_google_token_response(StatusCode::OK, json!({"access_token":"test", "expires_in":i64::MAX})).unwrap().expires_in, 86_400);
        assert_eq!(parse_google_token_response(StatusCode::OK, json!({"access_token":""})).unwrap_err().status, StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn google_callbacks_cannot_create_sessions_after_pocket_logout_or_expiry() -> anyhow::Result<()> {
        let state = auth_test_state().await?;
        insert_drill_pocket_session(&state.pool).await?;
        sqlx::query("INSERT INTO google_accounts VALUES ('test@example.invalid', X'00', 0)").execute(&state.pool).await?;
        let pocket = current_pocket_session(&state, &authenticated_headers()).await?.unwrap();
        let id = insert_google_session(&state, &pocket, "test@example.invalid", "test-access", 3600).await?;
        assert!(!id.is_empty());
        revoke_pocket_session(&state, &pocket.id_hash).await?;
        assert_eq!(insert_google_session(&state, &pocket, "test@example.invalid", "test-access", 3600).await.unwrap_err().status, StatusCode::UNAUTHORIZED);
        insert_drill_pocket_session(&state.pool).await?;
        sqlx::query("UPDATE pocket_sessions SET expires_at = ?").bind(now_seconds()).execute(&state.pool).await?;
        assert_eq!(insert_google_session(&state, &pocket, "test@example.invalid", "test-access", 3600).await.unwrap_err().status, StatusCode::UNAUTHORIZED);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions").fetch_one(&state.pool).await?;
        assert_eq!(count, 0);
        Ok(())
    }

    fn authenticated_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {SESSION_ID}"))
                .expect("static bearer token is a valid header"),
        );
        headers
    }

    #[tokio::test]
    async fn push_registration_requires_bearer_auth_and_logout_removes_devices() -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        sqlx::migrate!().run(&pool).await?;
        insert_drill_pocket_session(&pool).await?;
        let state = restore_test_state(pool.clone(), PathBuf::from("/tmp/unused-push-test.sqlite"), false);
        let installation = Uuid::new_v4().to_string();
        let payload = || PushDeviceRequest { installation_id: installation.clone(), token: "synthetic-token-for-notification-test".into() };
        let mut unsigned_headers = HeaderMap::new();
        unsigned_headers.insert(ORIGIN, HeaderValue::from_static("http://localhost:3000"));
        assert_eq!(register_push_device(State(state.clone()), unsigned_headers, Json(payload())).await.unwrap_err().status, StatusCode::UNAUTHORIZED);
        let mut foreign = authenticated_headers();
        foreign.insert(ORIGIN, HeaderValue::from_static("https://foreign.invalid"));
        assert_eq!(register_push_device(State(state.clone()), foreign, Json(payload())).await.unwrap_err().status, StatusCode::FORBIDDEN);
        let _ = register_push_device(State(state.clone()), authenticated_headers(), Json(payload())).await?;
        let subject: String = sqlx::query_scalar("SELECT subject FROM push_devices").fetch_one(&pool).await?;
        assert_eq!(subject, SUBJECT);
        let _ = logout_mobile_session(State(state), authenticated_headers()).await?;
        let devices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM push_devices").fetch_one(&pool).await?;
        assert_eq!(devices, 0);
        Ok(())
    }

    fn config(database_path: PathBuf) -> crate::Config {
        crate::Config {
            google_client_id: "unused-test-client".into(),
            google_client_secret: "unused-test-secret".into(),
            google_redirect_uri: "http://localhost:3000/auth/google/callback".into(),
            pocket_id_issuer: "https://pocket-id.test.invalid".into(),
            pocket_id_client_id: "unused-test-client".into(),
            pocket_id_redirect_uri: "http://localhost:3000/auth/pocket-id/callback".into(),
            pocket_id_allowed_subjects: [SUBJECT.to_string()].into_iter().collect(),
            public_origin: "http://localhost:3000".into(),
            apps_script_deployment_id: "unused-test-deployment-id".into(),
            allowed_emails: vec!["unused@example.invalid".into()],
            finance_db_read_enabled: true,
            finance_db_restore_enabled: false,
            finance_db_close_enabled: true,
            finance_db_category_corrections_enabled: true,
            finance_db_transaction_writes_enabled: true,
            finance_db_reconciliation_writes_enabled: true,
            encryption_key: [0; 32],
            cookie_secure: false,
            bind_addr: "127.0.0.1:0".parse().expect("valid test address"),
            database_path,
            static_dir: PathBuf::from("/tmp"),
        }
    }

    fn restore_test_state(
        pool: SqlitePool,
        database_path: PathBuf,
        restore_enabled: bool,
    ) -> AppState {
        let mut config = config(database_path);
        config.finance_db_restore_enabled = restore_enabled;
        config.finance_db_close_enabled = false;
        config.finance_db_category_corrections_enabled = false;
        config.finance_db_transaction_writes_enabled = false;
        config.finance_db_reconciliation_writes_enabled = false;
        AppState {
            config,
            pool,
            http: reqwest::Client::new(),
            pocket_oidc: PocketOidc {
                authorization_endpoint: "https://pocket-id.test.invalid/authorize".into(),
                token_endpoint: "https://pocket-id.test.invalid/token".into(),
                userinfo_endpoint: "https://pocket-id.test.invalid/userinfo".into(),
            },
        }
    }

    async fn open_empty_drill_database(path: &Path) -> anyhow::Result<SqlitePool> {
        assert!(!path.exists(), "drill restore target must be new");
        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(pool)
    }

    async fn insert_drill_pocket_session(pool: &SqlitePool) -> anyhow::Result<()> {
        let now = now_seconds();
        sqlx::query("INSERT INTO pocket_sessions (id_hash, subject, username, expires_at, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(hash_session_id(SESSION_ID))
            .bind(SUBJECT)
            .bind("disposable-drill")
            .bind(now + 3600)
            .bind(now)
            .execute(pool)
            .await?;
        Ok(())
    }

    fn restore_headers(sha256: &str) -> HeaderMap {
        let mut headers = authenticated_headers();
        headers.insert(
            header::HeaderName::from_static("x-finance-snapshot-sha256"),
            HeaderValue::from_str(sha256).expect("snapshot digest header is valid"),
        );
        headers
    }

    async fn assert_all_finance_tables_empty(pool: &SqlitePool) -> anyhow::Result<()> {
        for table in FINANCE_BACKUP_TABLES {
            let query = AssertSqlSafe(format!("SELECT COUNT(*) FROM \"{table}\""));
            let count: i64 = sqlx::query_scalar(query).fetch_one(pool).await?;
            assert_eq!(count, 0, "failed restore modified {table}");
        }
        Ok(())
    }

    async fn account_revisions(pool: &SqlitePool) -> anyhow::Result<Vec<(String, String, i64)>> {
        let rows = sqlx::query("SELECT id, current_native_balance, ledger_revision FROM finance_accounts WHERE is_real = 1 AND is_active = 1 ORDER BY source_row, id")
            .fetch_all(pool)
            .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    row.try_get("id")?,
                    row.try_get("current_native_balance")?,
                    row.try_get("ledger_revision")?,
                ))
            })
            .collect()
    }

    fn assert_status(error: ApiError, expected: StatusCode) {
        assert_eq!(error.status, expected, "{}", error.message);
    }

    fn api_ok(result: Result<Json<Value>, ApiError>) -> anyhow::Result<Value> {
        result
            .map(|Json(value)| value)
            .map_err(|error| anyhow::anyhow!("{}: {}", error.status, error.message))
    }

    #[tokio::test]
    async fn transaction_listing_requires_household_auth_and_valid_filters() -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!().run(&pool).await?;
        insert_drill_pocket_session(&pool).await?;
        let mut state = restore_test_state(
            pool,
            PathBuf::from("/tmp/unused-transaction-list.sqlite"),
            false,
        );
        state.config.finance_db_read_enabled = true;
        let query = || TransactionListQuery {
            month: Some("2000-01".into()),
            search: String::new(),
            kind: None,
            category: None,
            account: None,
            page: Some(1),
            page_size: Some(50),
            sort: None,
            direction: None,
        };
        let error = list_db_transactions(State(state.clone()), HeaderMap::new(), Query(query()))
            .await
            .unwrap_err();
        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
        let mut invalid = query();
        invalid.month = Some("2000-13".into());
        assert_eq!(
            list_db_transactions(
                State(state.clone()),
                authenticated_headers(),
                Query(invalid)
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::BAD_REQUEST
        );
        let mut invalid = query();
        invalid.page_size = Some(101);
        assert_eq!(
            list_db_transactions(
                State(state.clone()),
                authenticated_headers(),
                Query(invalid)
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::BAD_REQUEST
        );
        let result = api_ok(
            list_db_transactions(
                State(state.clone()),
                authenticated_headers(),
                Query(query()),
            )
            .await,
        )?;
        assert_eq!(result["total"], 0);
        state.config.finance_db_read_enabled = false;
        assert_eq!(
            list_db_transactions(State(state), authenticated_headers(), Query(query()))
                .await
                .unwrap_err()
                .status,
            StatusCode::SERVICE_UNAVAILABLE
        );
        Ok(())
    }

    #[tokio::test]
    async fn monthly_budget_changes_preserve_history_and_adjust_remaining() -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!().run(&pool).await?;
        insert_drill_pocket_session(&pool).await?;
        let mut state = restore_test_state(
            pool.clone(),
            PathBuf::from("/tmp/unused-budget-test.sqlite"),
            false,
        );
        state.config.finance_db_transaction_writes_enabled = true;
        let month = crate::budget::today()[..7].to_owned();
        sqlx::query("INSERT INTO finance_categories (id, name, is_real, source_row, source_row_json) VALUES ('food', 'Food', 1, 1, '{}')").execute(&pool).await?;
        sqlx::query(
            "INSERT INTO category_budget_periods VALUES ('food', '2000-01', '100', 1, 'null')",
        )
        .execute(&pool)
        .await?;
        sqlx::query("INSERT INTO budget_month_snapshots (category_id, month, expense_eur, remaining_eur, reallocation_impact_eur, source_expense_state, source_remaining_state, source_row, source_cells_json) VALUES ('food', ?, '30', '70', '0', 'literal', 'literal', 1, '{}')").bind(&month).execute(&pool).await?;
        let next: String =
            sqlx::query_scalar("SELECT strftime('%Y-%m', date(? || '-01', '+1 month'))")
                .bind(&month)
                .fetch_one(&pool)
                .await?;
        sqlx::query("INSERT INTO budget_month_snapshots (category_id, month, expense_eur, remaining_eur, reallocation_impact_eur, source_expense_state, source_remaining_state, source_row, source_cells_json) VALUES ('food', ?, '0', '170', '0', 'literal', 'literal', 1, '{}')").bind(&next).execute(&pool).await?;
        sqlx::query("INSERT INTO finance_import_runs VALUES ('budget-test','test','digest',?,'{}','2000-01-01')")
            .bind(json!({"currentCanonicalMonth":month}).to_string()).execute(&pool).await?;
        sqlx::query(
            "INSERT INTO budget_opening_state VALUES ('food',?,'0','test','2000-01-01','{}')",
        )
        .bind(&month)
        .execute(&pool)
        .await?;
        sqlx::query("INSERT INTO finance_transactions (id,kind,effective_date,description,amount_decimal,input_currency,from_category_id,currency_is_override,to_amount_is_override,source_workbook_id,source_sheet,source_row,original_row_fingerprint,source_row_json) VALUES ('spend','expense',?,'food','30','EUR','food',0,0,'test','test',1,'test','{}')")
            .bind(format!("{month}-01")).execute(&pool).await?;
        sqlx::query("INSERT INTO transaction_valuations (transaction_id,side,amount_eur,effective_date,valuation_source,source_cells_json) VALUES ('spend','from','30',?,'test','{}')").bind(format!("{month}-01")).execute(&pool).await?;
        let payload = |amount: &str| MonthlyBudgetRequest {
            effective_start_month: month.clone(),
            categories: vec![MonthlyBudgetAmount {
                name: "Food".into(),
                amount: amount.into(),
            }],
        };
        assert_status(
            save_monthly_budget(State(state.clone()), HeaderMap::from_iter([(ORIGIN, HeaderValue::from_static("http://localhost:3000"))]), Json(payload("150")))
                .await
                .unwrap_err(),
            StatusCode::UNAUTHORIZED,
        );
        assert_status(
            save_monthly_budget(
                State(state.clone()),
                authenticated_headers(),
                Json(payload("-1")),
            )
            .await
            .unwrap_err(),
            StatusCode::BAD_REQUEST,
        );
        api_ok(
            save_monthly_budget(
                State(state.clone()),
                authenticated_headers(),
                Json(payload("150")),
            )
            .await,
        )?;
        let remaining: String =
            sqlx::query_scalar("SELECT remaining_eur FROM budget_month_snapshots WHERE month = ?")
                .bind(&month)
                .fetch_one(&pool)
                .await?;
        assert_eq!(remaining, "70", "Imported open snapshot is provenance, not a mutable balance.");
        {
            let mut c = pool.acquire().await?;
            let calculated = crate::budget::categories(&mut c, &month).await?;
            assert_eq!(calculated[0].remaining_eur.as_deref(), Some("120"));
        }
        let historical: String = sqlx::query_scalar("SELECT base_monthly_eur FROM category_budget_periods WHERE effective_start_month = '2000-01'").fetch_one(&pool).await?;
        assert_eq!(historical, "100");
        let future: String =
            sqlx::query_scalar("SELECT remaining_eur FROM budget_month_snapshots WHERE month = ?")
                .bind(&next)
                .fetch_one(&pool)
                .await?;
        assert_eq!(future, "170", "Future stored provenance remains unchanged.");
        {
            let mut c = pool.acquire().await?;
            let calculated = crate::budget::history_at(&mut c, &next, &next).await?;
            assert_eq!(calculated.last().unwrap().categories[0].remaining_eur.as_deref(), Some("270"));
        }
        // Scheduling a later change must survive subsequent current-month edits.
        let mut schedule = payload("175");
        schedule.effective_start_month = next.clone();
        api_ok(
            save_monthly_budget(
                State(state.clone()),
                authenticated_headers(),
                Json(schedule),
            )
            .await,
        )?;
        let mut invalid = payload("200");
        invalid.categories.push(MonthlyBudgetAmount {
            name: "Missing".into(),
            amount: "10".into(),
        });
        assert_status(
            save_monthly_budget(State(state.clone()), authenticated_headers(), Json(invalid))
                .await
                .unwrap_err(),
            StatusCode::BAD_REQUEST,
        );

        api_ok(
            save_monthly_budget(State(state.clone()), authenticated_headers(), Json(payload("150"))).await,
        )?;
        let remaining: String =
            sqlx::query_scalar("SELECT remaining_eur FROM budget_month_snapshots WHERE month = ?")
                .bind(&month)
                .fetch_one(&pool)
                .await?;
        assert_eq!(remaining, "70", "Imported open snapshot is provenance, not a mutable balance.");
        {
            let mut c = pool.acquire().await?;
            let calculated = crate::budget::categories(&mut c, &month).await?;
            assert_eq!(calculated[0].remaining_eur.as_deref(), Some("120"));
        }
        assert_eq!(sqlx::query_scalar::<_,String>("SELECT base_monthly_eur FROM category_budget_periods WHERE effective_start_month=?").bind(&next).fetch_one(&pool).await?, "175");
        assert_status(cancel_monthly_budget(State(state.clone()), HeaderMap::from_iter([(ORIGIN, HeaderValue::from_static("http://localhost:3000"))]), Json(payload("0"))).await.unwrap_err(), StatusCode::UNAUTHORIZED);
        assert_status(cancel_monthly_budget(State(state.clone()), authenticated_headers(), Json(payload("0"))).await.unwrap_err(), StatusCode::BAD_REQUEST);
        let mut cancel = payload("0"); cancel.effective_start_month = next.clone();
        api_ok(cancel_monthly_budget(State(state.clone()), authenticated_headers(), Json(cancel)).await)?;
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM category_budget_periods WHERE effective_start_month=?").bind(&next).fetch_one(&pool).await?, 0);
        assert_eq!(sqlx::query_scalar::<_,String>("SELECT base_monthly_eur FROM category_budget_periods WHERE category_id='food' ORDER BY effective_start_month DESC LIMIT 1").fetch_one(&pool).await?, "150");
        assert_status(transaction_history(State(state.clone()), HeaderMap::new(), Query(std::collections::HashMap::from([("id".into(),"spend".into())]))).await.unwrap_err(), StatusCode::UNAUTHORIZED);
        sqlx::query("INSERT INTO finance_operations (operation_id,actor_subject,payload_fingerprint,affected_transaction_ids_json,response_json,created_at) VALUES ('history-test','partner','test','[]','{}','2026-10-01')").execute(&pool).await?;
        sqlx::query("INSERT INTO finance_transaction_revisions (transaction_id,revision,operation_id,snapshot_json,recorded_at) VALUES ('spend',1,'history-test',?, '2026-10-01'),('spend',2,'history-test',?, '2026-10-02')")
            .bind(json!({"recordedByName":"Partner","transaction":{"description":"Original purchase","amount":"30","fromCategoryId":"food"}}).to_string())
            .bind(json!({"recordedByName":"Partner","request":{"description":"Corrected purchase","amount":"35","fromCategoryId":"food"}}).to_string()).execute(&pool).await?;
        let create = |name: &str, amount: &str| Json(CreateBudgetCategoryRequest { name: name.into(), monthly_amount: amount.into() });
        assert_status(create_budget_category(State(state.clone()), HeaderMap::new(), create("Home", "25")).await.unwrap_err(), StatusCode::FORBIDDEN);
        assert_status(create_budget_category(State(state.clone()), authenticated_headers(), create("Home", "-1")).await.unwrap_err(), StatusCode::BAD_REQUEST);
        assert_status(create_budget_category(State(state.clone()), authenticated_headers(), create(" food ", "25")).await.unwrap_err(), StatusCode::CONFLICT);
        let created = api_ok(create_budget_category(State(state.clone()), authenticated_headers(), create(" Home ", "25")).await)?;
        assert_eq!(created["name"], "Home");
        assert_status(create_budget_category(State(state.clone()), authenticated_headers(), create("home", "25")).await.unwrap_err(), StatusCode::CONFLICT);
        {
            let mut c = pool.acquire().await?;
            let categories = crate::budget::categories(&mut c, &month).await?;
            let home = categories.iter().find(|c| c.name == "Home").unwrap();
            assert_eq!(home.remaining_eur.as_deref(), Some("25"));
            assert_eq!(home.carryover_eur.as_deref(), Some("0"));
            assert_eq!(categories.iter().find(|c| c.name == "Food").unwrap().remaining_eur.as_deref(), Some("120"));
        }
        state.config.finance_db_transaction_writes_enabled = false;
        assert_status(create_budget_category(State(state.clone()), authenticated_headers(), create("Other", "0")).await.unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
        let history = api_ok(transaction_history(State(state), authenticated_headers(), Query(std::collections::HashMap::from([("id".into(),"spend".into())]))).await)?;
        assert_eq!(history["history"].as_array().unwrap().len(), 2);
        assert_eq!(history["history"][0]["by"], "Partner");
        assert_eq!(history["history"][1]["revision"], 2);
        assert_eq!(history["history"][1]["details"]["request"]["description"], "Corrected purchase");
        assert_eq!(history["history"][1]["details"]["request"]["fromCategoryName"], "Food");

        Ok(())
    }

    #[tokio::test]
    async fn expected_income_settings_require_auth_and_persist_by_effective_month()
    -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!().run(&pool).await?;
        insert_drill_pocket_session(&pool).await?;
        let mut state = restore_test_state(
            pool.clone(),
            PathBuf::from("/tmp/unused-income-test.sqlite"),
            false,
        );
        state.config.finance_db_transaction_writes_enabled = true;
        let payload = || ExpectedIncomeRequest {
            effective_start_month: "2026-10".into(),
            amount: "10000".into(),
            currency: "USD".into(),
            arrival_day: 7,
        };
        let mut signed_out_headers = authenticated_headers();
        signed_out_headers.remove(COOKIE);
        signed_out_headers.remove(AUTHORIZATION);
        signed_out_headers.insert(ORIGIN, HeaderValue::from_str(&state.config.public_origin)?);
        assert_status(
            save_expected_income(State(state.clone()), signed_out_headers, Json(payload()))
                .await
                .unwrap_err(),
            StatusCode::UNAUTHORIZED,
        );
        let mut invalid = payload();
        invalid.amount = "-1".into();
        assert_status(
            save_expected_income(State(state.clone()), authenticated_headers(), Json(invalid))
                .await
                .unwrap_err(),
            StatusCode::BAD_REQUEST,
        );
        let mut invalid = payload();
        invalid.arrival_day = 0;
        assert_status(
            save_expected_income(State(state.clone()), authenticated_headers(), Json(invalid))
                .await
                .unwrap_err(),
            StatusCode::BAD_REQUEST,
        );
        let mut invalid = payload();
        invalid.currency = "GBP".into();
        assert_status(
            save_expected_income(State(state.clone()), authenticated_headers(), Json(invalid))
                .await
                .unwrap_err(),
            StatusCode::BAD_REQUEST,
        );
        api_ok(
            save_expected_income(
                State(state.clone()),
                authenticated_headers(),
                Json(payload()),
            )
            .await,
        )?;
        let row = sqlx::query("SELECT amount, currency, arrival_day FROM budget_income_expectations WHERE effective_start_month <= '2026-11' ORDER BY effective_start_month DESC LIMIT 1").fetch_one(&pool).await?;
        assert_eq!(row.try_get::<String, _>("amount")?, "10000");
        assert_eq!(row.try_get::<String, _>("currency")?, "USD");
        assert_eq!(row.try_get::<i64, _>("arrival_day")?, 7);
        let mut update = payload();
        update.amount = "11000".into();
        api_ok(
            save_expected_income(State(state.clone()), authenticated_headers(), Json(update)).await,
        )?;
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT amount FROM budget_income_expectations")
                .fetch_one(&pool)
                .await?,
            "11000"
        );
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM budget_income_expectations WHERE effective_start_month <= '2026-09'").fetch_one(&pool).await?, 0);
        state.config.finance_db_transaction_writes_enabled = false;
        assert_status(
            save_expected_income(State(state), authenticated_headers(), Json(payload()))
                .await
                .unwrap_err(),
            StatusCode::SERVICE_UNAVAILABLE,
        );
        Ok(())
    }

    #[tokio::test]
    async fn current_balance_reconciliation_reconstructs_previous_month_and_preserves_future_activity()
    -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!().run(&pool).await?;
        insert_drill_pocket_session(&pool).await?;
        for (id, name, real, balance, row) in [
            ("bank", "Bank", 1, "1010", 1),
            ("other", "Other", 1, "30", 2),
            ("system", "Reconciliation", 0, "0", 3),
        ] {
            sqlx::query("INSERT INTO finance_accounts (id, source_name, currency, is_real, is_active, current_native_balance, source_row, source_row_json) VALUES (?, ?, 'EUR', ?, 1, ?, ?, '{}')")
                .bind(id).bind(name).bind(real).bind(balance).bind(row).execute(&pool).await?;
        }
        sqlx::query("INSERT INTO finance_categories (id, name, is_real, source_row, source_row_json) VALUES ('recon', 'Reconciliation', 0, 1, '{}')").execute(&pool).await?;
        let today: String = sqlx::query_scalar("SELECT date('now')")
            .fetch_one(&pool)
            .await?;
        let tomorrow: String = sqlx::query_scalar("SELECT date('now', '+1 day')")
            .fetch_one(&pool)
            .await?;
        let closing: String = sqlx::query_scalar("SELECT date('now', 'start of month', '-1 day')")
            .fetch_one(&pool)
            .await?;
        for (index, kind, amount, from, to, date) in [
            (1, "income", "100", "system", "bank", &today),
            (2, "expense", "40", "bank", "system", &today),
            (3, "transfer", "30", "bank", "other", &today),
            (4, "expense", "20", "bank", "system", &tomorrow),
        ] {
            let id = uuid7(0x200 + index);
            sqlx::query("INSERT INTO finance_transactions (id, kind, effective_date, description, amount_decimal, input_currency, from_account_id, to_account_id, currency_is_override, to_amount_is_override, source_workbook_id, source_sheet, source_row, original_row_fingerprint, source_row_json) VALUES (?, ?, ?, 'test activity', ?, 'EUR', ?, ?, 0, 0, 'test', 'Transactions', ?, 'test', '{}')")
                .bind(&id).bind(kind).bind(date).bind(amount).bind(from).bind(to).bind(i64::from(index)).execute(&pool).await?;
            for side in ["from", "to"] {
                sqlx::query("INSERT INTO transaction_valuations (transaction_id, side, account_currency, amount_native, amount_eur, effective_date, valuation_source, source_cells_json) VALUES (?, ?, 'EUR', ?, ?, ?, 'test', '{}')")
                    .bind(&id).bind(side).bind(amount).bind(amount).bind(date).execute(&pool).await?;
            }
        }
        // A same-day transaction after the observed instant belongs outside
        // the current statement balance, while date-only legacy rows remain included.
        sqlx::query("UPDATE finance_transactions SET occurred_at = ? WHERE kind = 'transfer'")
            .bind(format!("{today}T13:00:00.000Z"))
            .execute(&pool)
            .await?;
        let mut connection = pool.acquire().await?;
        let timed_activity = reconciliation_activity(
            &mut connection,
            "bank",
            &closing,
            &today,
            Some(&format!("{today}T12:00:00.000Z")),
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
        assert_eq!(timed_activity.through_observation, "60");
        assert_eq!(timed_activity.after_observation, "-50");
        assert_eq!(timed_activity.transactions.len(), 2);
        drop(connection);
        sqlx::query("UPDATE finance_transactions SET occurred_at = NULL WHERE kind = 'transfer'")
            .execute(&pool)
            .await?;
        let mut state = restore_test_state(
            pool.clone(),
            PathBuf::from("/tmp/unused-reconciliation-test.sqlite"),
            false,
        );
        state.config.finance_db_reconciliation_writes_enabled = true;
        let preview = api_ok(
            get_reconciliation_preview(
                State(state.clone()),
                authenticated_headers(),
                Query(ReconciliationPreviewQuery {
                    observed_date: today.clone(),
                    closing_month: None,
                }),
            )
            .await,
        )?;
        assert_eq!(preview["closingDate"], closing);
        assert_eq!(preview["accounts"][0]["balanceNative"], "1030");
        assert_eq!(preview["accounts"][0]["closingBalanceNative"], "1000");
        assert_eq!(preview["accounts"][0]["activityNative"], "30");
        assert_eq!(
            preview["accounts"][0]["transactions"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(preview["totalEur"], "1060"); // Transfer is neutral across accounts.
        let request = DbReconciliationRequest {
            operation_id: uuid7(0x210),
            date: closing.clone(),
            observed_at: Some(preview["observedAt"].as_str().unwrap().into()),
            observed_date: Some(today.clone()),
            accounts: vec![DbReconciliationAccount {
                account_id: "bank".into(),
                actual_balance: "1015".into(),
                transaction_id: uuid7(0x211),
                expected_revision: 0,
            }],
        };
        let saved = api_ok(
            record_db_reconciliation(
                State(state.clone()),
                authenticated_headers(),
                Json(request.clone()),
            )
            .await,
        )?;
        let retried = api_ok(
            record_db_reconciliation(
                State(state.clone()),
                authenticated_headers(),
                Json(request.clone()),
            )
            .await,
        )?;
        assert_eq!(saved, retried);
        let item = sqlx::query("SELECT observed_native_balance, actual_native_balance, post_month_activity_native, included_transactions_json, variance_native FROM reconciliation_items").fetch_one(&pool).await?;
        assert_eq!(
            item.try_get::<String, _>("observed_native_balance")?,
            "1015"
        );
        assert_eq!(item.try_get::<String, _>("actual_native_balance")?, "985");
        assert_eq!(
            item.try_get::<String, _>("post_month_activity_native")?,
            "30"
        );
        assert_eq!(item.try_get::<String, _>("variance_native")?, "15");
        assert_eq!(
            serde_json::from_str::<Value>(
                &item.try_get::<String, _>("included_transactions_json")?
            )?
            .as_array()
            .unwrap()
            .len(),
            3
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT current_native_balance FROM finance_accounts WHERE id = 'bank'"
            )
            .fetch_one(&pool)
            .await?,
            "995"
        ); // 1015 observed less future 20.
        let replacement_preview = api_ok(
            get_reconciliation_preview(
                State(state.clone()),
                authenticated_headers(),
                Query(ReconciliationPreviewQuery {
                    observed_date: today.clone(),
                    closing_month: None,
                }),
            )
            .await,
        )?;
        assert_eq!(
            replacement_preview["accounts"][0]["existingAdjustmentNative"],
            "15"
        );
        assert_eq!(replacement_preview["closingTotalEur"], "1000");
        let mut stale = request.clone();
        stale.operation_id = uuid7(0x212);
        assert_status(
            record_db_reconciliation(State(state.clone()), authenticated_headers(), Json(stale))
                .await
                .unwrap_err(),
            StatusCode::CONFLICT,
        );
        let mut replacement = request.clone();
        replacement.operation_id = uuid7(0x213);
        replacement.accounts[0].expected_revision = 1;
        replacement.accounts[0].actual_balance = "1000".into();
        api_ok(
            record_db_reconciliation(
                State(state.clone()),
                authenticated_headers(),
                Json(replacement),
            )
            .await,
        )?;
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT amount_decimal FROM finance_transactions WHERE kind = 'reconciliation'"
            )
            .fetch_one(&pool)
            .await?,
            "30"
        );
        let mut wrong_month = request;
        wrong_month.operation_id = uuid7(0x214);
        wrong_month.date = today;
        assert_status(
            record_db_reconciliation(State(state), authenticated_headers(), Json(wrong_month))
                .await
                .unwrap_err(),
            StatusCode::CONFLICT,
        );
        Ok(())
    }

    #[tokio::test]
    async fn transaction_timestamps_default_to_now_and_respect_date_only_and_cutoffs()
    -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!().run(&pool).await?;
        let mut connection = pool.acquire().await?;
        let today = crate::budget::today();
        let local = chrono::NaiveDate::parse_from_str(&today,"%Y-%m-%d")?;
        let yesterday = local.pred_opt().unwrap().to_string();
        let tomorrow = local.succ_opt().unwrap().to_string();
        let resolved = resolve_transaction_timestamp(&mut connection, &today, None, false)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?
            .unwrap();
        assert!(valid_transaction_timestamp(&resolved));
        assert!(
            resolve_transaction_timestamp(&mut connection, &today, None, true)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?
                .is_none()
        );
        assert!(
            resolve_transaction_timestamp(&mut connection, &yesterday, None, false)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?
                .is_none()
        );
        assert!(
            resolve_transaction_timestamp(&mut connection, &tomorrow, None, false)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?
                .is_none()
        );
        let captured = format!("{yesterday}T12:00:00.000Z");
        assert_eq!(
            resolve_transaction_timestamp(&mut connection, &yesterday, Some(&captured), false)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?,
            Some(captured.clone())
        );
        // Local posting dates can fall on the next UTC calendar date.
        assert!(
            resolve_transaction_timestamp(&mut connection, &today, Some(&captured), false)
                .await
                .is_ok()
        );
        assert!(
            resolve_transaction_timestamp(&mut connection, &tomorrow, Some(&captured), false)
                .await
                .is_err()
        );
        assert!(
            resolve_transaction_timestamp(
                &mut connection,
                &tomorrow,
                Some(&format!("{tomorrow}T12:00:00.000Z")),
                false
            )
            .await
            .is_err()
        );
        for invalid in [
            "2026-02-30T12:00:00Z",
            "2026-10-01T24:00:00Z",
            "2026-10-01T12:60:00Z",
            "2026-10-01",
            "invalid",
            "2026-10-01T12:00:00.00Z",
        ] {
            assert!(!valid_transaction_timestamp(invalid));
        }
        drop(connection);
        Ok(())
    }

    // This explicit drill requires an imported sanitized fixture database. The
    // shell wrapper creates it under /tmp, invokes this test, then verifies a
    // full export/restore round trip. It stays ignored in ordinary test runs.
    #[tokio::test]
    #[ignore = "run through scripts/test-canonical-db-drill.sh"]
    async fn finance_write_reconciliation_and_close_drill() -> anyhow::Result<()> {
        let database_path = PathBuf::from(
            env::var("PRESTO_FINANCE_DRILL_DB")
                .expect("the drill wrapper must provide a disposable SQLite path"),
        );
        let temp_dir = env::temp_dir();
        assert!(
            database_path.starts_with(temp_dir),
            "drill DB must be under the temporary directory"
        );
        assert!(database_path.is_file(), "sanitized fixture DB must exist");

        let options =
            SqliteConnectOptions::from_str(&format!("sqlite:{}", database_path.display()))?
                .foreign_keys(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let now = now_seconds();
        sqlx::query("INSERT INTO pocket_sessions (id_hash, subject, username, expires_at, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(hash_session_id(SESSION_ID))
            .bind(SUBJECT)
            .bind("disposable-drill")
            .bind(now + 3600)
            .bind(now)
            .execute(&pool)
            .await?;
        // The fixture's source-oriented rate rows intentionally have no ECB
        // provider identity. Add one synthetic prior quote for exercising the
        // canonical USD reconciliation valuation path.
        sqlx::query("INSERT INTO exchange_rates (id, rate_date, base_currency, quote_currency, rate_decimal, provider, source_pair, source_column, source_value_json, source_workbook_id, source_sheet, source_row, original_row_fingerprint) VALUES ('drill-usd-rate', '2026-09-29', 'EUR', 'USD', '1.10', 'ECB', 'EURUSD', 'drill', '{}', 'disposable-drill', 'Exchange Rates', 1, 'synthetic-drill-rate')")
            .execute(&pool)
            .await?;
        // The sanitized archive intentionally omits the month immediately
        // before cutover. Supply an explicit reviewed opening for this drill.
        sqlx::query("INSERT INTO budget_opening_state (category_id,effective_before_month,remaining_eur,verified_by,verified_at,source_json) SELECT id,'2026-09','100','sanitized-api-drill','2026-09-01','{}' FROM finance_categories WHERE is_real=1")
            .execute(&pool).await?;
        let state = AppState {
            config: config(database_path),
            pool: pool.clone(),
            http: reqwest::Client::new(),
            pocket_oidc: PocketOidc {
                authorization_endpoint: "https://pocket-id.test.invalid/authorize".into(),
                token_endpoint: "https://pocket-id.test.invalid/token".into(),
                userinfo_endpoint: "https://pocket-id.test.invalid/userinfo".into(),
            },
        };
        let headers = authenticated_headers();
        let snapshot = api_ok(
            get_db_finance(
                State(state.clone()),
                headers.clone(),
                Query(FinanceQuery {
                    as_of: "2026-09-15".into(),
                    recent_count: Some(200),
                }),
            )
            .await,
        )?;
        assert_eq!(snapshot["budgetMonth"], MONTH);
        assert_eq!(snapshot["canReallocate"], true);
        assert!(snapshot["budgets"].as_array().unwrap().iter().all(|category| category["carryoverEur"] == "100"));


        let mut unauthenticated_headers = HeaderMap::new();
        unauthenticated_headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://localhost:3000"),
        );
        let unauthenticated = create_db_transaction(
            State(state.clone()),
            unauthenticated_headers,
            Json(DbTransactionWriteRequest {
                corrects_transaction_id: None,
            income_treatment: None,
                expected_income_month: None,
                operation_id: uuid7(0x101),
                transaction_id: uuid7(0x102),
                kind: "expense".into(),
                date: "2026-09-15".into(),
                occurred_at: None,
                date_only: false,
                description: "Unauthenticated drill".into(),
                amount: "1.00".into(),
                currency: None,
                to_amount: None,
                from_category_id: Some(GROCERIES.into()),
                to_category_id: None,
                from_account_id: Some(EUR_ACCOUNT.into()),
                to_account_id: None,
            }),
        )
        .await
        .unwrap_err();
        assert_status(unauthenticated, StatusCode::UNAUTHORIZED);

        let balance_before: String =
            sqlx::query_scalar("SELECT current_native_balance FROM finance_accounts WHERE id = ?")
                .bind(EUR_ACCOUNT)
                .fetch_one(&pool)
                .await?;
        let budget_before = {
            let mut c = pool.acquire().await?;
            let category = crate::budget::categories(&mut c, MONTH).await?.into_iter().find(|category| category.category_id==GROCERIES).unwrap();
            (category.expense_eur, category.remaining_eur)
        };
        let timestamp_available: bool =
            sqlx::query_scalar("SELECT julianday('now') >= julianday('2026-09-15T12:00:00.000Z')")
                .fetch_one(&pool)
                .await?;
        let create = DbTransactionWriteRequest {
            corrects_transaction_id: None,
            income_treatment: None,
            expected_income_month: None,
            operation_id: uuid7(0x103),
            transaction_id: uuid7(0x104),
            kind: "expense".into(),
            date: "2026-09-15".into(),
            occurred_at: timestamp_available.then(|| "2026-09-15T12:00:00.000Z".to_string()),
            date_only: false,
            description: "Disposable drill expense".into(),
            amount: "1.00".into(),
            currency: None,
            to_amount: None,
            from_category_id: Some(GROCERIES.into()),
            to_category_id: None,
            from_account_id: Some(EUR_ACCOUNT.into()),
            to_account_id: None,
        };
        let created = api_ok(
            create_db_transaction(State(state.clone()), headers.clone(), Json(create.clone()))
                .await,
        )?;
        assert_eq!(created["recorded"], true);
        assert_eq!(created["transactionId"], create.transaction_id);
        let created_retry = api_ok(
            create_db_transaction(State(state.clone()), headers.clone(), Json(create.clone()))
                .await,
        )?;
        assert_eq!(
            created, created_retry,
            "create retry must return its saved result"
        );
        let mut conflicting_create = create.clone();
        conflicting_create.description.push_str(" changed");
        let conflict = create_db_transaction(
            State(state.clone()),
            headers.clone(),
            Json(conflicting_create),
        )
        .await
        .unwrap_err();
        assert_status(conflict, StatusCode::CONFLICT);

        let post_create_balance: String =
            sqlx::query_scalar("SELECT current_native_balance FROM finance_accounts WHERE id = ?")
                .bind(EUR_ACCOUNT)
                .fetch_one(&pool)
                .await?;
        assert_eq!(
            post_create_balance,
            crate::finance_model::subtract_decimal_text(&balance_before, "1.00")
                .map_err(anyhow::Error::msg)?
        );
        let post_create_budget = {
            let mut c = pool.acquire().await?;
            let category = crate::budget::categories(&mut c, MONTH)
                .await?
                .into_iter()
                .find(|category| category.category_id == GROCERIES)
                .unwrap();
            (
                category.expense_eur.unwrap(),
                category.remaining_eur.unwrap(),
            )
        };
        assert_eq!(
            post_create_budget.0,
            crate::finance_model::add_decimal_text(budget_before.0.as_deref().unwrap(), "1.00")
                .map_err(anyhow::Error::msg)?
        );
        assert_eq!(
            post_create_budget.1,
            crate::finance_model::subtract_decimal_text(
                budget_before.1.as_deref().unwrap(),
                "1.00"
            )
            .map_err(anyhow::Error::msg)?
        );

        let edit = DbTransactionEditRequest {
            transaction: DbTransactionWriteRequest {
                corrects_transaction_id: None,
            income_treatment: None,
                expected_income_month: None,
                operation_id: uuid7(0x105),
                transaction_id: create.transaction_id.clone(),
                kind: "expense".into(),
                date: "2026-09-15".into(),
                occurred_at: None,
                date_only: false,
                description: "Disposable drill expense edited".into(),
                amount: "2.00".into(),
                currency: None,
                to_amount: None,
                from_category_id: Some(GROCERIES.into()),
                to_category_id: None,
                from_account_id: Some(EUR_ACCOUNT.into()),
                to_account_id: None,
            },
            expected_revision: 1,
            expected_account_revisions: vec![DbExpectedAccountRevision {
                account_id: EUR_ACCOUNT.into(),
                revision: 1,
            }],
        };
        let edited = api_ok(
            edit_db_transaction(State(state.clone()), headers.clone(), Json(edit.clone())).await,
        )?;
        assert_eq!(edited["updated"], true);
        assert_eq!(edited["revision"], 2);
        let edited_retry = api_ok(
            edit_db_transaction(State(state.clone()), headers.clone(), Json(edit.clone())).await,
        )?;
        assert_eq!(
            edited, edited_retry,
            "edit retry must return its saved result"
        );
        let persisted_timestamp: Option<String> =
            sqlx::query_scalar("SELECT occurred_at FROM finance_transactions WHERE id = ?")
                .bind(&create.transaction_id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(
            persisted_timestamp, create.occurred_at,
            "description/amount edits preserve occurrence time"
        );
        let post_edit_balance: String =
            sqlx::query_scalar("SELECT current_native_balance FROM finance_accounts WHERE id = ?")
                .bind(EUR_ACCOUNT)
                .fetch_one(&pool)
                .await?;
        assert_eq!(
            post_edit_balance,
            crate::finance_model::subtract_decimal_text(&balance_before, "2.00")
                .map_err(anyhow::Error::msg)?
        );
        let revisions: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM finance_transaction_revisions WHERE transaction_id = ?",
        )
        .bind(&create.transaction_id)
        .fetch_one(&pool)
        .await?;
        assert_eq!(revisions, 2);
        let mut stale_edit = edit.clone();
        stale_edit.transaction.operation_id = uuid7(0x106);
        let stale_error =
            edit_db_transaction(State(state.clone()), headers.clone(), Json(stale_edit))
                .await
                .unwrap_err();
        assert_status(stale_error, StatusCode::CONFLICT);

        let before_partial = account_revisions(&pool).await?;
        let first_real = before_partial.first().expect("fixture has a real account");
        let partial = DbReconciliationRequest {
            operation_id: uuid7(0x107),
            date: "2026-09-30".into(),
            observed_at: None,
            observed_date: None,
            accounts: vec![DbReconciliationAccount {
                account_id: first_real.0.clone(),
                actual_balance: first_real.1.clone(),
                transaction_id: uuid7(0x108),
                expected_revision: first_real.2,
            }],
        };
        let partial_result = api_ok(
            record_db_reconciliation(State(state.clone()), headers.clone(), Json(partial.clone()))
                .await,
        )?;
        assert_eq!(partial_result["recorded"], true);

        let close_id = uuid7(0x109);
        let premature_close = close_budget_month(
            State(state.clone()),
            headers.clone(),
            Json(CloseBudgetMonthRequest {
                operation_id: close_id.clone(),
                month: MONTH.into(),
            }),
        )
        .await
        .unwrap_err();
        assert_status(premature_close, StatusCode::CONFLICT);

        let after_partial = account_revisions(&pool).await?;
        let mut full_accounts = Vec::new();
        for (index, (account_id, balance, revision)) in after_partial.iter().enumerate() {
            let actual_balance = match account_id.as_str() {
                EUR_ACCOUNT => crate::finance_model::add_decimal_text(balance, "0.55")
                    .map_err(anyhow::Error::msg)?,
                USD_ACCOUNT => crate::finance_model::subtract_decimal_text(balance, "1.25")
                    .map_err(anyhow::Error::msg)?,
                _ => anyhow::bail!("unexpected real account in sanitized fixture"),
            };
            full_accounts.push(DbReconciliationAccount {
                account_id: account_id.clone(),
                actual_balance,
                transaction_id: uuid7(0x110 + index as u16),
                expected_revision: *revision,
            });
        }
        assert_eq!(
            full_accounts.len(),
            2,
            "fixture expects two active real accounts"
        );
        let full = DbReconciliationRequest {
            operation_id: uuid7(0x112),
            date: "2026-09-30".into(),
            observed_at: None,
            observed_date: None,
            accounts: full_accounts,
        };
        let reconciled = api_ok(
            record_db_reconciliation(State(state.clone()), headers.clone(), Json(full.clone()))
                .await,
        )?;
        assert_eq!(reconciled["recorded"], true);
        let reconciliation_ids = reconciled["accounts"]
            .as_array()
            .expect("response contains account results")
            .iter()
            .map(|entry| entry["transactionId"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let reconciled_retry = api_ok(
            record_db_reconciliation(State(state.clone()), headers.clone(), Json(full.clone()))
                .await,
        )?;
        assert_eq!(
            reconciled, reconciled_retry,
            "reconciliation retry must return saved result"
        );

        let after_full = account_revisions(&pool).await?;
        let replacement_accounts = full
            .accounts
            .iter()
            .zip(after_full.iter())
            .enumerate()
            .map(|(index, (old, current))| {
                Ok(DbReconciliationAccount {
                    account_id: old.account_id.clone(),
                    actual_balance: crate::finance_model::add_decimal_text(
                        &old.actual_balance,
                        "0.10",
                    )
                    .map_err(anyhow::Error::msg)?,
                    transaction_id: uuid7(0x120 + index as u16),
                    expected_revision: current.2,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let replacement = DbReconciliationRequest {
            operation_id: uuid7(0x122),
            date: "2026-09-30".into(),
            observed_at: None,
            observed_date: None,
            accounts: replacement_accounts,
        };
        let replaced = api_ok(
            record_db_reconciliation(
                State(state.clone()),
                headers.clone(),
                Json(replacement.clone()),
            )
            .await,
        )?;
        let replaced_ids = replaced["accounts"]
            .as_array()
            .expect("replacement response contains account results")
            .iter()
            .map(|entry| entry["transactionId"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            reconciliation_ids, replaced_ids,
            "replacement keeps stable reconciliation transaction IDs"
        );
        let replaced_retry = api_ok(
            record_db_reconciliation(
                State(state.clone()),
                headers.clone(),
                Json(replacement.clone()),
            )
            .await,
        )?;
        assert_eq!(
            replaced, replaced_retry,
            "replacement retry must return saved result"
        );

        let stale_replacement = DbReconciliationRequest {
            operation_id: uuid7(0x123),
            date: replacement.date.clone(),
            observed_at: None,
            observed_date: None,
            accounts: full.accounts.clone(),
        };
        let stale_reconciliation_error = record_db_reconciliation(
            State(state.clone()),
            headers.clone(),
            Json(stale_replacement),
        )
        .await
        .unwrap_err();
        assert_status(stale_reconciliation_error, StatusCode::CONFLICT);
        let batches: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM reconciliation_batches WHERE effective_date = '2026-09-30'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(
            batches, 3,
            "partial, full, and replacement batches are retained"
        );
        let live_reconciliation_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM finance_transactions WHERE kind = 'reconciliation' AND effective_date = '2026-09-30'")
            .fetch_one(&pool)
            .await?;
        assert_eq!(
            live_reconciliation_rows, 2,
            "one replaceable ledger row remains per real account"
        );

        let closed = api_ok(
            close_budget_month(
                State(state.clone()),
                headers.clone(),
                Json(CloseBudgetMonthRequest {
                    operation_id: close_id.clone(),
                    month: MONTH.into(),
                }),
            )
            .await,
        )?;
        assert_eq!(closed["closed"], true);
        assert_eq!(closed["categoryCount"], 2);
        let closed_retry = api_ok(
            close_budget_month(
                State(state.clone()),
                headers.clone(),
                Json(CloseBudgetMonthRequest {
                    operation_id: close_id,
                    month: MONTH.into(),
                }),
            )
            .await,
        )?;
        assert_eq!(
            closed, closed_retry,
            "close retry must return its saved digest"
        );
        let second_close = close_budget_month(
            State(state.clone()),
            headers.clone(),
            Json(CloseBudgetMonthRequest {
                operation_id: uuid7(0x124),
                month: MONTH.into(),
            }),
        )
        .await
        .unwrap_err();
        assert_status(second_close, StatusCode::CONFLICT);
        let mutation_after_close = sqlx::query(
            "UPDATE budget_month_snapshots SET remaining_eur = remaining_eur WHERE month = ?",
        )
        .bind(MONTH)
        .execute(&pool)
        .await;
        assert!(
            mutation_after_close.is_err(),
            "closed snapshot trigger must reject updates"
        );
        let posting_after_close = create_db_transaction(
            State(state.clone()),
            headers.clone(),
            Json(DbTransactionWriteRequest {
                corrects_transaction_id: None,
            income_treatment: None,
                expected_income_month: None,
                operation_id: uuid7(0x125),
                transaction_id: uuid7(0x126),
                kind: "expense".into(),
                date: "2026-09-28".into(),
                occurred_at: None,
                date_only: false,
                description: "Must not post into closed month".into(),
                amount: "1.00".into(),
                currency: None,
                to_amount: None,
                from_category_id: Some(GROCERIES.into()),
                to_category_id: None,
                from_account_id: Some(EUR_ACCOUNT.into()),
                to_account_id: None,
            }),
        )
        .await
        .unwrap_err();
        assert_status(posting_after_close, StatusCode::CONFLICT);

        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&pool)
            .await?;
        assert_eq!(integrity, "ok");
        let foreign_key_issues = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await?;
        assert!(foreign_key_issues.is_empty());
        pool.close().await;
        println!(
            "Disposable finance API drill passed: auth, create/retry/conflict, edit/retry/stale revision, reconciliation partial/full/retry/replacement/stale revision, close prerequisite/retry/immutability, and closed-month write rejection."
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "run through scripts/test-canonical-db-drill.sh"]
    async fn mobile_snapshot_restore_drill() -> anyhow::Result<()> {
        let source_path = PathBuf::from(
            env::var("PRESTO_FINANCE_DRILL_DB")
                .expect("the drill wrapper must provide a disposable SQLite path"),
        );
        assert!(source_path.starts_with(env::temp_dir()));
        assert!(source_path.is_file());
        let source_options =
            SqliteConnectOptions::from_str(&format!("sqlite:{}", source_path.display()))?
                .foreign_keys(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let source_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(source_options)
            .await?;
        let default_snapshot =
            crate::finance_read_model::load(&source_pool, "2026-09-01", 0).await?;
        assert_eq!(default_snapshot.income_expectation.amount, "10000");
        assert_eq!(default_snapshot.income_expectation.currency, "USD");
        assert!(default_snapshot.recent_transactions.is_empty());
        let today: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%d', 'now')")
            .fetch_one(&source_pool)
            .await?;
        let expected_date = if today.starts_with("2026-09") {
            today.as_str()
        } else {
            "2026-09-30"
        };
        assert_eq!(default_snapshot.account_valuation.as_of, expected_date);
        assert!(default_snapshot.account_valuation.rates.iter().all(|rate| {
            rate.date
                .as_deref()
                .is_none_or(|date| date <= expected_date)
        }));
        sqlx::query("INSERT INTO budget_income_expectations (effective_start_month, amount, currency, arrival_day) VALUES ('2026-09', '10000', 'USD', 7)")
            .execute(&source_pool).await?;
        let source_state = restore_test_state(source_pool.clone(), source_path.clone(), false);
        let backup_response =
            get_db_finance_backup(State(source_state.clone()), authenticated_headers())
                .await
                .map_err(|error| anyhow::anyhow!("{}: {}", error.status, error.message))?;
        let snapshot_digest = backup_response
            .headers()
            .get("x-finance-snapshot-sha256")
            .expect("snapshot response includes digest")
            .to_str()?
            .to_string();
        let snapshot_bytes =
            axum::body::to_bytes(backup_response.into_body(), 64 * 1024 * 1024).await?;
        let snapshot_text = String::from_utf8(snapshot_bytes.to_vec())?;
        let source_export: Value = serde_json::from_str(&snapshot_text)?;
        assert_eq!(source_export["interchangeFormatVersion"], 1);
        assert_eq!(
            source_export["tableCounts"].as_object().unwrap().len(),
            FINANCE_BACKUP_TABLES.len()
        );

        let bad_digest_path = source_path.with_file_name("restore-bad-digest.sqlite");
        let bad_digest_pool = open_empty_drill_database(&bad_digest_path).await?;
        insert_drill_pocket_session(&bad_digest_pool).await?;
        let bad_digest_state =
            restore_test_state(bad_digest_pool.clone(), bad_digest_path.clone(), true);
        let bad_digest_error = restore_db_finance(
            State(bad_digest_state.clone()),
            restore_headers(&"0".repeat(64)),
            axum::body::Bytes::from(snapshot_bytes.to_vec()),
        )
        .await
        .unwrap_err();
        assert_status(bad_digest_error, StatusCode::BAD_REQUEST);
        assert_all_finance_tables_empty(&bad_digest_pool).await?;

        let mut wrong_schema: Value = serde_json::from_slice(&snapshot_bytes)?;
        let schema_version = wrong_schema["databaseSchemaVersion"]
            .as_i64()
            .expect("snapshot has an integer database schema version");
        wrong_schema["databaseSchemaVersion"] = json!(schema_version + 1);
        let wrong_schema_bytes = serde_json::to_vec(&wrong_schema)?;
        let wrong_schema_digest = format!("{:x}", Sha256::digest(&wrong_schema_bytes));
        let schema_error = restore_db_finance(
            State(bad_digest_state),
            restore_headers(&wrong_schema_digest),
            axum::body::Bytes::from(wrong_schema_bytes),
        )
        .await
        .unwrap_err();
        assert_status(schema_error, StatusCode::CONFLICT);
        assert_all_finance_tables_empty(&bad_digest_pool).await?;

        let nonempty_path = source_path.with_file_name("restore-nonempty.sqlite");
        let nonempty_pool = open_empty_drill_database(&nonempty_path).await?;
        insert_drill_pocket_session(&nonempty_pool).await?;
        sqlx::query("INSERT INTO finance_categories (id, name, is_real, source_row, source_row_json) VALUES ('recovery-sentinel', 'Recovery sentinel', 1, 1, '{}')")
            .execute(&nonempty_pool)
            .await?;
        let nonempty_state = restore_test_state(nonempty_pool.clone(), nonempty_path.clone(), true);
        let nonempty_error = restore_db_finance(
            State(nonempty_state),
            restore_headers(&snapshot_digest),
            axum::body::Bytes::from(snapshot_bytes.to_vec()),
        )
        .await
        .unwrap_err();
        assert_status(nonempty_error, StatusCode::CONFLICT);
        let sentinel_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM finance_categories WHERE id = 'recovery-sentinel'",
        )
        .fetch_one(&nonempty_pool)
        .await?;
        assert_eq!(sentinel_count, 1, "restore must preserve the existing row");
        let transaction_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_transactions")
                .fetch_one(&nonempty_pool)
                .await?;
        assert_eq!(
            transaction_count, 0,
            "non-empty restore must not partially import"
        );

        let restored_path = source_path.with_file_name("restore-success.sqlite");
        let restored_pool = open_empty_drill_database(&restored_path).await?;
        insert_drill_pocket_session(&restored_pool).await?;
        let restored_state = restore_test_state(restored_pool.clone(), restored_path.clone(), true);
        let restored = restore_db_finance(
            State(restored_state.clone()),
            restore_headers(&snapshot_digest),
            axum::body::Bytes::from(snapshot_bytes.to_vec()),
        )
        .await
        .map(|Json(value)| value)
        .map_err(|error| anyhow::anyhow!("{}: {}", error.status, error.message))?;
        assert_eq!(restored["restored"], true);
        assert_eq!(
            restored["tableCounts"].as_object().unwrap().len(),
            FINANCE_BACKUP_TABLES.len()
        );

        let restored_response =
            get_db_finance_backup(State(restored_state), authenticated_headers())
                .await
                .map_err(|error| anyhow::anyhow!("{}: {}", error.status, error.message))?;
        let restored_bytes =
            axum::body::to_bytes(restored_response.into_body(), 64 * 1024 * 1024).await?;
        let restored_export: Value = serde_json::from_slice(&restored_bytes)?;
        assert_eq!(
            restored_export["databaseSchemaVersion"],
            source_export["databaseSchemaVersion"]
        );
        assert_eq!(
            restored_export["operationWatermark"],
            source_export["operationWatermark"]
        );
        assert_eq!(restored_export["tableCounts"], source_export["tableCounts"]);
        assert_eq!(restored_export["tables"], source_export["tables"]);
        for table in FINANCE_BACKUP_TABLES {
            let count = restored_export["tableCounts"][*table]
                .as_u64()
                .expect("restored table has a declared count");
            assert_eq!(
                count,
                source_export["tables"][*table].as_array().unwrap().len() as u64,
                "restored row count differs for {table}"
            );
        }
        assert_all_finance_tables_empty(&bad_digest_pool).await?;
        bad_digest_pool.close().await;
        nonempty_pool.close().await;
        restored_pool.close().await;
        source_pool.close().await;
        println!(
            "Disposable mobile snapshot restore drill passed: digest rejection, schema mismatch, non-empty target rejection, and exact restore parity for all finance tables."
        );
        Ok(())
    }
}
