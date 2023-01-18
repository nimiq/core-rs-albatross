mod sync_utils;

use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;

use crate::sync_utils::{sync_two_peers, SyncMode};

#[test(tokio::test)]
async fn two_peers_can_sync_empty_chain() {
    sync_two_peers(0, 1, SyncMode::Full).await
}

#[test(tokio::test)]
async fn two_peers_can_sync_single_batch() {
    sync_two_peers(1, 1, SyncMode::Full).await
}

#[test(tokio::test)]
async fn two_peers_can_sync_two_batches() {
    sync_two_peers(2, 1, SyncMode::Full).await
}

#[test(tokio::test)]
async fn two_peers_can_sync_epoch_minus_batch() {
    sync_two_peers(
        (Policy::batches_per_epoch() - 1) as usize,
        1,
        SyncMode::Full,
    )
    .await
} //

#[test(tokio::test)]
async fn two_peers_can_sync_epoch_plus_batch() {
    sync_two_peers(
        (Policy::batches_per_epoch() + 1) as usize,
        1,
        SyncMode::Full,
    )
    .await
} //

#[test(tokio::test)]
async fn two_peers_can_sync_epoch_plus_two_batches() {
    sync_two_peers(
        (Policy::batches_per_epoch() + 2) as usize,
        1,
        SyncMode::Full,
    )
    .await
} //

#[test(tokio::test)]
async fn two_peers_can_sync_single_epoch() {
    sync_two_peers(Policy::batches_per_epoch() as usize, 1, SyncMode::Full).await
} //

#[test(tokio::test)]
async fn two_peers_can_sync_two_epochs() {
    sync_two_peers(
        (Policy::batches_per_epoch() * 2) as usize,
        1,
        SyncMode::Full,
    )
    .await
} //
