use crate::server::ServerArgs;
use clap::{Parser, Subcommand};
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
    Run(ServerArgs),
}
