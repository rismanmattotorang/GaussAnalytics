//! `gaussctl` — the GaussAnalytics command-line entry point.
//!
//! A single binary that runs the server, launches the administration TUI, runs
//! database migrations, and reports version information. GaussAnalytics is
//! owned and operated by Gaussian Technologies.

#![forbid(unsafe_code)]

use std::error::Error;

use clap::{Parser, Subcommand};
use gauss_config::AppConfig;

/// GaussAnalytics control CLI.
#[derive(Parser)]
#[command(
    name = "gaussctl",
    version,
    about = "GaussAnalytics by Gaussian Technologies"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP API server (serves the API and the frontend).
    Serve,
    /// Launch the operator administration console (TUI).
    Admin,
    /// Run application-database migrations (Phase 2).
    Migrate,
    /// Print version and ownership information.
    Version,
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let cli = Cli::parse();
    init_tracing();

    match cli.command {
        Command::Serve => {
            let config = AppConfig::from_env()?;
            // The TUI is sync; the server is async. Build a runtime just here.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(gauss_server::serve(config))?;
        }
        Command::Admin => {
            gauss_tui::run()?;
        }
        Command::Migrate => {
            let config = AppConfig::from_env()?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(gauss_db::migrate_url(&config.database.url))?;
            println!(
                "GaussAnalytics: migrations applied to {}",
                config.database.url
            );
        }
        Command::Version => {
            println!(
                "{} {}\nowner: Gaussian Technologies",
                gauss_server::PRODUCT_NAME,
                env!("CARGO_PKG_VERSION"),
            );
        }
    }
    Ok(())
}

/// Initialize structured logging, honoring `RUST_LOG` (default: `info`).
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
