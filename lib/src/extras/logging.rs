use std::fmt;
use std::fs::{self, File};
use std::io;
use std::sync::Arc;

use ansi_term::{Color, Style};
use file_rotate::{compression::Compression, suffix::AppendCount, ContentLimit, FileRotate};
use log::{level_filters::LevelFilter, Event, Level, Subscriber};
use parking_lot::Mutex;
use tracing_log::NormalizeEvent;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields, Layer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
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
    "nimiq_address",
    "nimiq_block",
    "nimiq_block_production",
    "nimiq_blockchain",
    "nimiq_bls",
    "nimiq_bls",
    "nimiq_client",
    "nimiq_collections",
    "nimiq_consensus",
    "nimiq_database",
    "nimiq_devnet",
    "nimiq_genesis",
    "nimiq_genesis_builder",
    "nimiq_handel",
    "nimiq_hash",
    "nimiq_hash_derive",
    "nimiq_key_derivation",
    "nimiq_keys",
    "nimiq_lib",
    "nimiq_macros",
    "nimiq_mempool",
    "nimiq_mmr",
    "nimiq_mnemonic",
    "nimiq_nano_blockchain",
    "nimiq_nano_primitives",
    "nimiq_nano_zkp",
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
    "nimiq_spammer",
    "nimiq_subscription",
    "nimiq_tendermint",
    "nimiq_test_utils",
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

    struct Formatting<T: FormatTime>(T);

    impl<S, N, T: FormatTime> FormatEvent<S, N> for Formatting<T>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        N: for<'a> FormatFields<'a> + 'static,
    {
        fn format_event(
            &self,
            ctx: &FmtContext<'_, S, N>,
            mut writer: Writer<'_>,
            event: &Event<'_>,
        ) -> fmt::Result {
            let (bold, dim) = if writer.has_ansi_escapes() {
                (Style::default().bold(), Style::default().dimmed())
            } else {
                (Style::default(), Style::default())
            };
            write!(&mut writer, "{}", dim.prefix())?;
            self.0.format_time(&mut writer)?;
            write!(&mut writer, "{}", dim.suffix())?;

            // Format values from the event's metadata:
            let normalized_metadata = event.normalized_metadata();
            let metadata = normalized_metadata
                .as_ref()
                .unwrap_or_else(|| event.metadata());

            let color = if writer.has_ansi_escapes() {
                Style::from(match *metadata.level() {
                    Level::TRACE => Color::Purple,
                    Level::DEBUG => Color::Blue,
                    Level::INFO => Color::Green,
                    Level::WARN => Color::Yellow,
                    Level::ERROR => Color::Red,
                })
            } else {
                Style::default()
            };

            let mut target = metadata.target();
            // Drop everything except the module name.
            if let Some(pos) = target.rfind("::") {
                target = &target[pos + 2..];
            }
            // Pretend `target` is ASCII-only
            const MAX_MODULE_WIDTH: usize = 20;
            let truncate = target.len() > MAX_MODULE_WIDTH;
            if truncate {
                for i in (0..MAX_MODULE_WIDTH).rev() {
                    if target.is_char_boundary(i) {
                        target = &target[..i];
                        break;
                    }
                }
            }
            let indicator = if truncate {
                "…"
            } else if target.len() < MAX_MODULE_WIDTH {
                " "
            } else {
                ""
            };

            write!(
                &mut writer,
                " {}{:5}{} {}{:width$}{}{} | ",
                color.prefix(),
                metadata.level(),
                color.suffix(),
                dim.prefix(),
                target,
                indicator,
                dim.suffix(),
                width = MAX_MODULE_WIDTH - 1,
            )?;

            // Write fields on the event
            ctx.field_format().format_fields(writer.by_ref(), event)?;

            // Format all the spans in the event's span context.
            if let Some(scope) = ctx.event_scope() {
                for span in scope.from_root() {
                    write!(
                        writer,
                        ", {}{}{{{}",
                        bold.prefix(),
                        span.name(),
                        bold.suffix()
                    )?;

                    // `FormattedFields` is a formatted representation of the span's
                    // fields, which is stored in its extensions by the `fmt` layer's
                    // `new_span` method. The fields will have been formatted
                    // by the same field formatter that's provided to the event
                    // formatter in the `FmtContext`.
                    let ext = span.extensions();
                    let fields = &ext
                        .get::<FormattedFields<N>>()
                        .expect("will never be `None`");

                    // Skip formatting the fields if the span had no fields.
                    if !fields.is_empty() {
                        write!(writer, "{}{}}}{}", fields, bold.prefix(), bold.suffix())?;
                    }
                }
            }

            writeln!(writer)?;

            Ok(())
        }
    }

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
                .event_format(Formatting(SystemTime))
                .with_filter(
                    Targets::new()
                        .with_default(DEFAULT_LEVEL)
                        .with_nimiq_targets(LevelFilter::TRACE),
                ),
        )
    } else {
        None
    };

    let (gelf_layer, gelf_task) = if let Some(graylog) = &settings.graylog {
        let mut builder = tracing_gelf::Logger::builder();
        for (name, value) in &graylog.extra_fields {
            builder = builder.additional_field(name.clone(), value.clone());
        }
        let (layer, bg_task) = builder.connect_tcp(graylog.address.clone())?;
        let layer = layer.with_filter(
            Targets::new()
                .with_default(DEFAULT_LEVEL)
                .with_nimiq_targets(LevelFilter::TRACE),
        );
        (Some(layer), Some(bg_task))
    } else {
        (None, None)
    };

    tracing_subscriber::registry()
        .with(gelf_layer)
        .with(rotating_layer)
        .with(
            Layer::new()
                .with_writer(out)
                .with_ansi(settings.file.is_none())
                .event_format(Formatting(MaybeSystemTime(settings.timestamps)))
                .with_filter(filter),
        )
        .init();

    // Spawn the graylog task after setting up logging so we still get error messages from the
    // graylog logging itself.
    if let Some(task) = gelf_task {
        tokio::spawn(task);
    }
    Ok(())
}
