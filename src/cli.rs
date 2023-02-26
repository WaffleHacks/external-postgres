use crate::{client::DatabaseCommand, server::ServerArgs};
use clap::{Parser, Subcommand};
use std::fmt::{Debug, Formatter};
use tracing::Level;
use url::Url;

#[derive(Parser)]
#[command(author, version, about)]
pub struct Cli {
    /// The minimum level to log at, one of: trace|debug|info|warn|error
    #[arg(short, long, default_value_t = Level::INFO)]
    pub log_level: Level,

    /// The address of the management server
    #[arg(short, long, default_value = "http://127.0.0.1:8032", env = "ADDRESS")]
    pub address: Url,

    #[command(subcommand)]
    pub command: Command,
}

impl Debug for Cli {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cli")
            .field("log_level", &self.log_level.as_str())
            .field("address", &self.address.as_str())
            .field("command", &self.command)
            .finish()
    }
}

#[derive(Debug, Subcommand)]
#[command(rename_all = "kebab-case")]
pub enum Command {
    /// Launch the server
    Run(ServerArgs),
    /// Manage databases
    #[command(subcommand)]
    Database(DatabaseCommand),
}
