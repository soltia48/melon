//! `melon-server` binary entry point.

use std::process::ExitCode;
use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use melon_auth::{KeyStore, SessionManager};
use melon_server::{AppState, Config, router, spawn_expiry_sweeper};

#[tokio::main]
async fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("configuration error: {e}");
            return ExitCode::FAILURE;
        }
    };

    match run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(config: Config) -> Result<(), String> {
    let pool = melon_db::connect(&config.database_url)
        .await
        .map_err(|e| format!("database connection failed: {e}"))?;
    melon_db::migrate(&pool)
        .await
        .map_err(|e| format!("migrations failed: {e}"))?;
    tracing::info!("migrations applied");

    let keystore = KeyStore::from_jsonl(&config.keys_path).map_err(|e| e.message)?;
    let manager = SessionManager::new(
        Arc::new(keystore),
        None,
        config.session_ttl,
        config.max_sessions,
    );
    Arc::clone(&manager).spawn_reaper();

    bootstrap_admin(&pool, config.bootstrap_admin.as_ref()).await?;
    if !config.cookie_secure {
        tracing::warn!(
            "session cookies are NOT marked Secure (loopback/plain-HTTP mode). \
             Set MELON_COOKIE_SECURE=true behind TLS in production."
        );
    }

    let tz = melon_core::expiry::expiry_timezone().map_err(|e| e.to_string())?;
    let state = AppState {
        pool,
        manager,
        tz,
        user_session_ttl: config.user_session_ttl,
        cookie_secure: config.cookie_secure,
        default_fee_bps: config.default_fee_bps,
        default_credit_limit: config.default_credit_limit,
    };
    spawn_expiry_sweeper(state.clone(), config.sweep_interval);
    melon_server::spawn_session_reaper(state.clone());

    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .map_err(|e| format!("failed to bind {}: {e}", config.bind_addr))?;
    tracing::info!(addr = %config.bind_addr, "melon-server listening");

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("server error: {e}"))
}

/// First run: create the initial admin user from the environment. Idempotent —
/// it does nothing once any admin exists, so the credentials can stay in the
/// deployment config without recreating or resetting the account.
async fn bootstrap_admin(
    pool: &melon_db::Pool,
    creds: Option<&(String, String)>,
) -> Result<(), String> {
    if melon_db::users::admin_exists(pool)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    let Some((email, password)) = creds else {
        tracing::warn!(
            "no admin user exists and MELON_BOOTSTRAP_ADMIN_EMAIL / \
             MELON_BOOTSTRAP_ADMIN_PASSWORD are unset — nobody can sign in to /admin"
        );
        return Ok(());
    };
    let hash = melon_server::auth::hash_password(password)
        .map_err(|_| "failed to hash the bootstrap admin password".to_string())?;
    melon_db::users::create_user(pool, email, "Administrator", &hash, "admin", None)
        .await
        .map_err(|e| format!("failed to create the bootstrap admin: {e}"))?;
    tracing::info!(email, "bootstrap admin user created");
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
