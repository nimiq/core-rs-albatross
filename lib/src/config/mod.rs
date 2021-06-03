pub mod command_line;
/// This modules manages the Nimiq configuration. It provides a builder and can load from
/// config file or command line.
///
/// # ToDo
///
/// * Load config from environment variables. envopt doesn't look very usable, but we can easily
///   write our own derive macro, that will also use FromStr to parse the variables.
#[allow(clippy::module_inception)]
pub mod config;
pub mod config_file;
pub mod consts;
pub mod paths;
pub mod user_agent;
