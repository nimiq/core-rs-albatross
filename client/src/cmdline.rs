use std::collections::HashMap;
use std::str::FromStr;

use log::LevelFilter;
use clap::{Arg, App, Values};
use failure::Fail;

use crate::settings::{Network, NodeType};


#[derive(Debug, Fail)]
pub(crate) enum ParseError {
    #[fail(display = "Failed to parse port.")]
    Port,
    #[fail(display = "Failed to parse consensus type.")]
    ConsensusType,
    #[fail(display = "Failed to parse network ID.")]
    Network,
    #[fail(display = "Failed to parse log tag.")]
    LogTag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub config_file: Option<String>,
    pub log_level: Option<String>,
    pub log_tags: HashMap<String, LevelFilter>,
    pub passive: bool,
    pub consensus_type: Option<NodeType>,
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
            .arg(Arg::with_name("consensus_type")
                .long("type")
                .value_name("TYPE")
                .help("Configure consensus type, one of full (default), light or nano")
                .possible_values(&["full", "light", "nano"])
                .case_insensitive(true))
            .arg(Arg::with_name("network")
                .long("network")
                .value_name("NAME")
                .help("Configure the network to connect to, one of main (default), test or dev.")
                .possible_values(&["main", "test", "dev"]))
    }

    /// Parses a command line option from a string into `T` and returns `error`, when parsing fails.
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
        value.map(String::from)
    }

    fn parse_log_tags(values_opt: Option<Values>) -> Result<HashMap<String, LevelFilter>, ParseError> {
        let mut tags: HashMap<String, LevelFilter> = HashMap::new();
        if let Some(values) = values_opt {
            for value in values {
                let split = value.split(':').collect::<Vec<&str>>();
                if split.len() != 2 {
                    return Err(ParseError::LogTag);
                }
                tags.insert(String::from(split[0]), LevelFilter::from_str(split[1])
                    .map_err(|_| ParseError::LogTag)?);
            }
        }
        Ok(tags)
    }

    pub fn parse() -> Result<Options, ParseError> {
        let app = Self::create_app();
        let matches = app.get_matches();

        Ok(Options {
            hostname: Self::parse_option_string(matches.value_of("hostname")),
            port: Self::parse_option::<u16>(matches.value_of("port"), ParseError::Port)?,
            config_file: matches.value_of("config").map(String::from),
            log_level: matches.value_of("log_level").map(String::from),
            log_tags: Self::parse_log_tags(matches.values_of("log_tags"))?,
            passive: matches.is_present("passive"),
            consensus_type: Self::parse_option::<NodeType>(matches.value_of("consensus_type"), ParseError::ConsensusType)?,
            network: Self::parse_option::<Network>(matches.value_of("network"), ParseError::Network)?,
        })
    }
}

