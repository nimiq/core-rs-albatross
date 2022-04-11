use std::fmt::Debug;

use beserial::{Deserialize, Serialize};
use nimiq_network_interface::request::Request;

use crate::contribution::AggregatableContribution;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LevelUpdate<C: AggregatableContribution> {
    /// The updated multi-signature for this level
    pub aggregate: C,

    /// The individual signature of the sender, or `None`
    pub(crate) individual: Option<C>,

    /// The level to which this multi-signature belongs to
    pub(crate) level: u8,

    /// The validator ID of the sender (a.k.a. `pk_idx`)
    ///
    /// NOTE: It's safe to just send your own validator ID, since everything critical is authenticated
    /// by signatures anyway.
    pub(crate) origin: u16,
}

impl<C: AggregatableContribution> LevelUpdate<C> {
    /// crate a new LevelUpdate
    /// * `aggregate` - The aggregated contribution
    /// * `individual` - The contribution of the sender, or none. Must have `individual.num_contributors() == 1`
    /// * `level` - The level this update belongs to
    /// * `origin` - the identifier of the sender
    pub fn new(aggregate: C, individual: Option<C>, level: usize, origin: usize) -> Self {
        Self {
            aggregate,
            individual,
            level: level as u8,
            origin: origin as u16,
        }
    }

    /// Add a tag to the Update, resulting in a LevelUpdateMessage which can be send over wire.
    /// * `tag` The message this aggregation runs over
    pub fn with_tag<T: Clone + Debug + Serialize + Deserialize + Send + Unpin>(
        self,
        tag: T,
    ) -> LevelUpdateMessage<C, T> {
        LevelUpdateMessage { update: self, tag }
    }

    /// The source (i.e id) of the sender of this update
    pub fn origin(&self) -> usize {
        self.origin as usize
    }

    /// return the level this update is for
    pub fn level(&self) -> usize {
        self.level as usize
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LevelUpdateMessage<
    C: AggregatableContribution,
    T: Clone + Debug + Serialize + Deserialize + Send + Unpin,
> {
    /// The update for that level
    pub update: LevelUpdate<C>,

    /// The message this aggregation is running over. This is needed to differentiate to which
    /// aggregation this belongs to.
    pub tag: T,
}

impl<
        C: AggregatableContribution + 'static,
        T: Clone + Debug + Serialize + Deserialize + Send + Sync + Unpin + 'static,
    > Request for LevelUpdateMessage<C, T>
{
    // The Type ID to use will come from the AggregatableContribution implementation
    // since having a fixed value here would imply that there could be different
    // types using the same type ID which would confuse the network at decoding
    // messages upon receiving them.
    const TYPE_ID: u16 = C::TYPE_ID;
}
