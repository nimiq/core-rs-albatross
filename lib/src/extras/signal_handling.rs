use signal_hook::{consts::SIGINT, iterator::Signals};
use tokio::time::{sleep, Duration};

pub fn initialize_signal_handler() {
    let signals = Signals::new([SIGINT]);

    if let Ok(mut signals) = signals {
        tokio::spawn(async move {
            if signals.forever().next().is_some() {
                log::warn!("Received Ctrl+C. Closing client");
                // Add some delay for the log message to propagate into loki
                sleep(Duration::from_millis(200)).await;
                std::process::exit(0);
            }
        });
    } else {
        log::error!("Could not obtain SIGINT signal");
    }
}
