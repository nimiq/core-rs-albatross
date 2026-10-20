use nimiq_keys::Address;
use nimiq_primitives::{account::AccountError, coin::Coin, transaction::TransactionError};
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::{
    account::AccountType,
    key_nibbles::KeyNibbles,
    trie::error::{IncompleteTrie, MerkleRadixTrieError},
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::account::bridge_contract::{
    AnyMerkleProof, BridgeError, ChainConfig, OutgoingTransaction, ParsedBurnData,
};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::account::bridge_contract::{
    CreationTransactionData, OutgoingBridgeTransactionData,
};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::{inherent::Inherent, HashType, Transaction};
#[cfg(feature = "interaction-traits")]
use store::BridgeContractStore;
#[cfg(feature = "interaction-traits")]
pub use store::BridgeContractStoreWrite;
pub use store::BridgeNonce;
use thiserror::Error;

use crate::{account::oracle_contract::OracleContract, convert_receipt, AccountReceipt};
#[cfg(feature = "interaction-traits")]
use crate::{
    data_store::{DataStoreRead, DataStoreWrite},
    interaction_traits::{
        AccountInherentInteraction, AccountPruningInteraction, AccountTransactionInteraction,
    },
    reserved_balance::ReservedBalance,
    Account, BlockState, InherentLogger, Log, TransactionLog,
};
mod store;

/// The Bridge contract for cross-chain asset transfers.
///
/// This contract manages cross-chain transactions by validating Merkle proofs
/// against oracle-verified state hashes and processing asset transfers between
/// different blockchain networks.
///
/// The bridge contract uses a subtrie structure similar to the staking contract:
/// ```text
/// BRIDGE_CONTRACT_ADDRESS: BridgeContract
///     |--> PREFIX_NONCE || ADDRESS: BridgeNonce
/// ```
///
/// Nonces are stored in the accounts tree subtree for replay protection.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BridgeContract {
    /// The owner/operator of the bridge contract
    pub owner: Address,

    /// The oracle contract address for state verification
    pub oracle_address: Address,

    /// Contract balance (deposits and fees)
    pub balance: Coin,

    /// Source chain ID this bridge instance supports
    pub source_chain_id: u32,

    /// Chain-specific configuration (address format, endianness, hash function, etc.)
    /// This is used to validate incoming transactions according to the source chain's rules
    pub chain_config: ChainConfig,

    /// Total number of cross-chain transactions processed
    pub transaction_count: u64,
}

impl BridgeContract {
    /// The last nonce released to `address`, or 0 if none was. Fails if the node is still syncing
    /// its accounts tree and has not synced the bridge's nonce store that far yet.
    #[cfg(feature = "interaction-traits")]
    pub fn get_nonce(
        &self,
        data_store: &DataStoreRead,
        address: &Address,
    ) -> Result<u64, IncompleteTrie> {
        stored_nonce(data_store, address)
    }

