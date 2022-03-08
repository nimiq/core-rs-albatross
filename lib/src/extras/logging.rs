use std::sync::atomic::{AtomicUsize, Ordering};

use actual_log::LevelFilter;
use colored::Colorize;
use fern::colors::{Color, ColoredLevelConfig};
use fern::{log_file, Dispatch};
use file_rotate::{compression::Compression, suffix::AppendCount, ContentLimit, FileRotate};
use log::Level;
use time::OffsetDateTime;

use crate::{
    config::{command_line::CommandLine, config_file::LogSettings},
    error::Error,
};

static MAX_MODULE_WIDTH: AtomicUsize = AtomicUsize::new(20);

static NIMIQ_MODULES: &'static [&'static str] = &[
    "beserial",
    "beserial_derive",
    "nimiq_account",
    "nimiq_accounts",
    "nimiq_address",
    "nimiq_block",
    "nimiq_block_production",
    "nimiq_blockchain",
    "nimiq_bls",
    "nimiq_bls",
    "nimiq_build_tools",
    "nimiq_client",
    "nimiq_collections",
    "nimiq_consensus",
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
    "nimiq_nano_zkp",
    "nimiq_network",
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
    "nimiq_trie",
    "nimiq_utils",
    "nimiq_validator",
    "nimiq_validator_network",
    "nimiq_vrf",
    "nimiq_wallet",
];

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
    #[must_use]
    fn pretty_logging(self, show_timestamps: bool, formatted: bool) -> Self;

    /// Setup nimiq modules log level.
    #[must_use]
    fn level_for_nimiq(self, level: LevelFilter) -> Self;

    /// Filters out every target not starting with "nimiq".
    /// Note that this excludes beserial and libargon2_sys!
    #[must_use]
    fn only_nimiq(self) -> Self;
}

fn pretty_logging(
    dispatch: Dispatch,
    colors_level: ColoredLevelConfig,
    formatted: bool,
) -> Dispatch {
    dispatch.format(move |out, message, record| {
        let target_text = record.target().split("::").last().unwrap();
        let max_width = max_module_width(target_text);
        let target = format!("{: <width$}", target_text, width = max_width);

        if formatted {
            out.finish(format_args!(
                " {level: <5} {target} | {message}",
                target = target.bold(),
                level = colors_level.color(record.level()),
                message = message,
            ));
        } else {
            out.finish(format_args!(
                " {level: <5} {target} | {message}",
                target = target,
                level = record.level(),
                message = message,
            ));
        }
    })
}

fn pretty_logging_with_timestamps(
    dispatch: Dispatch,
    colors_level: ColoredLevelConfig,
    formatted: bool,
) -> Dispatch {
    dispatch.format(move |out, message, record| {
        let target_text = record.target().split("::").last().unwrap();
        let max_width = max_module_width(target_text);
        let target = format!("{: <width$}", target_text, width = max_width);
        let ts_format = time::format_description::parse(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]",
        )
        .unwrap();

        if formatted {
            out.finish(format_args!(
                " {timestamp} {level: <5} {target} | {message}",
                timestamp = OffsetDateTime::now_utc().format(&ts_format).unwrap(),
                target = target.bold(),
                level = colors_level.color(record.level()),
                message = message,
            ));
        } else {
            out.finish(format_args!(
                " {timestamp} {level: <5} {target} | {message}",
                timestamp = OffsetDateTime::now_utc().format(&ts_format).unwrap(),
                target = target,
                level = record.level(),
                message = message,
            ));
        }
    })
}

impl NimiqDispatch for Dispatch {
    fn pretty_logging(self, show_timestamps: bool, formatted: bool) -> Self {
        let colors_level = ColoredLevelConfig::new()
            .error(Color::Red)
            .warn(Color::Yellow)
            .info(Color::Green)
            .debug(Color::Blue)
            .trace(Color::Magenta);

        if show_timestamps {
            pretty_logging_with_timestamps(self, colors_level, formatted)
        } else {
            pretty_logging(self, colors_level, formatted)
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
        if log::enabled!($lvl) {
            log::event!($lvl, $($arg)+);
        } else {
            eprintln!($($arg)+);
        }
    })
}

pub fn log_error_cause_chain<E: std::error::Error>(e: &E) {
    force_log!(Level::ERROR, "{}", e);

    if let Some(mut e) = e.source() {
        force_log!(Level::ERROR, "  caused by");
        force_log!(Level::ERROR, "    {}", e);

        while let Some(source) = e.source() {
            force_log!(Level::ERROR, "    {}", source);

            e = source;
        }
    }
}

pub fn initialize_logging(
    command_line_opt: Option<&CommandLine>,
    settings_opt: Option<&LogSettings>,
) -> Result<(), Error> {
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
        // Do not format (colors, bold components) for file output
        .pretty_logging(settings.timestamps, settings.file.is_none())
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

    if let Some(rotating_file_settings) = settings.rotating_trace_log {
        std::fs::create_dir_all(rotating_file_settings.path.clone())?;

        // Create rotating log file according to settings
        let log = FileRotate::new(
            rotating_file_settings.path.join("log.log"),
            AppendCount::new(rotating_file_settings.file_count),
            ContentLimit::Bytes(rotating_file_settings.size),
            Compression::None,
        );

        dispatch = Dispatch::new().chain(dispatch);

        dispatch = dispatch.chain(
            Dispatch::new()
                .pretty_logging(true, false) // always log with timestamps and without formatting
                .level(DEFAULT_LEVEL)
                .level_for_nimiq(LevelFilter::Trace)
                .chain(Box::new(log) as Box<dyn std::io::Write + Send>),
        );
    }

    dispatch.apply()?;

    Ok(())
}
