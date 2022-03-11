#[macro_use]
extern crate log;

use std::fs::{canonicalize, remove_file};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Error;
use structopt::StructOpt;

use docker::Docker;

mod docker;

#[derive(Debug, StructOpt)]
#[structopt(about = "Run an Albatross DevNet locally")]
struct Args {
    #[structopt(parse(from_os_str))]
    /// Path to the environment. Have a look at `devnet-environments`.
    env: PathBuf,
}

#[shellfn::shell]
// NOTE: env_dir needs to be absolute
fn build_client<S: ToString>(
    env_dir: S,
) -> Result<impl Iterator<Item = Result<String, Error>>, Error> {
    r#"
    mkdir -p $ENV_DIR/build/
    export NIMIQ_OVERRIDE_DEVNET_CONFIG=$ENV_DIR/dev-albatross.toml
    cargo build --target=x86_64-unknown-linux-gnu --bin nimiq-client -Z unstable-options --out-dir $ENV_DIR/build/
"#
}

fn run_devnet(args: Args, keyboard_interrupt: Arc<AtomicBool>) -> Result<(), Error> {
    let env_dir = canonicalize(&args.env).expect("Can't get absolute path");

    // Build `nimiq-client` binary
    info!("Building nimiq-client");
    for line in build_client(env_dir.to_str().unwrap())? {
        println!("{}", line?);
    }

    let docker = Docker::new(&env_dir);

    // build docker containers
    docker.build()?;

    // run and print output
    // TODO: We could split up the streams from validators
    for line in docker.up()? {
        if keyboard_interrupt.load(Ordering::SeqCst) {
            break;
        }
        println!("{}", line?);
    }

    docker.down()?;

    // TODO: prune docker containers

    // delete build directory
    //remove_dir_all(env_dir.join("build"))
    //    .expect("Failed to delete build directory");
    remove_file(env_dir.join("build").join("nimiq-client"))
        .expect("Failed to delete nimiq-client binary");

    Ok(())
}

#[paw::main]
fn main(args: Args) {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    debug!("{:#?}", args);

    // register handler for Ctrl-C
    let keyboard_interrupt = Arc::new(AtomicBool::new(false));
    {
        let keyboard_interrupt = Arc::clone(&keyboard_interrupt);
        ctrlc::set_handler(move || {
            info!("Keyboard interrupt");
            keyboard_interrupt.store(false, Ordering::SeqCst);
        })
        .expect("Failed to register handler for Ctrl-C");
    }

    if let Err(e) = run_devnet(args, keyboard_interrupt) {
        error!("Error: {}", e);
    }
}