    /// Checks that `burn_proof` proves its burn record against `oracle`, the bridge's oracle.
    ///
    /// The oracle has `n` slots (`hash_count`). Its write at index `i` stores the root `r_i` of a
    /// record tree in slot `i mod n`, replacing the slot's value `s` with `H(s ++ r_i)`; a slot
    /// starts as the zero hash. A slot's value therefore commits to every root ever written to it,
    /// not only to the latest one.
    ///
    /// The proof is one `MerklePath` from the record's leaf to the current value of the slot of
    /// `oracle_state_index` (`i` below), in three parts:
    /// 1. the record tree, from the leaf up to `r_i`;
    /// 2. the slot's value before write `i`, as a left sibling (the zero hash if `i` was the
    ///    slot's first write);
    /// 3. every root written to the slot after `i`, as right siblings: `r_{i+n}`, `r_{i+2n}`, ...
    ///
    /// While `i` is inside the oracle's window, its last `n` writes, part 3 is empty. Once the
    /// oracle writes `i + n`, a proof built before that fails with `SlotMismatch` until `r_{i+n}`
    /// is appended, and each later write to the slot adds one more node. A proof never expires:
    /// `max_proof_depth` bounds only part 1, because parts 2 and 3 have exactly
    /// `1 + slot_rewrites_since(i)` nodes, a number the oracle fixes.
    pub fn verify_burn_proof(
        &self,
        oracle: &OracleContract,
        burn_proof: &OutgoingTransaction,
    ) -> Result<(), BurnProofError> {
        match burn_proof.merkle_proof {
            AnyMerkleProof::Blake2bPath(_)
            | AnyMerkleProof::Sha256Path(_)
            | AnyMerkleProof::Keccak256Path(_) => {}
            AnyMerkleProof::Blake2b(_)
            | AnyMerkleProof::Sha256(_)
            | AnyMerkleProof::Keccak256(_) => return Err(BurnProofError::NotAMerklePath),
        }

        let index = burn_proof.oracle_state_index;
        let slot_value = oracle
            .get_hash_at_index(index)
            .ok_or(BurnProofError::UnwrittenIndex(index))?;

        // A prover who names an older index of the same slot is credited more slot levels, but
        // the path must still end in the slot's value, and the target's nonce blocks replays.
        let slot_levels = oracle
            .slot_rewrites_since(index)
            .ok_or(BurnProofError::UnwrittenIndex(index))?
            .saturating_add(1);
        let tree_depth = (burn_proof.proof_depth() as u64).saturating_sub(slot_levels);
        if tree_depth > self.chain_config.max_proof_depth as u64 {
            return Err(BurnProofError::TooDeep {
                depth: tree_depth as usize,
                max_depth: self.chain_config.max_proof_depth,
            });
        }

        let leaf_hash = burn_proof
            .extract_burn_transaction_hash(&self.chain_config.hash_function)
            .map_err(BurnProofError::InvalidRecord)?;

        // Folding fails only if the path's hash type differs from the leaf's.
        let root = burn_proof
            .compute_merkle_root(leaf_hash)
            .map_err(|_| BurnProofError::SlotMismatch(index))?;

        if &root != slot_value {
            return Err(BurnProofError::SlotMismatch(index));
        }

        Ok(())
    }

    /// Checks that `burn`, a record parsed by the validation program, may be released by this
    /// bridge, which lives at `bridge_address`.
    ///
    /// The relayer commits records from any contract that forwards data, so a record must come
    /// from the bridge's `source_contract` on its source chain. A burn names the one Nimiq bridge
    /// that may release it, so a second bridge with the same source contract, which starts with
    /// empty nonce counters, cannot release every historical burn again.
    pub fn verify_burn_record(
        &self,
        bridge_address: &Address,
        burn: &ParsedBurnData,
    ) -> Result<(), BurnRecordError> {
        if burn.source_contract != self.chain_config.source_contract {
            return Err(BurnRecordError::WrongSource {
                source_contract: hex::encode(&burn.source_contract),
                expected: hex::encode(&self.chain_config.source_contract),
            });
        }
        if burn.target_chain_id != self.source_chain_id {
            return Err(BurnRecordError::WrongChain {
                chain_id: burn.target_chain_id,
                expected: self.source_chain_id,
            });
        }
        if &burn.bridge_address != bridge_address {
            return Err(BurnRecordError::WrongBridge {
                bridge: burn.bridge_address.clone(),
                expected: bridge_address.clone(),
            });
        }
        Ok(())
    }
}

/// Why a burn record may not be released by a bridge.
#[derive(Clone, Debug, Error)]
pub enum BurnRecordError {
    #[error(
        "the record comes from contract {source_contract}, not from the source contract {expected}"
    )]
    WrongSource {
        source_contract: String,
        expected: String,
    },
    #[error("the burn is for chain {chain_id}, but the bridge serves chain {expected}")]
    WrongChain { chain_id: u32, expected: u32 },
    #[error("the burn may be released only by bridge {bridge}, not by {expected}")]
    WrongBridge { bridge: Address, expected: Address },
}

/// Why a burn proof does not prove its record against the bridge's oracle.
#[derive(Clone, Debug, Error)]
pub enum BurnProofError {
    #[error("the burn proof is not a Merkle path")]
    NotAMerklePath,
    #[error("the record tree has {depth} levels, more than the maximum of {max_depth}")]
    TooDeep { depth: usize, max_depth: u32 },
    #[error("oracle index {0} has not been written")]
    UnwrittenIndex(u64),
    #[error("cannot hash the burn record: {0}")]
    InvalidRecord(BridgeError),
    #[error("the Merkle path does not end in the current value of the oracle slot of index {0}")]
    SlotMismatch(u64),
}

