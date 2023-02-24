use clap::Parser;
use dotenvy::dotenv;
use tracing::debug;

mod cli;
mod logging;
mod server;

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    dotenv().ok();
    let args = Cli::parse();

    logging::init(args.log_level)?;
    debug!(?args);

    match args.command {
        Command::Run(args) => server::launch(args).await?,
    }

    Ok(())
}
