use std::env;
use std::fs::{self, File};
use std::io;
#[cfg(tokio_unstable)]
use std::net::ToSocketAddrs;
use std::sync::Arc;

use file_rotate::{compression::Compression, suffix::AppendCount, ContentLimit, FileRotate};
use log::{level_filters::LevelFilter, Level, Subscriber};
use parking_lot::Mutex;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::time::SystemTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

use crate::{
    config::{command_line::CommandLine, config_file::LogSettings},
    error::Error,
};
use nimiq_log::{Formatting, MaybeSystemTime, TargetsExt};

pub const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;

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
    // Set logging level from the environment
    filter = filter.with_env();

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
            tracing_subscriber::fmt::layer()
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

    let (loki_layer, loki_task) = if let Some(loki) = &settings.loki {
        let (layer, bg_task) = tracing_loki::layer(
            loki.url.clone(),
            loki.labels.clone(),
            loki.extra_fields.clone(),
        )?;
        let layer = layer.with_filter(
            Targets::new()
                .with_default(DEFAULT_LEVEL)
                .with_nimiq_targets(LevelFilter::TRACE),
        );
        (Some(layer), Some(bg_task))
    } else {
        (None, None)
    };

    #[cfg(tokio_unstable)]
    fn initialize_tokio_console<S>(bind_address: &str) -> Option<Box<dyn Layer<S> + Send + Sync>>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let addresses = bind_address
            .to_socket_addrs()
            .expect("Unable to resolve tokio console bind address");
        Some(Box::new(
            console_subscriber::ConsoleLayer::builder()
                .with_default_env()
                .server_addr(addresses.as_slice()[0])
                .spawn(),
        ))
    }
    #[cfg(not(tokio_unstable))]
    fn initialize_tokio_console<S>(bind_address: &str) -> Option<Box<dyn Layer<S> + Send + Sync>>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let _ = bind_address;
        log::error!("cannot enable tokio console server, need `RUSTFLAGS=\"--cfg tokio_unstable\"` at compile time.");
        None
    }

    let tokio_console_bind_address =
        settings
            .tokio_console_bind_address
            .or_else(|| match env::var("TOKIO_CONSOLE_BIND") {
                Ok(v) => Some(v),
                Err(env::VarError::NotPresent) => None,
                Err(env::VarError::NotUnicode(_)) => {
                    panic!("non-UTF-8 TOKIO_CONSOLE_BIND variable")
                }
            });

    let tokio_console_layer =
        tokio_console_bind_address.and_then(|addr| initialize_tokio_console(&addr));

    tracing_subscriber::registry()
        .with(loki_layer)
        .with(rotating_layer)
        .with(tokio_console_layer)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(out)
                .with_ansi(settings.file.is_none())
                .event_format(Formatting(MaybeSystemTime(settings.timestamps)))
                .with_filter(filter),
        )
        .init();

    if let Some(task) = loki_task {
        tokio::spawn(task);
    }
    Ok(())
}
