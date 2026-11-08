//! `hura-cloud`: the service, configured by its environment (see
//! `hura_cloud::config`). Brings the database up to date, then serves until
//! told to stop.

use std::process::ExitCode;
use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hura-cloud: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let config = hura_cloud::config::Config::from_env()?;
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .map_err(|e| format!("could not reach the database: {e}"))?;
    hura_cloud::migrate(&db)
        .await
        .map_err(|e| format!("could not bring the database up to date: {e}"))?;

    hura_cloud::install_transport();
    let state = Arc::new(hura_cloud::State {
        db,
        mail: config.mail,
        keys: config.keys,
        relay: hura_cloud::push::Relay::https(config.relay),
    });
    tokio::spawn(hura_cloud::watch::run(state.clone()));
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .map_err(|e| format!("could not listen on {}: {e}", config.listen))?;
    eprintln!("hura-cloud: listening on {}", config.listen);
    axum::serve(listener, hura_cloud::app(state))
        .with_graceful_shutdown(stopped())
        .await
        .map_err(|e| e.to_string())
}

/// Ctrl-C, or the SIGTERM a container is stopped with.
async fn stopped() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("a SIGTERM handler");
        tokio::select! {
            _ = interrupt => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = interrupt.await;
}
