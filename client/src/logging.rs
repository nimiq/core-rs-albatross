use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

use colored::Colorize;
use fern::colors::{Color, ColoredLevelConfig};
use fern::Dispatch;
use log::LevelFilter;
use log::ParseLevelError;

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

/// Convert a &str into a LevelFilter or use default.
pub fn to_level(level: &str) -> Result<LevelFilter, ParseLevelError> {
    LevelFilter::from_str(level)
}

/// Trait that implements Nimiq specific behavior for fern's Dispatch.
pub trait NimiqDispatch {
    /// Setup logging in pretty_env_logger style.
    fn pretty_logging(self) -> Self;

    /// Setup nimiq modules log level.
    fn level_for_nimiq(self, level: LevelFilter) -> Self;

    /// Filters out every target not starting with "nimiq".
    /// Note that this excludes beserial and libargon2_sys!
    fn only_nimiq(self) -> Self;
}

impl NimiqDispatch for Dispatch {
    fn pretty_logging(self) -> Self {
        let colors_level = ColoredLevelConfig::new()
            .error(Color::Red)
            .warn(Color::Yellow)
            .info(Color::Green)
            .debug(Color::Blue)
            .trace(Color::Magenta);

        self.format(move |out, message, record| {
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
