use std::fmt;
use std::sync::Arc;
use std::io::{Error as IoError, ErrorKind};

use block_albatross::ViewChange;
use messages::Message;

use handel::sender::Sender;
use handel::update::LevelUpdate;

use super::voting::{VotingProtocol, VoteAggregation, Tag};


impl Tag for ViewChange {
    fn create_level_update_message(&self, update: LevelUpdate) -> Message {
        Message::HandelViewChange(Box::new(update.with_tag(self.clone())))
    }
}

pub type ViewChangeAggregation = VoteAggregation<ViewChange>;

impl fmt::Debug for ViewChangeAggregation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ViewChangeAggregation {{ node_id: {}, block_number: {}, new_view_number: {} }}", self.node_id(), self.tag().block_number, self.tag().new_view_number)
    }
}
