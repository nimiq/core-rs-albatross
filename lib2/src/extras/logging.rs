use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Local;
use colored::Colorize;
use failure::Fail;
use fern::colors::{Color, ColoredLevelConfig};
use fern::{Dispatch, log_file};
use log::{Level, LevelFilter, SetLoggerError};

use crate::error::Error;

static MAX_MODULE_WIDTH: AtomicUsize = AtomicUsize::new(20);

lazy_static! {
    static ref NIMIQ_MODULES: Vec<&'static str> = vec![
        "nimiq_accounts",
        "beserial",
        "nimiq_bls",
        "nimiq_blockchain",
        "nimiq_blockchain_albatross",
        "nimiq_block_production",
        "nimiq_block_production_albatross",
        "nimiq_block",
        "nimiq_block_albatross",
        "nimiq_block_base",
        "nimiq_account",
        "nimiq_transaction",
        "nimiq_client",
        "nimiq_client2",
        "nimiq_collections",
        "nimiq_consensus",
        "nimiq_database",
        "nimiq_hash",
        "nimiq_key_derivation",
        "nimiq_keys",
        "nimiq_lib",
        "nimiq_lib2",
        "libargon2_sys",
        "nimiq_macros",
        "nimiq_mempool",
        "nimiq_messages",
        "nimiq_metrics_server",
        "nimiq_mnemonic",
        "nimiq_network",
        "nimiq_network_primitives",
        "nimiq_primitives",
        "nimiq_rpc_server",
        "nimiq_utils",
        "nimiq_validator",
        "nimiq_handel",
    ];
}

pub const DEFAULT_LEVEL: LevelFilter = LevelFilter::Warn;

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
        let target = format!("{: <width$}", target_text, width=max_width);
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
        let target = format!("{: <width$}", target_text, width=max_width);
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
        if log_enabled!($lvl) {
            log!($lvl, $($arg)+);
        } else {
            eprintln!($($arg)+);
        }
    })
}

pub fn log_error_cause_chain(mut fail: &dyn Fail) {
    let level = Level::Error;
    force_log!(level, "{}", fail);
    if fail.cause().is_some() {
        force_log!(level, "  caused by");
        while let Some(cause) = fail.cause() {
            force_log!(level, "    {}", cause);
            fail = cause;
        }
    }
}



pub fn initialize_logging() -> Result<(), Error> {
    // TODO: Take from config/commandline that we loaded.
    // Create a LoggingConfig in logging module that works like the ClientConfig and can
    // be configure from the ConfigFile and CommandLine structs.

    use crate::config::config_file::LogSettings;
    let mut config = LogSettings::default();
    // For now lets just use debug
    config.level = Some(LevelFilter::Debug);

    let mut dispatch = Dispatch::new()
        .pretty_logging(config.timestamps)
        .level(DEFAULT_LEVEL)
        .level_for_nimiq(config.level
            .unwrap_or(DEFAULT_LEVEL));

    for (module, level) in &config.tags {
        dispatch = dispatch.level_for(module.clone(), level.clone());
    }

    if let Some(ref filename) = config.file {
        dispatch = dispatch.chain(log_file(filename)?);
    }
    else {
        dispatch = dispatch.chain(std::io::stderr());
    }

    // TODO: Return LoggingError
    dispatch.apply()?;

    Ok(())
}

impl From<SetLoggerError> for Error {
    fn from(e: SetLoggerError) -> Self {
        Error::config_error(format!("Failed to set logger: {:?}", e))
    }
}
