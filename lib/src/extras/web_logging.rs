use log::{level_filters::LevelFilter, Level};
use nimiq_log::{Formatting, MaybeSystemTime, TargetsExt};
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt, Layer};
use tracing_web::MakeWebConsoleWriter;

use crate::{config::config_file::LogSettings, error::Error};

pub const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;

macro_rules! force_log {
    ($lvl:expr, $($arg:tt)+) => ({
        if log::enabled!($lvl) {
            log::event!($lvl, $($arg)+);
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

pub fn initialize_web_logging(settings_opt: Option<&LogSettings>) -> Result<(), Error> {
    // Get config from config file
    let settings = settings_opt.cloned().unwrap_or_default();

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

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(MakeWebConsoleWriter::new())
                .with_ansi(false)
                .event_format(Formatting(MaybeSystemTime(settings.timestamps)))
                .with_filter(filter),
        )
        .init();
    Ok(())
}
