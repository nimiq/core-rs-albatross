extern crate toml;
#[macro_use]
extern crate serde_derive;
extern crate pretty_env_logger;
#[macro_use]
extern crate log;

#[cfg(debug_assertions)]
extern crate dotenv;

mod settings;

use std::path::Path;
use std::error::Error;

use crate::settings::Settings;


// There is no really good way for a condition on the compilation envirnoment (dev, staging, prod)
#[cfg(debug_assertions)]
fn dev_init() {
    dotenv::dotenv().expect("Couldn't load dotenv");
}

fn main() -> Result<(), Box<dyn Error>> {
    dev_init();

    pretty_env_logger::init();

    let argv: Vec<String> = std::env::args().collect();
    let path = argv.get(1)
        .expect(&format!("Usage: {} CONFIG", argv.get(0).unwrap_or(&String::from("nimiq"))));

    info!("Reading configuration: {}", path);
    let settings = Settings::from_file("./config.toml")?;

    println!("{:#?}", settings);

    Ok(())
}
