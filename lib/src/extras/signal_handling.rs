use signal_hook::{consts::SIGINT, iterator::Signals};

pub fn initialize_signal_handler() {
    let signals = Signals::new(&[SIGINT]);

    if let Ok(mut signals) = signals {
        tokio::spawn(async move {
            for _ in signals.forever() {
                log::warn!("Received Ctrl+C. Closing client");
                std::process::exit(0);
            }
        });
    } else {
        log::error!("Could not obtain SIGINT signal");
    }
}
