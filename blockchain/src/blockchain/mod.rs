use nimiq_block::Block;
use nimiq_blockchain_interface::{PushError, PushResult};

mod abstract_blockchain;
pub mod accounts;
#[allow(clippy::module_inception)]
pub mod blockchain;
pub mod history_sync;
pub mod inherents;
pub mod push;
pub(super) mod rebranch_utils;
pub mod slots;
pub mod verify;
pub mod wrappers;
pub mod zkp_sync;

/// A post-validation hook that can be registered with the blockchain to run after block validation.
/// For `extend` and `rebranch` operations, this is called before a block is committed to the database.
/// It thus provides a timing advantage.
pub trait PostValidationHook {
    /// Run the post-validation hook.
    /// For `extend` and `rebranch` this is called before a block is committed to the database.
    fn post_validation(&self, block: &Block, push_result: Result<&PushResult, &PushError>);
}

/// A dummy post-validation hook that does nothing.
impl PostValidationHook for () {
    fn post_validation(&self, _block: &Block, _push_result: Result<&PushResult, &PushError>) {}
}

impl<F: PostValidationHook> PostValidationHook for Option<F> {
    fn post_validation(&self, block: &Block, push_result: Result<&PushResult, &PushError>) {
        self.as_ref()
            .inspect(|hook| hook.post_validation(block, push_result));
    }
}
