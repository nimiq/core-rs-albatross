use std::{thread, time::Duration};

use signal_hook::{consts::SIGINT, iterator::Signals};

pub fn initialize_signal_handler() {
    let signals = Signals::new([SIGINT]);

    if let Ok(mut signals) = signals {
        thread::spawn(move || {
            if signals.forever().next().is_some() {
                log::warn!("Received Ctrl+C. Closing client");
                // Add some delay for the log message to propagate into loki
                thread::sleep(Duration::from_millis(200));
                std::process::exit(0);
            }
        });
    } else {
        log::error!("Could not obtain SIGINT signal");
    }
}
