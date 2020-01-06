
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
use crate::extras::deadlock::DeadlockDetector;
#[cfg(feature = "logging")]
use crate::extras::logging::initialize_logging;
use url::Url;
use lazy_static::lazy_static;
#[cfg(feature = "panic")]
extern crate log_panics;


pub fn go() -> Launcher {
    Launcher::default()
}


// TODO: Move into panic
#[derive(Debug, Default)]
pub struct PanicMode {
    /// Write panics into log
    logging: bool,

    /// Prints panic message with instruction to report and provides a dump file with stack trace.
    human_panic: bool,

    /// This is only supposed to be used by Team Nimiq for automatically collecting panics.
    ///
    /// # ToDo
    ///
    /// * Have this behind a separate feature. Never enable this by default! Read the report URL
    ///   from an environment variable
    report_url: Option<Url>,
}

#[derive(Debug, Default)]
pub struct Launcher {
    deadlock_detection: bool,
    logging: bool,
    panic: PanicMode,
}

// TODO Unused and ugly
lazy_static! {
    static ref GLOBAL_DEADLOCK_DETECTOR: Mutex<Option<DeadlockDetector>> = Mutex::new(None);
}

impl Launcher {
    #[cfg(feature = "deadlock")]
    pub fn deadlock_detection(mut self) -> Self {
        self.deadlock_detection = true;
        *GLOBAL_DEADLOCK_DETECTOR.lock() = DeadlockDetector::new();
        self
    }

    #[cfg(feature = "logging")]
    pub fn logging(mut self) -> Self {
        self.logging = true;
        initialize_logging(None, None).unwrap();
        self
    }

    #[cfg(fature = "panic")]
    pub fn panic_reporting(mut self) -> Self {
        self.panic = PanicMode::default();
        self.panic.logging = true;
        initialize_panic_reporting();
    }
}
