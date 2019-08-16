use std::fmt;
use std::sync::Arc;
use std::io::{Error as IoError, ErrorKind};

use block_albatross::{PbftProposal, PbftPrepareMessage, PbftCommitMessage};
use hash::Blake2bHash;
use messages::Message;

use handel::sender::Sender;
use handel::update::LevelUpdate;

use super::voting::{VotingProtocol, VoteAggregation, Tag};


impl Tag for PbftPrepareMessage {
    fn create_level_update_message(&self, update: LevelUpdate) -> Message {
        Message::HandelPbftPrepare(Box::new(update.with_tag(self.clone())))
    }
}

impl Tag for PbftCommitMessage {
    fn create_level_update_message(&self, update: LevelUpdate) -> Message {
        Message::HandelPbftCommit(Box::new(update.with_tag(self.clone())))
    }
}


pub type PbftPrepareProtocol = VotingProtocol<PbftPrepareMessage>;
pub type PbftCommitProtocol = VotingProtocol<PbftCommitMessage>;

// TODO: Implement a Aggregation, that does both prepare and commit.