/// The state a release is checked against: the bridge's own nonce store, and the global accounts
/// tree for the oracle.
#[cfg(feature = "interaction-traits")]
trait ReleaseState {
    /// The last nonce released to `target`, or 0 if none was.
    fn last_nonce(&self, target: &Address) -> Result<u64, AccountError>;

    /// The account at `address`, if there is one.
    fn account(&self, address: &Address) -> Result<Option<Account>, AccountError>;
}

/// The mempool checks releases against this state, also while the node is still syncing its
/// accounts tree. The bridge's account is synced by then, but its nonce store or its oracle may not
/// be: a release that needs them is refused rather than checked.
#[cfg(feature = "interaction-traits")]
impl ReleaseState for DataStoreRead<'_, '_, '_, '_> {
    fn last_nonce(&self, target: &Address) -> Result<u64, AccountError> {
        stored_nonce(self, target).map_err(not_synced)
    }

    fn account(&self, address: &Address) -> Result<Option<Account>, AccountError> {
        self.try_get_global(&KeyNibbles::from(address))
            .map_err(not_synced)
    }
}

/// Blocks are applied to a complete tree only (`Accounts::commit_batch` asserts it).
#[cfg(feature = "interaction-traits")]
impl ReleaseState for DataStoreWrite<'_, '_, '_, '_, '_> {
    fn last_nonce(&self, target: &Address) -> Result<u64, AccountError> {
        Ok(self
            .get::<BridgeNonce>(&BridgeContractStore::nonce_key(target))
            .map_or(0, |nonce| nonce.nonce))
    }

    fn account(&self, address: &Address) -> Result<Option<Account>, AccountError> {
        Ok(self.get_global(&KeyNibbles::from(address)))
    }
}

/// The last nonce released to `target` according to the bridge's nonce store in `data_store`, or 0
/// if none was.
#[cfg(feature = "interaction-traits")]
fn stored_nonce(data_store: &DataStoreRead, target: &Address) -> Result<u64, IncompleteTrie> {
    let nonce = data_store.try_get::<BridgeNonce>(&BridgeContractStore::nonce_key(target))?;
    Ok(nonce.map_or(0, |nonce| nonce.nonce))
}

/// A read reached the part of the accounts tree that is not synced yet.
#[cfg(feature = "interaction-traits")]
fn not_synced(_: IncompleteTrie) -> AccountError {
    AccountError::ChunkError(MerkleRadixTrieError::IncompleteTrie)
}

#[cfg(feature = "interaction-traits")]
impl BridgeContract {
    /// Runs the validation program on the record that `release` carries.
    fn parse_burn(
        &self,
        release: &OutgoingBridgeTransactionData,
    ) -> Result<ParsedBurnData, AccountError> {
        release
            .burn_proof
            .parse_burn_data(&self.chain_config)
            .map_err(|error| {
                log::debug!(?error, "Failed to parse burn data");
                AccountError::InvalidTransaction(TransactionError::InvalidData)
            })
    }

    /// Parses `transaction` as a release and checks it against the current state, signature
    /// included. Returns the burn it pays out.
    fn verify_release(
        &self,
        transaction: &Transaction,
        state: &impl ReleaseState,
    ) -> Result<ParsedBurnData, AccountError> {
        let release = OutgoingBridgeTransactionData::parse(transaction)
            .map_err(AccountError::InvalidTransaction)?;
        release
            .verify(transaction)
            .map_err(AccountError::InvalidTransaction)?;
        self.check_release(transaction, &release, state)
    }

    /// Checks `transaction`, a release carrying `release`, against the current state and returns
    /// the burn it pays out. These are all of the bridge's checks on a release except that the
    /// signature is valid, which does not depend on the state: `Transaction::verify` checks it
    /// before the mempool admits a transaction and before a block is applied.
    fn check_release(
        &self,
        transaction: &Transaction,
        release: &OutgoingBridgeTransactionData,
        state: &impl ReleaseState,
    ) -> Result<ParsedBurnData, AccountError> {
        let burn = self.parse_burn(release)?;

        // Only the target can release its burn, since the release consumes the target's nonce.
        // Anyone can still submit a release that the target signed.
        if !release.proof.is_signed_by(&burn.target_address) {
            log::debug!(
                signer = %release.proof.compute_signer(),
                target = %burn.target_address,
                "Release is not signed by the burn target",
            );
            return Err(AccountError::InvalidSignature);
        }

        // The fee is taken from the burned amount: the target receives `value` and the block
        // reward receives `fee`, so together they must add up to exactly the burned amount.
        if transaction.value.checked_add(transaction.fee) != Some(burn.amount) {
            log::debug!(
                tx_value = %transaction.value,
                tx_fee = %transaction.fee,
                parsed_value = %burn.amount,
                "Transaction value and fee do not add up to the burned amount"
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidValue,
            ));
        }

