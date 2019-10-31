use failure::Fail;


#[derive(Clone, Debug, Fail)]
pub enum Error {
    #[fail(display = "Configuration error: {}", _0)]
    ConfigError(String),
}
