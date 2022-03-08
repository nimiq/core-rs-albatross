use std::fmt;
use std::fs::{self, File};
use std::io;
use std::sync::Arc;

use file_rotate::{compression::Compression, suffix::AppendCount, ContentLimit, FileRotate};
use log::{level_filters::LevelFilter, Level};
use parking_lot::Mutex;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime};
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer as _;

use crate::{
    config::{command_line::CommandLine, config_file::LogSettings},
    error::Error,
};

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

pub const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;

trait TargetsExt {
    fn with_nimiq_targets(self, level: LevelFilter) -> Self;
}

impl TargetsExt for Targets {
    fn with_nimiq_targets(mut self, level: LevelFilter) -> Targets {
        for &module in NIMIQ_MODULES.iter() {
            self = self.with_target(module, level);
        }
        self
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

enum LoggingTarget {
    File(Arc<File>),
    Stderr(io::Stderr),
}

impl io::Write for LoggingTarget {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        use self::LoggingTarget::*;
        match self {
            File(f) => (&**f).write(buf),
            Stderr(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        use self::LoggingTarget::*;
        match self {
            File(f) => (&**f).flush(),
            Stderr(s) => s.flush(),
        }
    }
    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        use self::LoggingTarget::*;
        match self {
            File(f) => (&**f).write_vectored(bufs),
            Stderr(s) => s.write_vectored(bufs),
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
    let mut filter = Targets::new()
        .with_default(DEFAULT_LEVEL)
        .with_nimiq_targets(settings.level.unwrap_or(DEFAULT_LEVEL));
    // Set logging level for specific selected modules
    filter = filter.with_targets(settings.tags);

    let file = match &settings.file {
        Some(filename) => Some(Arc::new(File::open(filename)?)),
        None => None,
    };
    let out = move || -> io::LineWriter<LoggingTarget> {
        io::LineWriter::new(if let Some(file) = &file {
            LoggingTarget::File(file.clone())
        } else {
            LoggingTarget::Stderr(io::stderr())
        })
    };

    struct MaybeSystemTime(bool);

    impl FormatTime for MaybeSystemTime {
        fn format_time(&self, w: &mut Writer) -> fmt::Result {
            if self.0 {
                SystemTime.format_time(w)
            } else {
                ().format_time(w)
            }
        }
    }

    #[derive(Clone)]
    struct Rotating(Arc<Mutex<FileRotate<AppendCount>>>);

    impl io::Write for Rotating {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.lock().flush()
        }
        fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
            self.0.lock().write_vectored(bufs)
        }
    }

    let rotating_layer = if let Some(rotating_file_settings) = settings.rotating_trace_log {
        fs::create_dir_all(&rotating_file_settings.path)?;

        // Create rotating log file according to settings
        let log = Rotating(Arc::new(Mutex::new(FileRotate::new(
            rotating_file_settings.path.join("log.log"),
            AppendCount::new(rotating_file_settings.file_count),
            ContentLimit::Bytes(rotating_file_settings.size),
            Compression::None,
        ))));

        Some(
            Layer::new()
                .with_writer(move || log.clone())
                .with_ansi(false)
                .with_timer(SystemTime)
                .with_filter(
                    Targets::new()
                        .with_default(DEFAULT_LEVEL)
                        .with_nimiq_targets(LevelFilter::TRACE),
                ),
        )
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(rotating_layer)
        .with(
            Layer::new()
                .with_writer(out)
                .with_ansi(settings.file.is_none())
                .with_timer(MaybeSystemTime(settings.timestamps))
                .with_filter(filter),
        )
        .init();

    Ok(())
}
