use nimiq_account::{Account, AccountsError, BlockState};
use nimiq_block::{
    EquivocationProof, MacroBlock, MacroBody, MacroHeader, MicroBlock, MicroBody, MicroHeader,
    MicroJustification, SkipBlockInfo, SkipBlockProof,
};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_database::{mdbx::MdbxReadTransaction as DBTransaction, traits::WriteTransaction};
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash};
use nimiq_keys::KeyPair as SchnorrKeyPair;
use nimiq_primitives::policy::Policy;
use nimiq_transaction::{
    historic_transaction::HistoricTransaction, inherent::Inherent, Transaction,
};
use rand::{CryptoRng, Rng, RngCore};
use thiserror::Error;

use crate::{interface::HistoryInterface, Blockchain};

#[derive(Debug, Error)]
pub enum BlockProducerError {
    #[error("Failed to commit: error={0}, account={1:?}, transactions={2:?}, inherents={3:?}")]
    AccountsError(AccountsError, Account, Vec<Transaction>, Vec<Inherent>),
    #[error("Failed to add to history")]
    HistoryError,
    #[error("Accounts are incomplete")]
    AccountsIncomplete,
}

impl BlockProducerError {
    fn accounts_error(
        blockchain: &Blockchain,
        error: AccountsError,
        transactions: Vec<Transaction>,
        inherents: Vec<Inherent>,
    ) -> Self {
        let Some(account) = (match &error {
            AccountsError::InvalidTransaction(_, transaction) => {
                blockchain.get_account_if_complete(&transaction.sender)
            }
            AccountsError::InvalidInherent(_, inherent) => {
                blockchain.get_account_if_complete(inherent.target())
            }
            AccountsError::InvalidDiff(_) => unreachable!(),
        }) else {
            return BlockProducerError::AccountsIncomplete;
        };

        BlockProducerError::AccountsError(error, account, transactions, inherents)
    }
}

/// Struct that contains all necessary information to actually produce blocks.
/// It has the validator keys for this validator.
#[derive(Clone)]
pub struct BlockProducer {
    pub signing_key: SchnorrKeyPair,
    pub voting_key: BlsKeyPair,
}

impl BlockProducer {
    /// Creates a new BlockProducer struct given a blockchain and a validator key.
    pub fn new(signing_key: SchnorrKeyPair, voting_key: BlsKeyPair) -> Self {
        BlockProducer {
            signing_key,
            voting_key,
        }
    }

    /// Creates the next micro block.
    pub fn next_micro_block(
        &self,
        // The (upgradable) read locked guard to the blockchain
        blockchain: &Blockchain,
        // The timestamp for the block.
        timestamp: u64,
        // Proofs of any misbehavior by malicious validators. An equivocation proof may be submitted
        // during the batch when it happened or until the end of the reporting window, but not after
        // that.
        equivocation_proofs: Vec<EquivocationProof>,
        // The transactions to be included in the block body.
        transactions: Vec<Transaction>,
        // Extra data for this block.
        extra_data: Vec<u8>,
        // Skip block proof
        skip_block_proof: Option<SkipBlockProof>,
    ) -> Result<MicroBlock, BlockProducerError> {
        self.next_micro_block_with_rng(
            blockchain,
            timestamp,
            equivocation_proofs,
            transactions,
            extra_data,
            skip_block_proof,
            &mut rand::thread_rng(),
        )
    }

