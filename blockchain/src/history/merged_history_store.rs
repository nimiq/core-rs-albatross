use std::cmp;

use nimiq_block::{Block, MicroBlock};
use nimiq_database::mdbx::{MdbxReadTransaction, MdbxWriteTransaction};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_mmr::{
    error::Error,
    mmr::proof::{RangeProof, SizeProof},
};
use nimiq_primitives::policy::Policy;
use nimiq_transaction::{
    historic_transaction::{HistoricTransaction, RawTransactionHash},
    history_proof::HistoryTreeProof,
    inherent::Inherent,
    EquivocationLocator,
};

use super::interface::{HistoryIndexInterface, HistoryInterface};

/// A wrapper around two history stores, one for the pre-genesis epoch and one for the main epoch.
#[derive(Debug)]
pub struct HistoryStoreMerger<S: HistoryInterface> {
    /// The pre-genesis history store is read-only.
    /// When populating it, we load it separately.
    pre_genesis: Option<S>,
    /// The main history store.
    main: S,
}

impl<S: HistoryInterface> HistoryStoreMerger<S> {
    pub(crate) fn new(pre_genesis: Option<S>, main: S) -> Self {
        Self { pre_genesis, main }
    }
}

impl<S: HistoryInterface> HistoryInterface for HistoryStoreMerger<S> {
    fn add_block(
        &self,
        txn: &mut MdbxWriteTransaction,
        block: &Block,
        inherents: Vec<Inherent>,
    ) -> Option<(Blake2bHash, u64)> {
        assert_ne!(
            Policy::epoch_at(block.block_number()),
            0,
            "Epoch 0 is pre-genesis"
        );
        self.main.add_block(txn, block, inherents)
    }

    fn remove_block(
        &self,
        txn: &mut MdbxWriteTransaction,
        block: &MicroBlock,
        inherents: Vec<Inherent>,
    ) -> Option<u64> {
        assert_ne!(
            Policy::epoch_at(block.block_number()),
            0,
            "Epoch 0 is pre-genesis"
        );
        self.main.remove_block(txn, block, inherents)
    }

    fn remove_history(&self, txn: &mut MdbxWriteTransaction, epoch_number: u32) -> Option<()> {
        assert_ne!(epoch_number, 0, "Epoch 0 is pre-genesis");
        self.main.remove_history(txn, epoch_number)
    }

