use std::convert::TryFrom;
use log::LevelFilter;

use nimiq_primitives::networks::NetworkId;
use nimiq_lib::config::config_file::{ConfigFile, Network};
use nimiq_lib::config::config::ClientConfig;
use nimiq_lib::client::Client;
use std::thread;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Spawns the supplied closure in a thread and panics
/// if it doesn't finish within the specified duration.
/// The spawned thread does not get cleaned up.
fn assert_max_runtime<F>(timeout: Duration, func: F)
    where F: FnOnce() -> () + Send + 'static,
{
    struct State {
        thread: Mutex<bool>, // is thread running?
        spawn: Condvar, // spawn notification
        finish: Condvar, // finish notification
    }
    let state = Arc::new(State {
        thread: Mutex::new(false),
        spawn: Condvar::new(),
        finish: Condvar::new(),
    });
    let state2 = Arc::clone(&state);

    // Spawn closure in a thread
    thread::spawn(move || {
        let state = &*state2;
        // Mark thread as spawned
        {
            let mut thread_guard = state.thread.lock().unwrap();
            *thread_guard = true;
            state.spawn.notify_one();
        }
        // Execute user function
        func();
        state.finish.notify_one();
    });

    // Wait for thread to start
    let state = &*state;
    let mut thread_guard = match state.thread.lock() {
        Ok(guard) => guard,
        Err(_) => panic!("Test closure panicked!"),
    };
    loop {
        if *thread_guard {
            // Thread marked start
            break
        }
        thread_guard = state.spawn.wait(thread_guard)
            .expect("Thread closure panicked");
    }

    // Wait for thread to finish
    let (_, timeout) = state.finish
        .wait_timeout(thread_guard, timeout)
        .unwrap();
    if timeout.timed_out() {
        panic!("Test closure timed out");
        // TODO Kill closure thread
    }
}

#[test]
fn client_can_start_and_stop() {
    std::env::set_var("RUST_LOG", "TRACE");
    env_logger::init();

    let host = "localhost.localdomain".to_string();
    let port = 40237u16;

    let mut config_file = ConfigFile::from_str("").unwrap();
    config_file.network.host = Some(host.clone());
    config_file.network.port = Some(port);
    config_file.log.level = Some(LevelFilter::Trace);
    config_file.consensus.network = Network::UnitAlbatross;
    config_file.network.instant_inbound = Some(true);

    #[cfg(feature = "deadlock")]
    let deadlock_detector = nimiq_lib::extras::deadlock::DeadlockDetector::new();

    let mut builder = ClientConfig::builder();
    builder.network(NetworkId::UnitAlbatross);
    builder.config_file(&config_file).unwrap();
    builder.volatile();
    builder.ws(host, port);
    let config = builder.build().unwrap();

    log::info!("Created config");

    let timeout = Duration::from_secs(3);
    assert_max_runtime(timeout, move || {
        tokio_compat::runtime::current_thread::run_std(async move {
            let client = Client::try_from(config).unwrap();
            client.initialize().unwrap();
            log::info!("Initialized client");
            client.connect().unwrap();
            log::info!("Connected client");
            client.disconnect();
            log::info!("Disconnected client");
            drop(client);
        });
    });

    log::info!("Finished run");

    #[cfg(feature = "deadlock")]
    drop(deadlock_detector);
}
