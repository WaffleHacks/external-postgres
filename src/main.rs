use clap::Parser;
use dotenvy::dotenv;
use tracing::info;

mod cli;
mod logging;

use cli::Cli;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    dotenv().ok();
    let args = Cli::parse();

    logging::init(args.log_level)?;

    info!(?args);

    Ok(())
}
