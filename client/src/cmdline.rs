use std::collections::HashMap;
use std::str::FromStr;

use clap::{Arg, App, SubCommand, ArgMatches};

use crate::settings::{Network, NodeType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Port,
    Miner,
    Passive,
    RpcPort,
    MetricsPort,
    StatisticsInterval,
    ConsensusType,
    Network
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    hostname: Option<String>,
    port: Option<u16>,
    ssl_cert_file: Option<String>,
    ssl_key_file: Option<String>,
    config_file: Option<String>,
    log_level: Option<String>,
    log_tag: HashMap<String, String>,
    miner_threads: Option<u64>,
    passive: Option<bool>,
    rpc_port: Option<u16>,
    metrics_port: Option<u16>,
    stats_interval: Option<u64>,
    consensus_type: Option<NodeType>,
    wallet_seed: Option<String>,
    wallet_address: Option<String>,
    extra_data: Option<String>,
    network: Option<Network>
}


impl Options {
    fn create_app<'a, 'b>() -> App<'a, 'b> {
        App::new("nimiq")
            .version("0.1.0")
            .about("Nimiq's Rust client")
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
            .arg(Arg::with_name("ssl_cert")
                .long("cert")
                .value_name("SSL_CERT_FILE")
                .help("SSL certificate file for your domain. CN should match HOSTNAME.")
                .takes_value(true))
            .arg(Arg::with_name("ssl_key")
                .long("cert")
                .value_name("SSL_KEY_FILE")
                .help("SSL private key file for your domain.")
                .takes_value(true))
            .arg(Arg::with_name("config")
                .short("s")
                .long("config")
                .value_name("CONFIG")
                .help("Use CONFIG as config file")
                .takes_value(true))
            // Options
            .arg(Arg::with_name("log_level")
                .long("cert")
                .value_name("LEVEL")
                .help("Configure global log level.")
                .possible_values(&["TRACE", "DEBUG", "INFO", "WARN", "ERROR"]))
            .arg(Arg::with_name("log_tag")
                .long("cert")
                .value_name("TAG:LEVEL")
                .help("Configure log level for specific tag.")
                .takes_value(true)
                     .use_delimiter(true))
            .arg(Arg::with_name("miner_threads")
                .long("miner")
                .value_name("THREADS")
                .help("Activate mining on this node with THREADS parallel threads")
                .takes_value(true))
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
            .arg(Arg::with_name("stats_interval")
                .long("statistics")
                .value_name("INTERVAL")
                .help("Output mner statistics every INTERVAL seconds")
                .takes_value(true))
            .arg(Arg::with_name("consensus_type")
                .long("type")
                .value_name("TYPE")
                .help("Configure consensus type, one of full (default), light or nano")
                .possible_values(&["full", "light", "nano"]))
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
            .arg(Arg::with_name("extra_data")
                .long("extra-data")
                .value_name("EXTRA_DATA")
                .help("Extra data to add to every mined block.")
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
            ssl_cert_file: matches.value_of("ssl_cert").map(|s| String::from(s)),
            ssl_key_file: matches.value_of("ssl_key").map(|s| String::from(s)),
            config_file: matches.value_of("config").map(|s| String::from(s)),
            log_level: matches.value_of("log_level").map(|s| String::from(s)),
            log_tag: HashMap::new(), // TODO
            miner_threads: Self::parse_option::<u64>(matches.value_of("miner_threads"), ParseError::Miner)?,
            passive: Self::parse_option::<bool>(matches.value_of("passive"), ParseError::Port)?,
            rpc_port: Self::parse_option::<u16>(matches.value_of("rpc_port"), ParseError::RpcPort)?,
            metrics_port: Self::parse_option::<u16>(matches.value_of("metrics_port"), ParseError::MetricsPort)?,
            stats_interval: Self::parse_option::<u64>(matches.value_of("stats_interval"), ParseError::StatisticsInterval)?,
            consensus_type: Self::parse_option::<NodeType>(matches.value_of("consensus_type"), ParseError::ConsensusType)?,
            // TODO: parse as seed/address: Self::parse_option<Seed>([...])
            wallet_seed: Self::parse_option_string(matches.value_of("wallet_seed")),
            wallet_address: Self::parse_option_string(matches.value_of("wallet_address")),
            extra_data: Self::parse_option_string(matches.value_of("extra_data")),
            network: Self::parse_option::<Network>(matches.value_of("network"), ParseError::Network)?,
        })
    }
}