        // Verify the transaction recipient matches the burn proof target address
        if transaction.recipient != burn.target_address {
            log::debug!(
                tx_recipient = %transaction.recipient,
                parsed_recipient = %burn.target_address,
                "Transaction recipient mismatch",
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ));
        }

        // The record must come from the source contract on the source chain and name this bridge
        self.verify_burn_record(&transaction.sender, &burn)
            .map_err(|error| {
                log::debug!(%error, "Burn record not releasable by this bridge");
                AccountError::InvalidTransaction(TransactionError::InvalidData)
            })?;

        // Check nonce for replay protection
        let last_nonce = state.last_nonce(&burn.target_address)?;
        if last_nonce.checked_add(1) != Some(burn.target_nonce) {
            log::debug!(
                parsed_nonce = burn.target_nonce,
                last_nonce,
                "Invalid nonce: expected sequential nonce"
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ));
        }

        let oracle = match state.account(&self.oracle_address)? {
            Some(Account::Oracle(oracle)) => oracle,
            _ => {
                log::debug!(
                    oracle_address = %self.oracle_address,
                    "Oracle contract not found or invalid",
                );
                return Err(AccountError::InvalidForSender);
            }
        };

        // The proof must end in the current value of the oracle slot that the relayer's root was
        // written to.
        self.verify_burn_proof(&oracle, &release.burn_proof)
            .map_err(|error| {
                log::debug!(%error, "Invalid burn proof");
                match error {
                    BurnProofError::UnwrittenIndex(_) | BurnProofError::InvalidRecord(_) => {
                        AccountError::InvalidTransaction(TransactionError::InvalidData)
                    }
                    BurnProofError::NotAMerklePath
                    | BurnProofError::TooDeep { .. }
                    | BurnProofError::SlotMismatch(_) => {
                        AccountError::InvalidTransaction(TransactionError::InvalidProof)
                    }
                }
            })?;

        // The bridge pays out the whole burned amount, `value + fee`
        if self.balance < burn.amount {
            return Err(AccountError::InsufficientFunds {
                needed: burn.amount,
                balance: self.balance,
            });
        }

        Ok(burn)
    }
}

#[cfg(feature = "interaction-traits")]
fn validate_oracle_hash_compatibility(
    oracle_address: &Address,
    chain_config: &ChainConfig,
    data_store: DataStoreWrite,
) -> Result<(), AccountError> {
    // Convert address to KeyNibbles for data store query
    let oracle_key = KeyNibbles::from(oracle_address);

    // Query the oracle contract directly from the global accounts tree
    // (not the bridge's subtrie).
    let oracle_account: Option<Account> = data_store.get_global(&oracle_key);

    // Extract oracle contract if it exists
    let oracle = match oracle_account {
        Some(Account::Oracle(oracle)) => oracle,
        Some(_) => {
            log::warn!(
                %oracle_address,
                "Bridge creation failed: address exists but is not an oracle contract"
            );
            return Err(AccountError::InvalidForRecipient);
        }
        None => {
            log::warn!(
                %oracle_address,
                "Bridge creation failed: oracle contract does not exist in accounts tree"
            );
            return Err(AccountError::InvalidForRecipient);
        }
    };

    // The oracle fixes its algorithm at creation, so this holds even before its first update.
    let oracle_hash_type = oracle.hash_algorithm;
    let bridge_hash_type = HashType::from_any_hash(&chain_config.hash_function);

    // Validate hash types match
    if oracle_hash_type != bridge_hash_type {
        log::warn!(
            oracle_hash_type = oracle_hash_type.name(),
            bridge_hash_type = bridge_hash_type.name(),
            %oracle_address,
            "Bridge creation failed: hash function mismatch"
        );
        return Err(AccountError::InvalidTransaction(
            TransactionError::InvalidData,
        ));
    }

    log::info!(
        hash_type = oracle_hash_type.name(),
        "Bridge-oracle hash compatibility validated",
    );

    Ok(())
}

