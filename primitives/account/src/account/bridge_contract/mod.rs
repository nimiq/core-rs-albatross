use nimiq_keys::Address;
use nimiq_primitives::{account::AccountError, coin::Coin, transaction::TransactionError};
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::{account::AccountType, key_nibbles::KeyNibbles};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::account::bridge_contract::ChainConfig;
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::account::bridge_contract::{
    CreationTransactionData, OutgoingBridgeTransactionData,
};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::{inherent::Inherent, HashType, Transaction};
#[cfg(feature = "interaction-traits")]
pub use store::BridgeContractStoreWrite;
pub use store::BridgeNonce;
use store::{BridgeContractStoreRead, BridgeContractStoreReadOps};

use crate::{convert_receipt, data_store_ops::DataStoreReadOps, AccountReceipt};
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
    /// Get the nonce for a given address, if it exists.
    pub fn get_nonce<T: DataStoreReadOps>(&self, data_store: &T, address: &Address) -> u64 {
        BridgeContractStoreRead::new(data_store)
            .get_nonce(address)
            .map(|n| n.nonce)
            .unwrap_or(0)
    }
}

#[cfg(feature = "interaction-traits")]
impl BridgeContract {
    fn can_change_balance(
        &self,
        transaction: &Transaction,
        new_balance: Coin,
        is_reserve: bool,
    ) -> Result<(), AccountError> {
        // Outgoing bridge transactions are permissionless burn-releases: the Merkle
        // burn-proof verified in `commit_outgoing_transaction` is the sole authorization,
        // so we deliberately do NOT require the owner's signature here. Requiring it made
        // `reserve_balance` (mempool admission) reject the very transactions that `commit`
        // accepts, diverging the mempool from consensus and breaking permissionless
        // submission. Balance availability is still enforced by `reserve_balance` itself.

        // If withdrawing, must withdraw the full balance (contract deletion)
        if new_balance < self.balance {
            // For reserve_balance, we allow reserving any amount up to the full balance
            if is_reserve {
                return Ok(());
            }
            // For actual withdrawal, only allow full withdrawal (balance goes to zero)
            if new_balance != Coin::ZERO {
                return Err(AccountError::InvalidForSender);
            }
            // Check that the transaction value equals the current balance
            if transaction.value != self.balance {
                return Err(AccountError::InvalidForSender);
            }
        }

        Ok(())
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

    // If oracle has no hashes yet, any hash function is acceptable
    if oracle.hashes.is_empty() {
        log::info!(
            %oracle_address,
            hash_function=HashType::from_any_hash(&chain_config.hash_function).name(),
            "Oracle is empty, bridge hash function will be accepted",
        );
        return Ok(());
    }

    // Get the oracle's hash type from its first hash
    let oracle_hash_type = HashType::from_any_hash(&oracle.hashes[0]);
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
        // Parse the outgoing bridge transaction data from sender_data
        let outgoing_data = OutgoingBridgeTransactionData::parse(transaction)
            .map_err(AccountError::InvalidTransaction)?;

        // Owner signature check is on purpose missing  to enable permissionless burn proof submission.

        // Verify the transaction signature
        outgoing_data
            .verify(transaction)
            .map_err(AccountError::InvalidTransaction)?;

        // Parse burn data using the validation program
        let parsed_burn = outgoing_data
            .burn_proof
            .parse_burn_data(&self.chain_config)
            .map_err(|error| {
                log::warn!(?error, "Failed to parse burn data");
                AccountError::InvalidTransaction(TransactionError::InvalidData)
            })?;

        // Verify the transaction value matches the burn proof amount
        if transaction.value != parsed_burn.amount {
            log::warn!(
                tx_value = %transaction.value,
                parsed_value = %parsed_burn.amount,
                "Transaction value mismatch"
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidValue,
            ));
        }