    fn get_history_tree_root(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<Blake2bHash> {
        // The regular history store always returns a hash – even if no data is present.
        if Policy::epoch_at(block_number) == 0 && self.pre_genesis.is_some() {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .unwrap()
                .get_history_tree_root(block_number, None)
        } else {
            self.main.get_history_tree_root(block_number, txn_option)
        }
    }

    fn clear(&self, txn: &mut MdbxWriteTransaction) {
        self.main.clear(txn);
    }

    fn length_at(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<u32> {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis.as_ref()?.length_at(block_number, None)
        } else {
            self.main.length_at(block_number, txn_option)
        }
    }

    fn total_len_at_epoch(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        if epoch_number == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.total_len_at_epoch(0, None))
                .unwrap_or_default()
        } else {
            self.main.total_len_at_epoch(epoch_number, txn_option)
        }
    }

    fn history_store_range(&self, txn_option: Option<&MdbxReadTransaction>) -> (u32, u32) {
        let (pre_genesis_start, pre_genesis_end) = self
            .pre_genesis
            .as_ref()
            .map(|pre_genesis| pre_genesis.history_store_range(None))
            .unwrap_or_default();
        let (main_start, main_end) = self.main.history_store_range(txn_option);
        (
            cmp::min(pre_genesis_start, main_start),
            cmp::max(pre_genesis_end, main_end),
        )
    }

    fn add_to_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        assert_ne!(Policy::epoch_at(block_number), 0, "Epoch 0 is pre-genesis");
        self.main.add_to_history(txn, block_number, hist_txs)
    }

    fn add_to_history_for_epoch(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
        block_number: u32,
        hist_txs: &[HistoricTransaction],
    ) -> Option<(Blake2bHash, u64)> {
        assert_ne!(epoch_number, 0, "Epoch 0 is pre-genesis");
        self.main
            .add_to_history_for_epoch(txn, epoch_number, block_number, hist_txs)
    }

    fn remove_partial_history(
        &self,
        txn: &mut MdbxWriteTransaction,
        epoch_number: u32,
        num_hist_txs: usize,
    ) -> Option<(Blake2bHash, u64)> {
        assert_ne!(epoch_number, 0, "Epoch 0 is pre-genesis");
        self.main
            .remove_partial_history(txn, epoch_number, num_hist_txs)
    }

    fn tx_in_validity_window(
        &self,
        raw_tx_hash: &RawTransactionHash,
        txn_opt: Option<&MdbxReadTransaction>,
    ) -> bool {
        self.main.tx_in_validity_window(raw_tx_hash, txn_opt)
    }

    fn get_block_transactions(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.get_block_transactions(block_number, None))
                .unwrap_or_default()
        } else {
            self.main.get_block_transactions(block_number, txn_option)
        }
    }

    fn get_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        if epoch_number == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.get_epoch_transactions(epoch_number, None))
                .unwrap_or_default()
        } else {
            self.main.get_epoch_transactions(epoch_number, txn_option)
        }
    }

    fn num_epoch_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        if epoch_number == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.num_epoch_transactions(epoch_number, None))
                .unwrap_or_default()
        } else {
            self.main.num_epoch_transactions(epoch_number, txn_option)
        }
    }

    fn num_epoch_transactions_before(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> usize {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.num_epoch_transactions_before(block_number, None))
                .unwrap_or_default()
        } else {
            self.main
                .num_epoch_transactions_before(block_number, txn_option)
        }
    }

    fn get_epoch_transactions_after(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<HistoricTransaction> {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .map(|pre_genesis| pre_genesis.get_epoch_transactions_after(block_number, None))
                .unwrap_or_default()
        } else {
            self.main
                .get_epoch_transactions_after(block_number, txn_option)
        }
    }

    fn prove_chunk(
        &self,
        epoch_number: u32,
        verifier_block_number: u32,
        chunk_size: usize,
        chunk_index: usize,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<super::HistoryTreeChunk> {
        if epoch_number == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis.as_ref()?.prove_chunk(
                epoch_number,
                verifier_block_number,
                chunk_size,
                chunk_index,
                None,
            )
        } else {
            self.main.prove_chunk(
                epoch_number,
                verifier_block_number,
                chunk_size,
                chunk_index,
                txn_option,
            )
        }
    }

    fn tree_from_chunks(
        &self,
        epoch_number: u32,
        chunks: Vec<(Vec<HistoricTransaction>, RangeProof<Blake2bHash>)>,
        txn: &mut MdbxWriteTransaction,
    ) -> Result<Blake2bHash, Error> {
        assert_ne!(epoch_number, 0, "Epoch 0 is pre-genesis");
        self.main.tree_from_chunks(epoch_number, chunks, txn)
    }

    fn get_last_leaf_block_number(&self, txn_option: Option<&MdbxReadTransaction>) -> Option<u32> {
        self.main
            .get_last_leaf_block_number(txn_option)
            .or_else(|| self.pre_genesis.as_ref()?.get_last_leaf_block_number(None))
    }

    fn has_equivocation_proof(
        &self,
        locator: EquivocationLocator,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> bool {
        self.main.has_equivocation_proof(locator, txn_option)
    }

    fn prove_num_leaves(
        &self,
        block_number: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<SizeProof<Blake2bHash, HistoricTransaction>, Error> {
        if Policy::epoch_at(block_number) == 0 {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .ok_or(Error::EmptyTree)?
                .prove_num_leaves(block_number, None)
        } else {
            self.main.prove_num_leaves(block_number, txn_option)
        }
    }
}

impl<S: HistoryInterface + HistoryIndexInterface> HistoryIndexInterface for HistoryStoreMerger<S> {
    fn get_hist_tx_by_hash(
        &self,
        raw_tx_hash: &Blake2bHash,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<HistoricTransaction> {
        self.main
            .get_hist_tx_by_hash(raw_tx_hash, txn_option)
            .or_else(|| {
                self.pre_genesis
                    .as_ref()?
                    .get_hist_tx_by_hash(raw_tx_hash, None)
            })
    }

    fn get_tx_hashes_by_address(
        &self,
        address: &Address,
        max: u16,
        start_at: Option<Blake2bHash>,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<Blake2bHash> {
        let mut tx_hashes =
            self.main
                .get_tx_hashes_by_address(address, max, start_at.clone(), txn_option);

        // Fill up from pre-genesis if necessary.
        if tx_hashes.len() < max as usize && self.pre_genesis.is_some() {
            // If the transaction hashes are empty, we can start at the given hash
            // because the hash does not seem to be in the main database.
            let pre_genesis_start = if tx_hashes.is_empty() { start_at } else { None };

            let mut pre_genesis_tx_hashes =
                self.pre_genesis.as_ref().unwrap().get_tx_hashes_by_address(
                    address,
                    max - tx_hashes.len() as u16,
                    pre_genesis_start,
                    None,
                );
            tx_hashes.append(&mut pre_genesis_tx_hashes);
        }

        tx_hashes
    }

    fn prove(
        &self,
        epoch_number: u32,
        hashes: Vec<&Blake2bHash>,
        verifier_state: Option<usize>,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<HistoryTreeProof> {
        // The regular store always provides a proof.
        if epoch_number == 0 && self.pre_genesis.is_some() {
            // The pre-genesis database has separate transactions.
            // Since it is read-only, we can pass None as the transaction.
            self.pre_genesis
                .as_ref()
                .unwrap()
                .prove(epoch_number, hashes, verifier_state, None)
        } else {
            self.main
                .prove(epoch_number, hashes, verifier_state, txn_option)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;

    use nimiq_database::{
        mdbx::MdbxDatabase,
        traits::{Database, WriteTransaction},
    };
    use nimiq_primitives::{coin::Coin, networks::NetworkId};
    use nimiq_test_log::test;
    use nimiq_transaction::{
        historic_transaction::{
            EquivocationEvent, HistoricTransactionData, JailEvent, PenalizeEvent, RewardEvent,
        },
        ExecutedTransaction, ForkLocator, Transaction as BlockchainTransaction,
    };

    use super::*;
    use crate::HistoryStoreIndex;

    #[test]
    fn history_tree_root_works() {
        let start_block_number = Policy::genesis_block_number();
        test_history_fn(|history_store| {
            history_store.get_history_tree_root(start_block_number, None)
        });
        test_history_fn(|history_store| {
            history_store.get_history_tree_root(start_block_number + 1, None)
        });
        test_history_fn(|history_store| {
            history_store.get_history_tree_root(start_block_number + 2, None)
        });
    }

    #[test]
    fn length_at_works() {
        let start_block_number = Policy::genesis_block_number();
        test_history_fn(|history_store| history_store.length_at(start_block_number, None));
        test_history_fn(|history_store| history_store.length_at(start_block_number + 1, None));
        test_history_fn(|history_store| history_store.length_at(start_block_number + 2, None));
    }

    #[test]
    fn transaction_in_validity_window_works() {
        let hist_txs = gen_hist_txs();
        test_history_fn(|history_store| {
            history_store.tx_in_validity_window(&hist_txs[4].tx_hash(), None)
        });
    }

    #[test]
    fn get_hist_tx_by_hash_works() {
        let hist_txs = gen_hist_txs();

        for tx in hist_txs.iter() {
            test_history_fn(|history_store| history_store.get_hist_tx_by_hash(&tx.tx_hash(), None));
        }
    }

    #[test]
    fn get_block_transactions_works() {
        let start_block_number = Policy::genesis_block_number();
        test_history_fn(|history_store| {
            history_store.get_block_transactions(start_block_number, None)
        });
        test_history_fn(|history_store| {
            history_store.get_block_transactions(start_block_number + 1, None)
        });
        test_history_fn(|history_store| {
            history_store.get_block_transactions(start_block_number + 2, None)
        });
    }

    #[test]
    fn get_epoch_transactions_works() {
        test_history_fn(|history_store| history_store.get_epoch_transactions(0, None));
        test_history_fn(|history_store| history_store.get_epoch_transactions(1, None));
    }

    #[test]
    fn get_num_historic_transactions_works() {
        test_history_fn(|history_store| history_store.num_epoch_transactions(0, None));
        test_history_fn(|history_store| history_store.num_epoch_transactions(1, None));
    }

    #[test]
    fn get_tx_hashes_by_address_works() {
        test_history_fn(|history_store| {
            history_store.get_tx_hashes_by_address(
                &Address::from_user_friendly_address(
                    "NQ09 VF5Y 1PKV MRM4 5LE1 55KV P6R2 GXYJ XYQF",
                )
                .unwrap(),
                99,
                None,
                None,
            )
        });
        test_history_fn(|history_store| {
            history_store.get_tx_hashes_by_address(&Address::burn_address(), 2, None, None)
        });
        test_history_fn(|history_store| {
            history_store.get_tx_hashes_by_address(
                &Address::from_user_friendly_address(
                    "NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T",
                )
                .unwrap(),
                99,
                None,
                None,
            )
        });
        test_history_fn(|history_store| {
            history_store.get_tx_hashes_by_address(
                &Address::from_user_friendly_address(
                    "NQ28 1U7R M38P GN5A 7J8R GE62 8QS7 PK2S 4S31",
                )
                .unwrap(),
                99,
                None,
                None,
            )
        });
        // Create historic transactions.
        let hist_txs = gen_hist_txs();
        let hashes: Vec<_> = hist_txs.iter().map(|hist_tx| hist_tx.tx_hash()).collect();
        test_history_fn(|history_store| {
            history_store.get_tx_hashes_by_address(
                &Address::from_user_friendly_address(
                    "NQ09 VF5Y 1PKV MRM4 5LE1 55KV P6R2 GXYJ XYQF",
                )
                .unwrap(),
                2,
                Some(hashes[5].deref().clone()),
                None,
            )
        });
    }

    #[test]
    fn prove_works() {
        // Create historic transactions.
        let hist_txs = gen_hist_txs();
        let hashes: Vec<_> = hist_txs.iter().map(|hist_tx| hist_tx.tx_hash()).collect();

        test_history_fn(|history_store| {
            history_store
                .prove(0, vec![&hashes[0], &hashes[2]], None, None)
                .map(|proof| proof.proof.nodes)
        });
        test_history_fn(|history_store| {
            history_store
                .prove(1, vec![&hashes[3], &hashes[4], &hashes[6]], None, None)
                .unwrap()
                .proof
                .nodes
        });
    }

    fn create_reward_inherent(block: u32, value: u64) -> HistoricTransaction {
        let reward_address =
            Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap();
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Reward(RewardEvent {
                validator_address: Address::burn_address(),
                reward_address,
                value: Coin::from_u64_unchecked(value),
            }),
        }
    }

    fn create_jail_inherent(block: u32) -> HistoricTransaction {
        let jail_address =
            Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap();
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Jail(JailEvent {
                validator_address: jail_address,
                slots: 1..3,
                offense_event_block: block,
                new_epoch_slot_range: None,
            }),
        }
    }

    fn create_penalize_inherent(block: u32) -> HistoricTransaction {
        let jail_address =
            Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap();
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Penalize(PenalizeEvent {
                validator_address: jail_address,
                offense_event_block: block,
                slot: 0,
            }),
        }
    }

    fn create_equivocation_inherent(block: u32) -> HistoricTransaction {
        let address =
            Address::from_user_friendly_address("NQ04 B79B R4FF 4NGU A9H0 2PT9 9ART 5A88 J73T")
                .unwrap();
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Equivocation(EquivocationEvent {
                locator: EquivocationLocator::Fork(ForkLocator {
                    validator_address: address,
                    block_number: block,
                }),
            }),
        }
    }

    fn create_transaction(block: u32, value: u64) -> HistoricTransaction {
        HistoricTransaction {
            network_id: NetworkId::UnitAlbatross,
            block_number: block,
            block_time: 0,
            data: HistoricTransactionData::Basic(ExecutedTransaction::Ok(
                BlockchainTransaction::new_basic(
                    Address::from_user_friendly_address(
                        "NQ09 VF5Y 1PKV MRM4 5LE1 55KV P6R2 GXYJ XYQF",
                    )
                    .unwrap(),
                    Address::burn_address(),
                    Coin::from_u64_unchecked(value),
                    Coin::from_u64_unchecked(0),
                    0,
                    NetworkId::UnitAlbatross,
                ),
            )),
        }
    }

    fn test_history_fn<R, F>(func: F)
    where
        F: Fn(&dyn HistoryIndexInterface) -> R,
        R: Eq + std::fmt::Debug,
    {
        let (merged, plain) = create_history_store(true);
        assert_eq!(
            func(&merged),
            func(&plain),
            "Mismatch with pre-genesis data"
        );

        let (merged, plain) = create_history_store(false);
        assert_eq!(
            func(&merged),
            func(&plain),
            "Mismatch without pre-genesis data"
        );
    }

    fn create_history_store(
        with_pre_genesis: bool,
    ) -> (HistoryStoreMerger<HistoryStoreIndex>, HistoryStoreIndex) {
        // Generate the reference store.
        let env_plain = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store_plain =
            HistoryStoreIndex::new(env_plain.clone(), NetworkId::UnitAlbatross);

        // Generate the merged store.
        let env_main = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let history_store_main = HistoryStoreIndex::new(env_main.clone(), NetworkId::UnitAlbatross);

        let history_store_pre = if with_pre_genesis {
            let env_pre = MdbxDatabase::new_volatile(Default::default()).unwrap();
            Some(HistoryStoreIndex::new(
                env_pre.clone(),
                NetworkId::UnitAlbatross,
            ))
        } else {
            None
        };

        // Generate transactions and fill stores.
        let txns = gen_hist_txs();

        let mut txn_plain = env_plain.write_transaction();
        if with_pre_genesis {
            // Plain store
            history_store_plain.add_to_history(
                &mut txn_plain,
                Policy::genesis_block_number(),
                &txns[..3],
            );

            let pre_genesis_store = history_store_pre.as_ref().unwrap();
            let mut txn_pre = pre_genesis_store.db.write_transaction();
            pre_genesis_store.add_to_history(
                &mut txn_pre,
                Policy::genesis_block_number(),
                &txns[..3],
            );
            txn_pre.commit();
        }

        let mut txn_main = env_main.write_transaction();
        history_store_plain.add_to_history(
            &mut txn_plain,
            Policy::genesis_block_number() + 2,
            &txns[3..],
        );
        history_store_main.add_to_history(
            &mut txn_main,
            Policy::genesis_block_number() + 2,
            &txns[3..],
        );
        txn_plain.commit();
        txn_main.commit();

        (
            HistoryStoreMerger::new(history_store_pre, history_store_main),
            history_store_plain,
        )
    }

    fn gen_hist_txs() -> Vec<HistoricTransaction> {
        let start_block_number = Policy::genesis_block_number() + 0;
        let ext_0 = create_transaction(start_block_number + 0, 0);
        let ext_1 = create_transaction(start_block_number + 0, 1);
        let ext_2 = create_reward_inherent(start_block_number + 0, 2);

        let ext_3 = create_transaction(start_block_number + 1, 3);
        let ext_4 = create_reward_inherent(start_block_number + 1, 4);

        let ext_5 = create_transaction(start_block_number + 2, 5);
        let ext_6 = create_transaction(start_block_number + 2, 6);
        let ext_7 = create_reward_inherent(start_block_number + 2, 7);
        let ext_8 = create_jail_inherent(start_block_number + 2);
        let ext_9 = create_penalize_inherent(start_block_number + 2);
        let ext_10 = create_equivocation_inherent(start_block_number + 2);

        vec![
            ext_0, ext_1, ext_2, ext_3, ext_4, ext_5, ext_6, ext_7, ext_8, ext_9, ext_10,
        ]
    }
}
