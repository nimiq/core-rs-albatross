use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Local;
use colored::Colorize;
use failure::Fail;
use fern::colors::{Color, ColoredLevelConfig};
use fern::Dispatch;
use log::{Level, LevelFilter};

static MAX_MODULE_WIDTH: AtomicUsize = AtomicUsize::new(0);

const NIMIQ_MODULES: [&'static str; 22] = [
    "nimiq_accounts",
    "beserial",
    "nimiq_blockchain",
    "nimiq_client",
    "nimiq_collections",
    "nimiq_consensus",
    "nimiq_database",
    "nimiq_hash",
    "nimiq_key_derivation",
    "nimiq_keys",
    "nimiq_lib",
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
];
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
        let max_width = max_module_width(record.target());
        let target = format!("{: <width$}", record.target(), width=max_width);
        out.finish(format_args!(
            " {level} {target} > {message}",
            target = target.bold(),
            level = colors_level.color(record.level()),
            message = message,
        ));
    })
}

fn pretty_logging_with_timestamps(dispatch: Dispatch, colors_level: ColoredLevelConfig) -> Dispatch {
    dispatch.format(move |out, message, record| {
        let max_width = max_module_width(record.target());
        let target = format!("{: <width$}", record.target(), width=max_width);
        out.finish(format_args!(
            " {timestamp} {level} {target} > {message}",
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

        match show_timestamps {
            true => pretty_logging_with_timestamps(self, colors_level),
            false => pretty_logging(self, colors_level),
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

#[inline]
pub fn force_log_error_cause_chain(mut fail: &Fail, level: Level) {
    force_log!(level, "{}", fail);
    if fail.cause().is_some() {
        force_log!(level, "  caused by");
        while let Some(cause) = fail.cause() {
            force_log!(level, "    {}", cause);
            fail = cause;
        }
    }
}
