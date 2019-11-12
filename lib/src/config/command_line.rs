use std::path::PathBuf;

use failure::Fail;
use structopt::StructOpt;
use log::{LevelFilter, ParseLevelError};

use network_primitives::networks::NetworkId;

use crate::config::config_file::ConsensusType;

/*lazy_static! {
    static ref VALID_LOG_LEVELS: [&'static str; 6] = ["off", "error", "warn", "info", "debug", "trace"];
    static ref VALID_CONSENSUS_TYPES: [&'static str; 2] = ["full", "macro-sync"];
}*/


#[derive(Debug, StructOpt)]
#[structopt(rename_all="kebab")]
pub struct CommandLine {
    /// Hostname of this Nimiq client.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --hostname seed1.nimiq.dev`
    ///
    pub hostname: Option<String>,

    /// Port to listen on for connections
    ///
    /// # Examples
    ///
    /// * `nimiq-client --port 8000`
    ///
    pub port: Option<u16>,

    /// Use a custom configuration file.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --config ~/.nimiq/client-albatross.toml`
    ///
    pub config: Option<PathBuf>,

    /// Configure global log level.
    ///
    /// # Examples
    ///
    /// * `nimiq-client --log-level trace`
    ///
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
    pub passive: bool,

    /// Configure consensus type, one of full (default), or macro-sync
    ///
    /// # Examples
    ///
    /// * `nimiq-client --type macro-sync`
    ///
    #[structopt(long="type", parse(try_from_str))]
    pub consensus_type: Option<ConsensusType>,

    /// Configure the network to connect to, one of test-albatross, dev-albatross (default)
    ///
    /// # Examples
    ///
    /// * `nimiq-client --type test-albatross
    ///
    pub network: Option<NetworkId>,
}

impl CommandLine {
    pub fn from_args() -> Self {
        <Self as StructOpt>::from_args()
    }

    /// Load command line from command line arguments (std::env::args)
    pub fn from_iter<I: IntoIterator<Item=String>>(args: I) -> Self {
        <Self as StructOpt>::from_iter(args)
    }
}


#[derive(Debug, Fail)]
pub enum LogTagParseError {
    #[fail(display = "Log tag is missing seperator: {}", _0)]
    MissingColon(String),
    #[fail(display = "Invalid log level: {}", _0)]
    InvalidLogLevel(#[cause] ParseLevelError),
}

fn parse_log_tags(s: &str) -> Result<(String, LevelFilter), LogTagParseError> {
    let p: Vec<&str> = s.splitn(1, ":").collect();
    let tag = p.get(0).unwrap().to_string();
    let level = p.get(1)
        .ok_or_else(|| LogTagParseError::MissingColon(s.to_string()))?
        .parse()
        .map_err(|e| LogTagParseError::InvalidLogLevel(e))?;
    Ok((tag, level))
}