        // Verify the transaction recipient matches the burn proof target address
        if transaction.recipient != parsed_burn.target_address {
            log::warn!(
                tx_recipient = %transaction.recipient,
                parsed_recipient = %parsed_burn.target_address,
                "Transaction recipient mismatch",
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ));
        }

        // Verify the target chain ID matches this bridge's source chain ID
        // (burn happened on source chain, releasing on Nimiq)
        if parsed_burn.target_chain_id != self.source_chain_id {
            log::warn!(
                expected = self.source_chain_id,
                obtained = parsed_burn.target_chain_id,
                "Chain ID mismatch"
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ));
        }

        // Check nonce for replay protection
        let mut store = BridgeContractStoreWrite::new(&mut data_store);
        let highest_nonce = store
            .get_nonce(&parsed_burn.target_address)
            .map(|n| n.nonce)
            .unwrap_or(0);

        if parsed_burn.target_nonce != highest_nonce + 1 {
            log::warn!(
                parsed_nonce = parsed_burn.target_nonce,
                expected_nonce = highest_nonce + 1,
                "Invalid nonce: expected sequential nonce"
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ));
        }

        // Query the oracle contract from the global accounts tree.
        let oracle_key = KeyNibbles::from(&self.oracle_address);
        let oracle_account: Option<Account> = store.data_store().get_global(&oracle_key);

        let oracle = match oracle_account {
            Some(Account::Oracle(oracle)) => oracle,
            _ => {
                log::warn!(
                    oracle_address = %self.oracle_address,
                    "Oracle contract not found or invalid",
                );
                return Err(AccountError::InvalidForSender);
            }
        };

        // Verify the oracle state index is valid
        let oracle_state_index = outgoing_data.burn_proof.oracle_state_index as usize;
        if oracle_state_index >= oracle.hashes.len() {
            log::warn!(
                oracle_state_index,
                oracle_hashes_len = oracle.hashes.len(),
                "Oracle state index out of bounds",
            );
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ));
        }

        // Compute the leaf hash from burn transaction data
        let leaf_hash = outgoing_data
            .burn_proof
            .extract_burn_transaction_hash(&self.chain_config.hash_function)
            .map_err(|error| {
                log::warn!(?error, "Failed to compute leaf hash");
                AccountError::InvalidTransaction(TransactionError::InvalidData)
            })?;

        let zero_hash = leaf_hash.zero_of_same_type();

        // Get the oracle state hash at the specified index and its predecessor
        let (prev_oracle_state_hash, current_oracle_state_hash) = {
            let prev_oracle_state_hash = if oracle_state_index > 0 {
                oracle
                    .get_hash_at_index(oracle_state_index.saturating_sub(1) as u64)
                    .ok_or(AccountError::InvalidTransaction(
                        TransactionError::InvalidData,
                    ))?
            } else {
                &zero_hash
            };
            (
                prev_oracle_state_hash,
                oracle.get_hash_at_index(oracle_state_index as u64).ok_or(
                    AccountError::InvalidTransaction(TransactionError::InvalidData),
                )?,
            )
        };

        // Verify the merkle proof
        let merkle_root = outgoing_data
            .burn_proof
            .compute_merkle_root(leaf_hash)
            .map_err(|error| {
                log::warn!(
                    ?error,
                    "Unable to compute root given the proof and transaction"
                );
                AccountError::InvalidTransaction(TransactionError::InvalidProof)
            })?;

        if current_oracle_state_hash != &prev_oracle_state_hash.digest(&merkle_root) {
            log::warn!("Proof and oracle state mismatch");
            return Err(AccountError::InvalidTransaction(
                TransactionError::InvalidProof,
            ));
        }

        // Decrement bridge balance (releasing funds)
        self.balance = self.balance.checked_sub(parsed_burn.amount).ok_or(
            AccountError::InsufficientFunds {
                needed: parsed_burn.amount,
                balance: self.balance,
            },
        )?;

        // Update processed nonces in the accounts tree
        store.put_nonce(
            &parsed_burn.target_address,
            BridgeNonce {
                nonce: parsed_burn.target_nonce,
            },
        );

        // Increment transaction count
        self.transaction_count += 1;

        // Log the outgoing transaction
        tx_logger.push_log(Log::BridgeOutgoing {
            contract_address: transaction.sender.clone(),
            recipient: parsed_burn.target_address.clone(),
            value: parsed_burn.amount,
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

        let current_nonce = store
            .get_nonce(&receipt.target_address)
            .map(|n| n.nonce)
            .unwrap_or(0);

        if current_nonce == receipt.nonce {
            // Remove the nonce entry if it matches
            store.remove_nonce(&receipt.target_address);
        } else {
            // If nonce doesn't match, restore the previous nonce
            // This handles the case where multiple transactions were processed
            if receipt.nonce > 0 {
                store.put_nonce(
                    &receipt.target_address,
                    BridgeNonce {
                        nonce: receipt.nonce - 1,
                    },
                );
            } else {
                store.remove_nonce(&receipt.target_address);
            }
        }

        // Decrement transaction count
        self.transaction_count -= 1;

        // Log the revert
        tx_logger.push_log(Log::BridgeOutgoing {
            contract_address: transaction.sender.clone(),
            recipient: receipt.target_address.clone(),
            value: receipt.amount,
        });

        Ok(())
    }

    fn commit_failed_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        // Do nothing: Fee deduction handled by `Accounts`
        Ok(None)
    }

    fn revert_failed_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Do nothing: Fee revert handled by `Accounts`
        Ok(())
    }

    fn reserve_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _block_state: &BlockState,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        // Only the released `value` comes from the bridge balance. The fee is paid by
        // the burn-proof signer (see `Accounts::charge_fee_to_signer`), so it is reserved
        // against the signer's account by the mempool, not against the bridge here.
        let needed = reserved_balance
            .balance()
            .checked_add(transaction.value)
            .ok_or(AccountError::InvalidCoinValue)?;
        let new_balance = self.balance.safe_sub(needed)?;
        self.can_change_balance(transaction, new_balance, true)?;

        reserved_balance.reserve(self.balance, transaction.value)
    }

    fn release_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        reserved_balance.release(transaction.value);
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
