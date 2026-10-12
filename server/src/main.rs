mod budget;
mod budget_ledger;
#[allow(dead_code)]
mod finance_model;
mod finance_read_model;
mod server;
mod push;

use std::{collections::HashSet, env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use server::{AppState, app};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Clone)]
pub(crate) struct Config {
    google_client_id: String,
    google_client_secret: String,
    google_redirect_uri: String,
    pocket_id_issuer: String,
    pocket_id_client_id: String,
    pocket_id_redirect_uri: String,
    pocket_id_allowed_subjects: HashSet<String>,
    public_origin: String,
    apps_script_deployment_id: String,
    allowed_emails: Vec<String>,
    finance_db_read_enabled: bool,
    finance_db_restore_enabled: bool,
    finance_db_close_enabled: bool,
    finance_db_category_corrections_enabled: bool,
    finance_db_transaction_writes_enabled: bool,
    finance_db_reconciliation_writes_enabled: bool,
    encryption_key: [u8; 32],
    cookie_secure: bool,
    bind_addr: SocketAddr,
    database_path: PathBuf,
    static_dir: PathBuf,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        let configured_origin = required_env("PUBLIC_ORIGIN")?;
        let public_url = Url::parse(&(configured_origin.trim_end_matches('/').to_string() + "/"))
            .context("PUBLIC_ORIGIN must be a valid URL origin")?;
        let is_local_origin = public_url.scheme() == "http"
            && matches!(
                public_url.host_str(),
                Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
            );
        if public_url.scheme() != "https" && !is_local_origin {
            bail!("PUBLIC_ORIGIN must use HTTPS, except for localhost development.");
        }
        if !public_url.username().is_empty()
            || public_url.password().is_some()
            || public_url.path() != "/"
            || public_url.query().is_some()
            || public_url.fragment().is_some()
        {
            bail!("PUBLIC_ORIGIN must contain only the scheme, host, and optional port.");
        }
        let public_origin = public_url.origin().ascii_serialization();

        let allowed_emails = required_env("ALLOWED_EMAILS")?
            .split(',')
            .map(|email| email.trim().to_ascii_lowercase())
            .filter(|email| !email.is_empty())
            .collect::<Vec<_>>();
        if allowed_emails.is_empty() || allowed_emails.iter().any(|email| !email.contains('@')) {
            bail!("ALLOWED_EMAILS must contain one or more comma-separated email addresses.");
        }

        let encryption_key = STANDARD
            .decode(required_env("TOKEN_ENCRYPTION_KEY")?)
            .context("TOKEN_ENCRYPTION_KEY must be base64-encoded")?;
        let encryption_key: [u8; 32] = encryption_key
            .try_into()
            .map_err(|_| anyhow::anyhow!("TOKEN_ENCRYPTION_KEY must decode to exactly 32 bytes"))?;

        let cookie_secure = env::var("COOKIE_SECURE")
            .map(|value| value != "false")
            .unwrap_or(!is_local_origin);
        if !is_local_origin && !cookie_secure {
            bail!("COOKIE_SECURE must be true for hosted HTTPS deployments.");
        }

