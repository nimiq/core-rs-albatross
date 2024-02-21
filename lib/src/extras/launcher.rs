use url::Url;

/// # ToDo
///
/// A rocket-like launcher. We can use this to easily:
///
/// * initialize logging
/// * initialize dead-lock detection
/// * handle panics either by logging or human-panic
/// * load config file?
/// * parse command line?
/// * start tokio runtime and pass in the config struct
///

#[cfg(feature = "deadlock")]
use crate::extras::deadlock::initialize_deadlock_detection;
#[cfg(feature = "logging")]
use crate::extras::logging::initialize_logging;
#[cfg(feature = "panic")]
use crate::extras::panic::initialize_panic_reporting;

pub fn go() -> Launcher {
    Launcher::default()
}

// TODO: Move into panic
#[derive(Debug, Default)]
pub struct PanicMode {
    /// Write panics into log
    logging: bool,

    /// Prints panic message with instruction to report and provides a dump file with stack trace.
    _human_panic: bool,

    /// This is only supposed to be used by Team Nimiq for automatically collecting panics.
    ///
    /// # ToDo
    ///
    /// * Have this behind a separate feature. Never enable this by default! Read the report URL
    ///   from an environment variable
    _report_url: Option<Url>,
}

#[derive(Debug, Default)]
pub struct Launcher {
    deadlock_detection: bool,
    logging: bool,
    panic: PanicMode,
}

impl Launcher {
    #[cfg(feature = "deadlock")]
    #[must_use]
    pub fn deadlock_detection(mut self) -> Self {
        self.deadlock_detection = true;
        initialize_deadlock_detection();
        self
    }

    #[cfg(feature = "logging")]
    #[must_use]
    pub fn logging(mut self) -> Self {
        self.logging = true;
        initialize_logging(None, None).unwrap();
        self
    }

    #[cfg(feature = "panic")]
    #[must_use]
    pub fn panic_reporting(mut self) -> Self {
        self.panic = PanicMode::default();
        self.panic.logging = true;
        initialize_panic_reporting();
        self
    }
}
