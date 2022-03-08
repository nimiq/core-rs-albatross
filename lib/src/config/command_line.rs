use std::path::PathBuf;

use actual_log::{LevelFilter, ParseLevelError};
use structopt::StructOpt;
use thiserror::Error;

use nimiq_primitives::networks::NetworkId;

use crate::config::config_file::SyncMode;

/*
static VALID_LOG_LEVELS: [&'static str; 6] = ["off", "error", "warn", "info", "debug", "trace"];
static VALID_CONSENSUS_TYPES: [&'static str; 2] = ["full", "macro-sync"];
*/

#[derive(Debug, StructOpt)]
#[structopt(rename_all = "kebab")]
pub struct CommandLine {
    /// Use a custom configuration file.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --config ~/.nimiq/client-albatross.toml`
    ///
    #[structopt(long, short = "c")]
    pub config: Option<PathBuf>,

    /// Configure global log level.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --log-level trace`
    ///
    #[structopt(long)]
    pub log_level: Option<LevelFilter>,

    /// Configure log level for specific tag.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --log-tags nimiq-handel:trace --log-tags nimiq-validator:trace`
    ///
    #[structopt(long = "log-tags", parse(try_from_str = parse_log_tags))]
    pub log_tags: Option<Vec<(String, LevelFilter)>>,

    /// Do not actively connect to the network
    ///
    /// # Notes
    ///
    /// * This is currently not implemented.
    /// * This might be removed in the future.
    ///
    #[structopt(long)]
    pub passive: bool,

    /// Configure sync mode, one of history (default)
    ///
    /// # Examples
    ///
    /// * `nimiq-client --mode history`
    ///
    #[structopt(long = "mode", parse(try_from_str))]
    pub sync_mode: Option<SyncMode>,

    /// Configure the network to connect to, one of test-albatross, dev-albatross (default)
    ///
    /// # Examples
    ///
    /// * `nimiq-client --network test-albatross
    ///
    #[structopt(long)]
    pub network: Option<NetworkId>,
}

impl CommandLine {
    pub fn from_args() -> Self {
        <Self as StructOpt>::from_args()
    }
}

impl FromIterator<String> for CommandLine {
    /// Load command line from command line arguments (std::env::args)
    fn from_iter<I: IntoIterator<Item = String>>(args: I) -> Self {
        <Self as StructOpt>::from_iter(args)
    }
}

#[derive(Debug, Error)]
pub enum LogTagParseError {
    #[error("Log tag is missing separator: {0}")]
    MissingColon(String),
    #[error("Invalid log level: {0}")]
    InvalidLogLevel(#[from] ParseLevelError),
}

fn parse_log_tags(s: &str) -> Result<(String, LevelFilter), LogTagParseError> {
    let p: Vec<&str> = s.splitn(2, ':').collect();
    let tag = p.get(0).unwrap().to_string();
    let level = p
        .get(1)
        .ok_or_else(|| LogTagParseError::MissingColon(s.to_string()))?
        .parse()
        .map_err(LogTagParseError::InvalidLogLevel)?;
    Ok((tag, level))
}
