// TODO
//
// #[cfg(not(feature = "human-panic"))]
// log_panics::init();
//
// #[cfg(feature = "human-panic")]
// #[macro_use]
// extern crate human_panic;

pub fn initialize_panic_reporting() {
    log_panics::init();
}
