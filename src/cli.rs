use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;
use tracing::Level;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    /// The minimum level to log at, one of: trace|debug|info|warn|error
    #[arg(short, long, default_value_t = Level::INFO, env = "LOG_LEVEL")]
    pub log_level: Level,

    /// The address of the management server
    #[arg(short, long, default_value = "127.0.0.1:8032", env = "ADDRESS")]
    pub address: SocketAddr,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
#[command(rename_all = "kebab-case")]
pub enum Command {
    /// Launch the server
    Run(RunArgs),
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// The address for the management server to listen on
    #[arg(
        short,
        long,
        default_value = "127.0.0.1:8032",
        env = "MANAGEMENT_ADDRESS"
    )]
    pub management_address: SocketAddr,
}
