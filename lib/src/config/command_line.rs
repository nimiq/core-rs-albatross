use std::path::PathBuf;

use clap::Parser;
use log::level_filters::{LevelFilter, ParseLevelFilterError};
use nimiq_primitives::networks::NetworkId;
use thiserror::Error;

use crate::config::config_file::SyncMode;

#[derive(Debug, Parser)]
pub struct CommandLine {
    /// Use a custom configuration file.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --config ~/.nimiq/client-albatross.toml`
    ///
    #[clap(long, short = 'c')]
    pub config: Option<PathBuf>,

    /// Configure global log level, one of error, warn, info (default), debug, trace.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --log-level trace`
    ///
    #[clap(long)]
    pub log_level: Option<LevelFilter>,

    /// Configure log level for specific tag.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --log-tags nimiq-handel:trace --log-tags nimiq-validator:trace`
    ///
    #[clap(long = "log-tags", value_parser = parse_log_tags)]
    pub log_tags: Option<Vec<(String, LevelFilter)>>,

    /// Do not actively connect to the network
    ///
    /// # Notes
    ///
    /// * This is currently not implemented.
    /// * This might be removed in the future.
    ///
    #[clap(long)]
    pub passive: bool,

    /// Configure sync mode, one of history (default), full, light.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --mode history`
    ///
    #[clap(long = "mode", value_parser)]
    pub sync_mode: Option<SyncMode>,

    /// Configure the network to connect to, one of main-albatross, test-albatross (default), dev-albatross.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --network test-albatross
    ///
    #[clap(long)]
    pub network: Option<NetworkId>,

    /// Internally used flag to start a zero-knowledge prover process.
    #[clap(long, action)]
    pub prove: bool,
}

impl CommandLine {
    pub fn parse() -> CommandLine {
        Parser::parse()
    }
}

#[derive(Debug, Error)]
pub enum LogTagParseError {
    #[error("Log tag is missing separator: {0}")]
    MissingColon(String),
    #[error("Invalid log level: {0}")]
    InvalidLogLevel(#[from] ParseLevelFilterError),
}

fn parse_log_tags(s: &str) -> Result<(String, LevelFilter), LogTagParseError> {
    let p: Vec<&str> = s.splitn(2, ':').collect();
    let tag = p.first().unwrap().to_string();
    let level = p
        .get(1)
        .ok_or_else(|| LogTagParseError::MissingColon(s.to_string()))?
        .parse()
        .map_err(LogTagParseError::InvalidLogLevel)?;
    Ok((tag, level))
}