    /// Creates the next micro block.
    pub fn next_micro_block_with_rng<R: Rng + CryptoRng>(
        &self,
        // The (upgradable) read locked guard to the blockchain.
        blockchain: &Blockchain,
        // The timestamp for the block.
        timestamp: u64,
        // Proofs of any misbehavior by malicious validators. An equivocation proof may be submitted
        // during the batch when it happened or until the end of the reporting window, but not after
        // that.
        equivocation_proofs: Vec<EquivocationProof>,
        // The transactions to be included in the block body.
        transactions: Vec<Transaction>,
        // Extra data for this block.
        extra_data: Vec<u8>,
        // Skip block proof.
        skip_block_proof: Option<SkipBlockProof>,
        // The rng seed. We need this parameterized in order to have determinism when running unit tests.
        rng: &mut R,
    ) -> Result<MicroBlock, BlockProducerError> {
        // The network ID stays unchanged for the whole blockchain.
        let network = blockchain.head().network();

        // Calculate the block number. It is simply the previous block number incremented by one.
        let block_number = blockchain.block_number() + 1;

        // Calculate the timestamp. It must be greater than or equal to the previous block
        // timestamp (i.e. time must not go back).
        let timestamp = u64::max(timestamp, blockchain.timestamp());

        // Get the hash of the latest block. It can be any block type.
        let parent_hash = blockchain.head_hash();

        // Calculate the seed for this block by signing the previous block seed with the validator
        // key.
        let prev_seed = blockchain.head().seed().clone();

        let skip_block_info = if skip_block_proof.is_some() {
            Some(SkipBlockInfo {
                block_number,
                vrf_entropy: prev_seed.entropy(),
            })
        } else {
            None
        };

        let seed = if skip_block_proof.is_some() {
            // VRF seed of a skip block is carried over since a new VRF seed would require a new
            // leader.
            prev_seed
        } else {
            prev_seed.sign_next_with_rng(&self.signing_key, rng)
        };

        // Create the inherents from the equivocation proofs or skip block info.
        let inherents = blockchain.create_punishment_inherents(
            block_number,
            &equivocation_proofs,
            skip_block_info,
            None,
        );

        // Update the state and calculate the state root.
        let block_state = BlockState::new(block_number, timestamp);
        let (state_root, diff_root, executed_txns) = blockchain
            .state
            .accounts
            .exercise_transactions(&transactions, &inherents, &block_state)
            .map_err(|error| {
                BlockProducerError::accounts_error(
                    blockchain,
                    error,
                    transactions,
                    inherents.clone(),
                )
            })?;

        // Calculate the historic transactions from the transactions and the inherents.
        let hist_txs = HistoricTransaction::from(
            blockchain.network_id,
            block_number,
            timestamp,
            executed_txns.clone(),
            inherents,
            equivocation_proofs
                .iter()
                .map(|proof| proof.locator())
                .collect(),
        );

        // Store the historic transactions into the history tree and calculate the history root.
        let mut txn = blockchain.write_transaction();

        let (history_root, _) = blockchain
            .history_store
            .add_to_history(&mut txn, block_number, &hist_txs)
            .ok_or(BlockProducerError::HistoryError)?;

        // Not strictly necessary to drop the lock here, but sign as well as compress might be somewhat expensive
        // and there is no need to hold the lock after this point.
        // Abort txn so that blockchain is no longer borrowed.
        txn.abort();

        // Create the micro block body.
        let body = MicroBody {
            equivocation_proofs,
            transactions: executed_txns,
        };

        // Create the micro block header.
        let header = MicroHeader {
            network,
            version: Policy::VERSION,
            block_number,
            timestamp,
            parent_hash,
            seed,
            extra_data,
            state_root,
            body_root: body.hash(),
            diff_root,
            history_root,
        };

        let justification = if let Some(skip_block_proof) = skip_block_proof {
            MicroJustification::Skip(skip_block_proof)
        } else {
            // Signs the block header using the signing key.
            let hash = header.hash::<Blake2bHash>();
            let signature = self.signing_key.sign(hash.as_slice());
            MicroJustification::Micro(signature)
        };

        // Returns the micro block.
        Ok(MicroBlock {
            header,
            body: Some(body),
            justification: Some(justification),
        })
    }

    /// Creates a proposal for the next macro block (checkpoint or election). It is just a proposal,
    /// NOT a complete block. It still needs to go through the Tendermint protocol in order to be
    /// finalized.
    // Note: Needs to be called with the Blockchain lock held.
    pub fn next_macro_block_proposal(
        &self,
        // The (upgradable) read locked guard to the blockchain
        blockchain: &Blockchain,
        // The timestamp for the block proposal.
        timestamp: u64,
        // The round for the block proposal.
        round: u32,
        // Extra data for this block.
        extra_data: Vec<u8>,
    ) -> Result<MacroBlock, BlockProducerError> {
        self.next_macro_block_proposal_with_rng(
            blockchain,
            timestamp,
            round,
            extra_data,
            &mut rand::thread_rng(),
        )
    }

