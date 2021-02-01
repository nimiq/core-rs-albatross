use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Local;
use colored::Colorize;
use fern::colors::{Color, ColoredLevelConfig};
use fern::{log_file, Dispatch};
use lazy_static::lazy_static;
use log::{Level, LevelFilter};

use crate::{
    config::{command_line::CommandLine, config_file::LogSettings},
    error::Error,
};

static MAX_MODULE_WIDTH: AtomicUsize = AtomicUsize::new(20);

lazy_static! {
    static ref NIMIQ_MODULES: Vec<&'static str> = vec![
        "beserial",
        "beserial_derive",
        "nimiq_account",
        "nimiq_accounts",
        "nimiq_address",
        "nimiq_block_albatross",
        "nimiq_block_production_albatross",
        "nimiq_blockchain_albatross",
        "nimiq_bls",
        "nimiq_bls",
        "nimiq_build_tools",
        "nimiq_client",
        "nimiq_collections",
        "nimiq_consensus_albatross",
        "nimiq_database",
        "nimiq_devnet",
        "nimiq_genesis",
        "nimiq_genesis",
        "nimiq_handel",
        "nimiq_hash",
        "nimiq_hash_derive",
        "nimiq_key_derivation",
        "nimiq_keys",
        "nimiq_lib",
        "nimiq_macros",
        "nimiq_mempool",
        "nimiq_messages",
        "nimiq_metrics_server",
        "nimiq_mnemonic",
        "nimiq_nano_sync",
        "nimiq_network_albatross",
        "nimiq_network_interface",
        "nimiq_network_libp2p",
        "nimiq_network_mock",
        "nimiq_peer_address",
        "nimiq_primitives",
        "nimiq_rpc",
        "nimiq_rpc_client",
        "nimiq_rpc_interface",
        "nimiq_rpc_server",
        "nimiq_signtx",
        "nimiq_subscription",
        "nimiq_tendermint",
        "nimiq_tools",
        "nimiq_transaction",
        "nimiq_transaction_builder",
        "nimiq_tree_primitives",
        "nimiq_utils",
        "nimiq_validator",
        "nimiq_validator_network",
        "nimiq_vrf",
        "nimiq_wallet",
    ];
}

pub const DEFAULT_LEVEL: LevelFilter = LevelFilter::Info;

/// Retrieve and set max module width.
fn max_module_width(target: &str) -> usize {
    let mut max_width = MAX_MODULE_WIDTH.load(Ordering::Acquire);
    if max_width < target.len() {
        MAX_MODULE_WIDTH.store(target.len(), Ordering::Release);
        max_width = target.len();
    }
    max_width
}

/// Trait that implements Nimiq specific behavior for fern's Dispatch.
pub trait NimiqDispatch {
    /// Setup logging in pretty_env_logger style.
    fn pretty_logging(self, show_timestamps: bool) -> Self;

    /// Setup nimiq modules log level.
    fn level_for_nimiq(self, level: LevelFilter) -> Self;

    /// Filters out every target not starting with "nimiq".
    /// Note that this excludes beserial and libargon2_sys!
    fn only_nimiq(self) -> Self;
}

fn pretty_logging(dispatch: Dispatch, colors_level: ColoredLevelConfig) -> Dispatch {
    dispatch.format(move |out, message, record| {
        let target_text = record.target().split("::").last().unwrap();
        let max_width = max_module_width(target_text);
        let target = format!("{: <width$}", target_text, width = max_width);
        out.finish(format_args!(
            " {level: <5} {target} | {message}",
            target = target.bold(),
            level = colors_level.color(record.level()),
            message = message,
        ));
    })
}

fn pretty_logging_with_timestamps(dispatch: Dispatch, colors_level: ColoredLevelConfig) -> Dispatch {
    dispatch.format(move |out, message, record| {
        let target_text = record.target().split("::").last().unwrap();
        let max_width = max_module_width(target_text);
        let target = format!("{: <width$}", target_text, width = max_width);
        out.finish(format_args!(
            " {timestamp} {level: <5} {target} | {message}",
            timestamp = Local::now().format("%Y-%m-%d %H:%M:%S"),
            target = target.bold(),
            level = colors_level.color(record.level()),
            message = message,
        ));
    })
}

impl NimiqDispatch for Dispatch {
    fn pretty_logging(self, show_timestamps: bool) -> Self {
        let colors_level = ColoredLevelConfig::new()
            .error(Color::Red)
            .warn(Color::Yellow)
            .info(Color::Green)
            .debug(Color::Blue)
            .trace(Color::Magenta);

        if show_timestamps {
            pretty_logging_with_timestamps(self, colors_level)
        } else {
            pretty_logging(self, colors_level)
        }
    }

    fn level_for_nimiq(self, level: LevelFilter) -> Self {
        let mut builder = self;
        for &module in NIMIQ_MODULES.iter() {
            builder = builder.level_for(module, level);
        }
        builder
    }

    fn only_nimiq(self) -> Self {
        self.filter(|metadata| metadata.target().starts_with("nimiq"))
    }
}

macro_rules! force_log {
    ($lvl:expr, $($arg:tt)+) => ({
        if log::log_enabled!($lvl) {
            log::log!($lvl, $($arg)+);
        } else {
            eprintln!($($arg)+);
        }
    })
}

pub fn log_error_cause_chain<E: std::error::Error>(e: &E) {
    let level = Level::Error;

    force_log!(level, "{}", e);

    if let Some(mut e) = e.source() {
        force_log!(level, "  caused by");
        force_log!(level, "    {}", e);

        while let Some(source) = e.source() {
            force_log!(level, "    {}", source);

            e = source;
        }
    }
}

pub fn initialize_logging(command_line_opt: Option<&CommandLine>, settings_opt: Option<&LogSettings>) -> Result<(), Error> {
    // Get config from config file
    let mut settings = settings_opt.cloned().unwrap_or_default();

    // Override config from command line
    if let Some(command_line) = command_line_opt {
        if let Some(log_level) = command_line.log_level {
            settings.level = Some(log_level);
        }
        if let Some(log_tags) = &command_line.log_tags {
            settings.tags.extend(log_tags.clone());
        }
    }

    // Set logging level for Nimiq and all other modules
    let mut dispatch = Dispatch::new()
        .pretty_logging(settings.timestamps)
        .level(DEFAULT_LEVEL)
        .level_for_nimiq(settings.level.unwrap_or(DEFAULT_LEVEL));

    // Set logging level for specific selected modules
    for (module, level) in &settings.tags {
        dispatch = dispatch.level_for(module.clone(), *level);
    }

    // Log into file or to stderr
    if let Some(ref filename) = settings.file {
        dispatch = dispatch.chain(log_file(filename)?);
    } else {
        dispatch = dispatch.chain(std::io::stderr());
    }

    dispatch.apply()?;
    Ok(())
}
