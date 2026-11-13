use std::path::PathBuf;
use std::process::ExitCode;

use routing_console::{app::App, clock::Clock, http, store::Store};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("routing-console: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = match std::env::var("PORT") {
        Ok(p) => p
            .parse()
            .map_err(|_| format!("PORT {p:?} is not a port number"))?,
        Err(_) => 3000,
    };
    let db = std::env::var("ROUTING_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("routing-console.db"));
    let app = App::start(Store::open(&db)?, Clock::system())?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    eprintln!(
        "routing-console: serving http://localhost:{port} with database {}",
        db.display()
    );
    axum::serve(listener, http::router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
