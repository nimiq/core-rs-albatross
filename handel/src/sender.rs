use std::error::Error;
use std::fmt::Debug;

use crate::contribution::AggregatableContribution;
use crate::update::LevelUpdate;

pub trait Sender<C: AggregatableContribution> {
    type Error: Error + Debug;

    fn send_to(&self, peer_id: usize, update: LevelUpdate<C>);
}
