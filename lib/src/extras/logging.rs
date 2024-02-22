#[cfg(all(tokio_unstable, feature = "tokio-console"))]
use std::net::ToSocketAddrs;
use std::{
    env,
    fs::{self, File},
    io,
    sync::Arc,
};

use log::{level_filters::LevelFilter, Level, Subscriber};
use nimiq_log::{Formatting, MaybeSystemTime, TargetsExt};
use tracing_subscriber::{
    filter::Targets, layer::SubscriberExt, registry::LookupSpan, util::SubscriberInitExt, Layer,
};

use crate::{
    config::{command_line::CommandLine, config_file::LogSettings},
    error::Error,
};

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
    // Creating ZKPs with a log level below WARN will consume huge amounts of memory due to tracing annotations in the dependency.
    // That's why we specifically set its log level to WARN.
    let mut filter = Targets::new()
        .with_default(DEFAULT_LEVEL)
        .with_nimiq_targets(settings.level.unwrap_or(DEFAULT_LEVEL))
        .with_target("r1cs", LevelFilter::WARN);
    // Set logging level for specific selected modules
    filter = filter.with_targets(settings.tags);
    // Set logging level from the environment
    filter = filter.with_env();

    let file = match &settings.file {
        Some(filename) => {
            let file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(filename)
                .unwrap();
            Some(Arc::new(file))
        }
        None => None,
    };
    let out = move || -> io::LineWriter<LoggingTarget> {
        io::LineWriter::new(if let Some(file) = &file {
            LoggingTarget::File(file.clone())
        } else {
            LoggingTarget::Stderr(io::stderr())
        })
    };

    #[cfg(feature = "loki")]
    let (loki_layer, loki_task) = if let Some(loki) = &settings.loki {
        let (layer, bg_task) = tracing_loki::layer(
            loki.url.clone(),
            loki.labels.clone(),
            loki.extra_fields.clone(),
        )?;
        let layer = layer.with_filter(
            // Creating ZKPs with a log level below WARN will consume huge amounts of memory due to tracing annotations in the dependency.
            // That's why we specifically set its log level to WARN.
            Targets::new()
                .with_default(DEFAULT_LEVEL)
                .with_nimiq_targets(LevelFilter::TRACE)
                .with_target("r1cs", LevelFilter::WARN),
        );
        (Some(layer), Some(bg_task))
    } else {
        (None, None)
    };

    #[cfg(all(tokio_unstable, feature = "tokio-console"))]
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
    #[cfg(not(all(tokio_unstable, feature = "tokio-console")))]
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

    let mut formatting_layer = tracing_subscriber::fmt::layer().with_writer(out);
    if settings.file.is_some() {
        // Only disable ANSI colors when a log file is specified. Do not forcefully enable ANSI colors otherwise,
        // as they might be disabled through other means (e.g. with the NO_COLOR environment variable).
        formatting_layer = formatting_layer.with_ansi(false);
    }
    let formatting_layer = formatting_layer
        .event_format(Formatting(MaybeSystemTime(settings.timestamps)))
        .with_filter(filter);

    #[cfg(feature = "loki")]
    {
        tracing_subscriber::registry()
            .with(loki_layer)
            .with(tokio_console_layer)
            .with(formatting_layer)
            .init();

        if let Some(task) = loki_task {
            tokio::spawn(task);
        }
    }
    #[cfg(not(feature = "loki"))]
    tracing_subscriber::registry()
        .with(tokio_console_layer)
        .with(formatting_layer)
        .init();
    Ok(())
}