        let bind_addr = env::var("BIND_ADDR")
            .unwrap_or_else(|_| {
                format!(
                    "0.0.0.0:{}",
                    env::var("PORT").unwrap_or_else(|_| "3000".to_string())
                )
            })
            .parse::<SocketAddr>()
            .context("BIND_ADDR must be a socket address, for example 0.0.0.0:3000")?;
        let database_path = env::var_os("DATABASE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../data/presto.sqlite")
            });
        let static_dir = env::var_os("STATIC_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/dist"));

        let redirect_uri = required_env("GOOGLE_REDIRECT_URI")?;
        let redirect_url =
            Url::parse(&redirect_uri).context("GOOGLE_REDIRECT_URI must be a valid URL")?;
        if redirect_url.origin() != public_url.origin()
            || !redirect_url.username().is_empty()
            || redirect_url.password().is_some()
            || redirect_url.path() != "/auth/google/callback"
            || redirect_url.query().is_some()
            || redirect_url.fragment().is_some()
        {
            bail!("GOOGLE_REDIRECT_URI must use the PUBLIC_ORIGIN host.");
        }

        let pocket_id_redirect_uri = required_env("POCKET_ID_REDIRECT_URI")?;
        let pocket_id_redirect_url = Url::parse(&pocket_id_redirect_uri)
            .context("POCKET_ID_REDIRECT_URI must be a valid URL")?;
        if pocket_id_redirect_url.origin() != public_url.origin()
            || !pocket_id_redirect_url.username().is_empty()
            || pocket_id_redirect_url.password().is_some()
            || pocket_id_redirect_url.path() != "/auth/pocket-id/callback"
            || pocket_id_redirect_url.query().is_some()
            || pocket_id_redirect_url.fragment().is_some()
        {
            bail!("POCKET_ID_REDIRECT_URI must use the PUBLIC_ORIGIN host.");
        }

        let pocket_id_issuer = required_env("POCKET_ID_ISSUER")?;
        let pocket_id_issuer_url =
            Url::parse(&pocket_id_issuer).context("POCKET_ID_ISSUER must be a valid URL")?;
        if pocket_id_issuer_url.scheme() != "https"
            || pocket_id_issuer_url.host_str().is_none()
            || !pocket_id_issuer_url.username().is_empty()
            || pocket_id_issuer_url.password().is_some()
            || pocket_id_issuer_url.query().is_some()
            || pocket_id_issuer_url.fragment().is_some()
        {
            bail!("POCKET_ID_ISSUER must be a trusted HTTPS issuer URL.");
        }
        let pocket_id_issuer = pocket_id_issuer_url
            .as_str()
            .trim_end_matches('/')
            .to_string();

        let pocket_id_allowed_subjects = required_env("POCKET_ID_ALLOWED_SUBJECTS")?
            .split(',')
            .map(str::trim)
            .filter(|subject| !subject.is_empty())
            .map(str::to_string)
            .collect::<HashSet<_>>();
        if pocket_id_allowed_subjects.is_empty()
            || pocket_id_allowed_subjects
                .iter()
                .any(|subject| uuid::Uuid::parse_str(subject).is_err())
        {
            bail!(
                "POCKET_ID_ALLOWED_SUBJECTS must contain comma-separated Pocket ID subject UUIDs."
            );
        }

        let apps_script_deployment_id = required_env("APPS_SCRIPT_DEPLOYMENT_ID")?;
        if apps_script_deployment_id.len() < 20
            || !apps_script_deployment_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            bail!("APPS_SCRIPT_DEPLOYMENT_ID has an invalid format.");
        }

        let finance_db_close_enabled = match env::var("FINANCE_DB_CLOSE_ENABLED") {
            Ok(value) if value == "true" => true,
            Ok(value) if value == "false" => false,
            Ok(_) => bail!("FINANCE_DB_CLOSE_ENABLED must be true or false."),
            Err(env::VarError::NotPresent) => false,
            Err(error) => return Err(error).context("FINANCE_DB_CLOSE_ENABLED is invalid"),
        };
        let finance_db_read_enabled = match env::var("FINANCE_DB_READ_ENABLED") {
            Ok(value) if value == "true" => true,
            Ok(value) if value == "false" => false,
            Ok(_) => bail!("FINANCE_DB_READ_ENABLED must be true or false."),
            Err(env::VarError::NotPresent) => false,
            Err(error) => return Err(error).context("FINANCE_DB_READ_ENABLED is invalid"),
        };
        let finance_db_restore_enabled = match env::var("FINANCE_DB_RESTORE_ENABLED") {
            Ok(value) if value == "true" => true,
            Ok(value) if value == "false" => false,
            Ok(_) => bail!("FINANCE_DB_RESTORE_ENABLED must be true or false."),
            Err(env::VarError::NotPresent) => false,
            Err(error) => return Err(error).context("FINANCE_DB_RESTORE_ENABLED is invalid"),
        };
        let finance_db_category_corrections_enabled =
            match env::var("FINANCE_DB_CATEGORY_CORRECTIONS_ENABLED") {
                Ok(value) if value == "true" => true,
                Ok(value) if value == "false" => false,
                Ok(_) => bail!("FINANCE_DB_CATEGORY_CORRECTIONS_ENABLED must be true or false."),
                Err(env::VarError::NotPresent) => false,
                Err(error) => {
                    return Err(error)
                        .context("FINANCE_DB_CATEGORY_CORRECTIONS_ENABLED is invalid");
                }
            };
        let finance_db_transaction_writes_enabled =
            match env::var("FINANCE_DB_TRANSACTION_WRITES_ENABLED") {
                Ok(value) if value == "true" => true,
                Ok(value) if value == "false" => false,
                Ok(_) => bail!("FINANCE_DB_TRANSACTION_WRITES_ENABLED must be true or false."),
                Err(env::VarError::NotPresent) => false,
                Err(error) => {
                    return Err(error).context("FINANCE_DB_TRANSACTION_WRITES_ENABLED is invalid");
                }
            };
        let finance_db_reconciliation_writes_enabled =
            match env::var("FINANCE_DB_RECONCILIATION_WRITES_ENABLED") {
                Ok(value) if value == "true" => true,
                Ok(value) if value == "false" => false,
                Ok(_) => bail!("FINANCE_DB_RECONCILIATION_WRITES_ENABLED must be true or false."),
                Err(env::VarError::NotPresent) => false,
                Err(error) => {
                    return Err(error)
                        .context("FINANCE_DB_RECONCILIATION_WRITES_ENABLED is invalid");
                }
            };

        if finance_db_restore_enabled
            && (finance_db_transaction_writes_enabled
                || finance_db_reconciliation_writes_enabled
                || finance_db_category_corrections_enabled
                || finance_db_close_enabled)
        {
            bail!(
                "Disable all FINANCE_DB_* write flags before enabling FINANCE_DB_RESTORE_ENABLED."
            );
        }

        Ok(Self {
            google_client_id: required_env("GOOGLE_CLIENT_ID")?,
            google_client_secret: required_env("GOOGLE_CLIENT_SECRET")?,
            google_redirect_uri: redirect_uri,
            pocket_id_issuer,
            pocket_id_client_id: required_env("POCKET_ID_CLIENT_ID")?,
            pocket_id_redirect_uri,
            pocket_id_allowed_subjects,
            public_origin,
            apps_script_deployment_id,
            allowed_emails,
            finance_db_read_enabled,
            finance_db_restore_enabled,
            finance_db_close_enabled,
            finance_db_category_corrections_enabled,
            finance_db_transaction_writes_enabled,
            finance_db_reconciliation_writes_enabled,
            encryption_key,
            cookie_secure,
            bind_addr,
            database_path,
            static_dir,
        })
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    let value = env::var(name).with_context(|| format!("{name} is required"))?;
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(value)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    if let Some(parent) = config
        .database_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create database directory {}", parent.display()))?;
    }

    let sqlite_options = SqliteConnectOptions::new()
        .filename(&config.database_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(sqlite_options)
        .await
        .context("could not open SQLite database")?;
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("could not apply database migrations")?;

    let push = push::FcmSender::from_env()?;
    if let Some(sender) = push {
        push::start_worker(pool.clone(), sender, config.pocket_id_allowed_subjects.clone());
    }
    let state = AppState::new(config.clone(), pool).await?;
    let app = app(state, &config.static_dir)?;
    let listener = TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("could not bind to {}", config.bind_addr))?;
    tracing::info!(address = %config.bind_addr, "Presto finance server is listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("HTTP server stopped with an error")?;
    Ok(())
}
