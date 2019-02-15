use std::collections::HashMap;
use std::str::FromStr;
use std::fmt;
use std::error::Error;

use clap::{Arg, App};

use crate::settings::{Network, NodeType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParseError {
    Port,
    Passive,
    RpcPort,
    MetricsPort,
    ConsensusType,
    Network
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.description(), match self {
            ParseError::Port => "port",
            ParseError::Passive => "passive",
            ParseError::RpcPort => "rpc",
            ParseError::MetricsPort => "metrics",
            ParseError::ConsensusType => "type",
            ParseError::Network => "network"
        })
    }
}

impl Error for ParseError {
    fn description(&self) -> &str {
        return "Can't parse command-line option";
    }

    fn cause(&self) -> Option<&Error> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub config_file: Option<String>,
    pub log_level: Option<String>,
    pub log_tags: HashMap<String, String>,
    pub passive: Option<bool>,
    pub rpc_port: Option<u16>,
    pub metrics_port: Option<u16>,
    pub consensus_type: Option<NodeType>,
    pub wallet_seed: Option<String>,
    pub wallet_address: Option<String>,
    pub network: Option<Network>
}


impl Options {
    fn create_app<'a, 'b>() -> App<'a, 'b> {
        App::new("nimiq")
            .version("0.1.0")
            .about("Nimiq's Rust client")
            .author("The Nimiq Core Development Team <info@nimiq.com>")
            // Configuration
            .arg(Arg::with_name("hostname")
                .long("host")
                .value_name("HOSTNAME")
                .help("Hostname of this Nimiq client.")
                .takes_value(true))
            .arg(Arg::with_name("port")
                .long("port")
                .value_name("PORT")
                .help("Port to listen on for connections.")
                .takes_value(true))
            .arg(Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("CONFIG")
                .help("Use CONFIG as config file")
                .takes_value(true))
            // Options
            .arg(Arg::with_name("log_level")
                .long("log")
                .value_name("LEVEL")
                .help("Configure global log level.")
                .possible_values(&["error", "warn", "info", "debug", "trace"])
                .case_insensitive(true))
            .arg(Arg::with_name("log_tags")
                .long("log-tag")
                .value_name("TAG:LEVEL")
                .help("Configure log level for specific tag.")
                .takes_value(true)
                     .use_delimiter(true))
            .arg(Arg::with_name("passive")
                .long("passive")
                .help("Do not actively connect to the network.")
                .takes_value(false))
            .arg(Arg::with_name("rpc_port")
                .long("rpc")
                .value_name("PORT")
                .help("Start JSON-RPC server on PORT (default: 8648).")
                .takes_value(true))
            .arg(Arg::with_name("metrics_port")
                .long("metrics")
                .value_name("PORT")
                .help("Start Prometheus-compatible metrics server on PORT (default: 8649).")
                .takes_value(true))
            .arg(Arg::with_name("consensus_type")
                .long("type")
                .value_name("TYPE")
                .help("Configure consensus type, one of full (default), light or nano")
                .possible_values(&["full", "light", "nano"])
                .case_insensitive(true))
            .arg(Arg::with_name("wallet_seed")
                .long("wallet-seed")
                .value_name("SEED")
                .help("Initialize wallet using SEED as a wallet seed.")
                .takes_value(true))
            .arg(Arg::with_name("wallet_address")
                .long("wallet-address")
                .value_name("ADDRESS")
                .help("Initialize wallet using ADDRESS as a wallet address.")
                .takes_value(true))
            .arg(Arg::with_name("network")
                .long("network")
                .value_name("NAME")
                .help("Configure the network to connect to, one of main (default), test or dev.")
                .possible_values(&["main", "test", "dev"]))
    }

    /// Parses a command line option from a string into `T` and returns `error`, when parsing fails.
    /// TODO: Use failure here, to display the cause to the user.
    fn parse_option<T: FromStr>(value: Option<&str>, error: ParseError) -> Result<Option<T>, ParseError> {
        match value {
            None => Ok(None),
            Some(s) => match T::from_str(s.trim()) {
                Err(_) => Err(error), // type of _: <T as FromStr>::Err
                Ok(v) => Ok(Some(v))
            }
        }
    }

    fn parse_option_string(value: Option<&str>) -> Option<String> {
        value.map(|s| String::from(s))
    }

    pub fn parse() -> Result<Options, ParseError> {
        let app = Self::create_app();
        let matches = app.get_matches();

        Ok(Options {
            hostname: Self::parse_option_string(matches.value_of("hostname")),
            port: Self::parse_option::<u16>(matches.value_of("port"), ParseError::Port)?,
            config_file: matches.value_of("config").map(|s| String::from(s)),
            log_level: matches.value_of("log_level").map(|s| String::from(s)),
            log_tags: HashMap::new(), // TODO
            passive: Self::parse_option::<bool>(matches.value_of("passive"), ParseError::Passive)?,
            rpc_port: Self::parse_option::<u16>(matches.value_of("rpc_port"), ParseError::RpcPort)?,
            metrics_port: Self::parse_option::<u16>(matches.value_of("metrics_port"), ParseError::MetricsPort)?,
            consensus_type: Self::parse_option::<NodeType>(matches.value_of("consensus_type"), ParseError::ConsensusType)?,
            // TODO: parse as seed/address: Self::parse_option<Seed>([...])
            wallet_seed: Self::parse_option_string(matches.value_of("wallet_seed")),
            wallet_address: Self::parse_option_string(matches.value_of("wallet_address")),
            network: Self::parse_option::<Network>(matches.value_of("network"), ParseError::Network)?,
        })
    }
}

