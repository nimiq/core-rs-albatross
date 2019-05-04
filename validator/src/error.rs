use failure::Fail;

use consensus::Error as ConsensusError;

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "{}", _0)]
    ConsensusError(#[cause] ConsensusError),
}

impl From<ConsensusError> for Error {
    fn from(e: ConsensusError) -> Self {
        Error::ConsensusError(e)
    }
}