    /// Creates a proposal for the next macro block (checkpoint or election). It is just a proposal,
    /// NOT a complete block. It still needs to go through the Tendermint protocol in order to be
    /// finalized.
    // Note: Needs to be called with the Blockchain lock held.
    pub fn next_macro_block_proposal_with_rng<R: RngCore + CryptoRng>(
        &self,
        // The (upgradable) read locked guard to the blockchain.
        blockchain: &Blockchain,
        // The timestamp for the block proposal.
        timestamp: u64,
        // The round for the block proposal.
        round: u32,
        // Extra data for this block.
        extra_data: Vec<u8>,
        // The rng seed. We need this parameterized in order to have determinism when running unit tests.
        rng: &mut R,
    ) -> Result<MacroBlock, BlockProducerError> {
        // The network ID stays unchanged for the whole blockchain.
        let network = blockchain.head().network();

        // Calculate the block number. It is simply the previous block number incremented by one.
        let block_number = blockchain.block_number() + 1;

        // Calculate the timestamp. It must be greater than or equal to the previous block
        // timestamp (i.e. time must not go back).
        let timestamp = u64::max(timestamp, blockchain.timestamp());

        // Get the hash of the latest block (it is by definition a micro block).
        let parent_hash = blockchain.head_hash();

        // Get the hash of the latest election macro block.
        let parent_election_hash = blockchain.election_head_hash();

        let interlink = if Policy::is_election_block_at(block_number) {
            Some(blockchain.election_head().get_next_interlink().unwrap())
        } else {
            None
        };

        // Calculate the seed for this block by signing the previous block seed with the validator
        // key.
        let seed = blockchain
            .head()
            .seed()
            .sign_next_with_rng(&self.signing_key, rng);

        // If this is an election block, calculate the validator set for the next epoch.
        let validators = match Policy::is_election_block_at(block_number) {
            true => Some(blockchain.next_validators(&seed)),
            false => None,
        };

        // Get the staking contract PRIOR to any state changes.
        let staking_contract = blockchain
            .get_staking_contract_if_complete(None)
            .expect("Staking Contract must be complete to create a macro body");

        // Calculate the punished set for the next batch.
        let next_batch_initial_punished_set = staking_contract
            .punished_slots
            .next_batch_initial_punished_set(block_number, &staking_contract.active_validators);

        // Create the header for the macro block without the state root and the transactions root.
        // We need several fields of this header in order to calculate the transactions and the
        // state.
        let mut header = MacroHeader {
            network,
            version: Policy::VERSION,
            block_number,
            round,
            timestamp,
            parent_hash,
            parent_election_hash,
            interlink,
            seed,
            extra_data,
            state_root: Blake2bHash::default(),
            body_root: Blake2sHash::default(),
            diff_root: Blake2bHash::default(),
            history_root: Blake2bHash::default(),
            validators,
            next_batch_initial_punished_set,
        };

        // Create the body for the macro block.
        let body = Self::next_macro_body(blockchain, &header, None)?;

        // Add the root of the body to the header.
        header.body_root = body.hash();

        // Returns the block proposal.
        let mut macro_block = MacroBlock {
            header,
            body: Some(body),
            justification: None,
        };

        let inherents: Vec<Inherent> = blockchain.create_macro_block_inherents(&macro_block);

        // Update the state and add the state root to the header.
        let block_state = BlockState::new(block_number, timestamp);
        let (state_root, diff_root, _) = blockchain
            .state
            .accounts
            .exercise_transactions(&[], &inherents, &block_state)
            .map_err(|error| {
                BlockProducerError::accounts_error(blockchain, error, vec![], inherents.clone())
            })?;

        macro_block.header.state_root = state_root;
        macro_block.header.diff_root = diff_root;

        // Calculate the historic transactions from the transactions and the inherents.
        let hist_txs = HistoricTransaction::from(
            blockchain.network_id,
            block_number,
            timestamp,
            vec![],
            inherents,
            vec![],
        );

        // Store the historic transactions into the history tree and calculate the history root.
        let mut txn = blockchain.write_transaction();

        macro_block.header.history_root = blockchain
            .history_store
            .add_to_history(&mut txn, block_number, &hist_txs)
            .ok_or(BlockProducerError::HistoryError)?
            .0;

        txn.abort();

        Ok(macro_block)
    }

    pub fn next_macro_body(
        blockchain: &Blockchain,
        macro_header: &MacroHeader,
        txn_option: Option<&DBTransaction>,
    ) -> Result<MacroBody, BlockProducerError> {
        // Get the staking contract PRIOR to any state changes.
        let staking_contract = blockchain
            .get_staking_contract_if_complete(txn_option)
            .ok_or(BlockProducerError::AccountsIncomplete)?;

        // Calculate the reward transactions.
        let reward_transactions =
            blockchain.create_reward_transactions(macro_header, &staking_contract);

        // Create the body for the macro block.
        Ok(MacroBody {
            transactions: reward_transactions,
        })
    }
}