#[cfg(feature = "interaction-traits")]
impl AccountTransactionInteraction for BridgeContract {
    fn create_new_contract(
        transaction: &Transaction,
        initial_balance: Coin,
        _block_state: &BlockState,
        data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Account, AccountError> {
        let data = CreationTransactionData::parse(transaction)
            .map_err(AccountError::InvalidTransaction)?;

        // Verify the creation data
        data.verify().map_err(AccountError::InvalidTransaction)?;

        // Validate oracle hash compatibility
        validate_oracle_hash_compatibility(&data.oracle_address, &data.chain_config, data_store)?;

        // The deposit is the transaction value
        let deposit = transaction.value;

        tx_logger.push_log(Log::BridgeCreate {
            contract_address: transaction.recipient.clone(),
            owner: data.owner.clone(),
            oracle_address: data.oracle_address.clone(),
            source_chain_id: data.source_chain_id,
            deposit,
        });

        Ok(Account::Bridge(BridgeContract {
            balance: initial_balance + deposit,
            owner: data.owner,
            oracle_address: data.oracle_address,
            source_chain_id: data.source_chain_id,
            chain_config: data.chain_config,
            transaction_count: 0,
        }))
    }

    fn revert_new_contract(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance -= transaction.value;

        tx_logger.push_log(Log::BridgeCreate {
            contract_address: transaction.recipient.clone(),
            owner: self.owner.clone(),
            oracle_address: self.oracle_address.clone(),
            source_chain_id: self.source_chain_id,
            deposit: transaction.value,
        });

        Ok(())
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        // Regular incoming transactions to bridge (user locking funds)
        // The transaction should have IncomingTransaction in recipient_data
        // specifying the target chain and address

        // Increment bridge balance (user is locking funds)
        self.balance += transaction.value;
        self.transaction_count += 1;

        tx_logger.push_log(Log::BridgeIncoming {
            contract_address: transaction.recipient.clone(),
            sender: transaction.sender.clone(),
            value: transaction.value,
        });

        Ok(None)
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Revert the balance increase (user was locking funds)
        self.balance -= transaction.value;
        self.transaction_count -= 1;

        tx_logger.push_log(Log::BridgeIncoming {
            contract_address: transaction.recipient.clone(),
            sender: transaction.sender.clone(),
            value: transaction.value,
        });

        Ok(())
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        mut data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let parsed_burn = self.verify_release(transaction, &data_store)?;

        // Decrement bridge balance by the whole burned amount, `value + fee`
        self.balance = self.balance.safe_sub(parsed_burn.amount)?;

        // Update processed nonces in the accounts tree
        let mut store = BridgeContractStoreWrite::new(&mut data_store);
        store.put_nonce(
            &parsed_burn.target_address,
            BridgeNonce {
                nonce: parsed_burn.target_nonce,
            },
        );

        // Increment transaction count
        self.transaction_count += 1;

        // Log the outgoing transaction
        tx_logger.push_log(Log::pay_fee_log(transaction));
        tx_logger.push_log(Log::BridgeOutgoing {
            contract_address: transaction.sender.clone(),
            recipient: parsed_burn.target_address.clone(),
            value: transaction.value,
        });

        // Create receipt for revert
        let receipt = ProcessOutgoingReceipt {
            target_address: parsed_burn.target_address,
            nonce: parsed_burn.target_nonce,
            amount: parsed_burn.amount,
        };

        Ok(Some(receipt.into()))
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        mut data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Extract receipt
        let receipt = receipt.ok_or(AccountError::InvalidReceipt)?;
        let receipt = ProcessOutgoingReceipt::try_from(&receipt)?;

        // Revert balance change (add back the released funds)
        self.balance += receipt.amount;

        // Revert nonce update in the accounts tree
        let mut store = BridgeContractStoreWrite::new(&mut data_store);

        // Restore the previous nonce. If the reverted nonce was 1 (the first ever for
        // this address), remove the entry entirely so state matches the pre-tx condition.
        if receipt.nonce > 1 {
            store.put_nonce(
                &receipt.target_address,
                BridgeNonce {
                    nonce: receipt.nonce - 1,
                },
            );
        } else {
            store.remove_nonce(&receipt.target_address);
        }

        // Decrement transaction count
        self.transaction_count -= 1;

        // Log the revert
        tx_logger.push_log(Log::BridgeOutgoing {
            contract_address: transaction.sender.clone(),
            recipient: receipt.target_address.clone(),
            value: transaction.value,
        });
        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(())
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        // A release succeeds or is invalid, never failed. A failed transaction pays its fee from
        // the sender, but the bridge never pays for a failed release and nobody else does either:
        // otherwise a target could drain the bridge through the fees of releases it signs to fail,
        // and failing releases without a fee would fill blocks for free. So a block containing a
        // failing release is invalid. The error is the reason the release fails; if the bridge's
        // checks pass, the recipient refused it.
        Err(self
            .verify_release(transaction, &data_store)
            .err()
            .unwrap_or(AccountError::InvalidForRecipient))
    }

    fn revert_failed_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Do nothing: a failed release is never committed.
        Ok(())
    }

    fn reserve_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _block_state: &BlockState,
        data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        // A failing release makes its block invalid, so the mempool admits a release only if it
        // succeeds against the current state. The mempool calls this again for every pending
        // release after each block, which evicts a release once its nonce is used or its oracle
        // slot is overwritten. The signature was verified when the release entered the mempool.
        let release = OutgoingBridgeTransactionData::parse(transaction)
            .map_err(AccountError::InvalidTransaction)?;
        let burn = self.check_release(transaction, &release, &data_store)?;

        // A target can only release its next nonce, so the mempool holds one release per target:
        // the target's bucket fits the amount of one burn, as a staker's fits its retired balance.
        reserved_balance.reserve_for(&burn.target_address, burn.amount, burn.amount)?;

        // All pending releases together cannot take more than the bridge holds.
        reserved_balance
            .reserve(self.balance, burn.amount)
            .inspect_err(|_| reserved_balance.release_for(&burn.target_address, burn.amount))
    }

    fn release_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        // `reserve_balance` parsed this release, and the bridge's program never changes.
        let release = OutgoingBridgeTransactionData::parse(transaction)
            .map_err(AccountError::InvalidTransaction)?;
        let burn = self.parse_burn(&release)?;

        reserved_balance.release_for(&burn.target_address, burn.amount);
        reserved_balance.release(burn.amount);
        Ok(())
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountInherentInteraction for BridgeContract {
    fn commit_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _inherent_logger: &mut InherentLogger,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Err(AccountError::InvalidForTarget)
    }

    fn revert_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        _inherent_logger: &mut InherentLogger,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForTarget)
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountPruningInteraction for BridgeContract {
    fn can_be_pruned(&self) -> bool {
        self.balance.is_zero()
    }

    fn prune(self, _data_store: DataStoreRead) -> Option<AccountReceipt> {
        Some(PrunedBridgeContract::from(self).into())
    }

    fn restore(
        _ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        let receipt = pruned_account.ok_or(AccountError::InvalidReceipt)?;
        let pruned_account = PrunedBridgeContract::try_from(receipt)?;
        Ok(Account::Bridge(BridgeContract::from(pruned_account)))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct PrunedBridgeContract {
    pub owner: Address,
    pub oracle_address: Address,
    pub source_chain_id: u32,
    pub chain_config: ChainConfig,
    pub transaction_count: u64,
}

impl From<BridgeContract> for PrunedBridgeContract {
    fn from(contract: BridgeContract) -> Self {
        PrunedBridgeContract {
            owner: contract.owner,
            oracle_address: contract.oracle_address,
            source_chain_id: contract.source_chain_id,
            chain_config: contract.chain_config,
            transaction_count: contract.transaction_count,
        }
    }
}

impl From<PrunedBridgeContract> for BridgeContract {
    fn from(receipt: PrunedBridgeContract) -> Self {
        BridgeContract {
            balance: Coin::ZERO,
            owner: receipt.owner,
            oracle_address: receipt.oracle_address,
            source_chain_id: receipt.source_chain_id,
            chain_config: receipt.chain_config,
            transaction_count: receipt.transaction_count,
        }
    }
}

convert_receipt!(PrunedBridgeContract);

/// Receipt for process outgoing transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct ProcessOutgoingReceipt {
    /// The address that received the transaction
    pub target_address: Address,
    /// The nonce that was processed
    pub nonce: u64,
    /// The amount that was released
    pub amount: Coin,
}

convert_receipt!(ProcessOutgoingReceipt);
