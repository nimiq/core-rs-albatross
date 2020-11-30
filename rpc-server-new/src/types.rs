///! Defines the types used by the JSON RPC API[1]
///!
///! [1] https://github.com/nimiq/core-js/wiki/JSON-RPC-API#common-data-types
use std::fmt::{Display, Formatter};

use nimiq_blockchain_albatross::Blockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::policy;
use nimiq_primitives::{account::AccountType, coin::Coin};
use nimiq_transaction::account::htlc_contract::AnyHash;

use nimiq_block_albatross::{TendermintProof, ViewChangeProof};
use nimiq_bls::{CompressedPublicKey, CompressedSignature};
use nimiq_collections::BitSet;
use nimiq_primitives::slot::{SlotBand, ValidatorSlots};
use nimiq_vrf::VrfSeed;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    Macro,
    Micro,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    #[serde(rename = "type")]
    ty: BlockType,

    hash: Blake2bHash,

    block_number: u32,

    view_number: u32,

    batch: u32,

    epoch: u32,

    parent_hash: Blake2bHash,

    seed: VrfSeed,

    state_root: Blake2bHash,

    body_root: Blake2bHash,

    timestamp: u64,

    #[serde(flatten)]
    additional_fields: BlockAdditionalFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum BlockAdditionalFields {
    Macro {
        is_election_block: bool,

        parent_election_hash: Blake2bHash,

        // None if not an election block
        slots: Option<Vec<Slots>>,

        #[serde(skip_serializing_if = "Option::is_none")]
        lost_reward_set: Option<BitSet>,

        #[serde(skip_serializing_if = "Option::is_none")]
        justification: Option<MacroJustification>,
    },
    Micro {
        #[serde(with = "crate::serde_helpers::hex")]
        extra_data: Vec<u8>,

        #[serde(skip_serializing_if = "Option::is_none")]
        fork_proofs: Option<Vec<ForkProof>>,

        #[serde(skip_serializing_if = "Option::is_none")]
        transactions: Option<Vec<Transaction>>,

        producer: Slot,

        // TODO: Can this really be None? I think it's an `Option` in the actual block, because a block can be built
        // without justification, then signed and the justification attached to it.
        justification: Option<MicroJustification>,
    },
}

/// Likely no longer necessary and can be repalced by TendermintProof directly as
/// the vote count in terms of slots is encoded in there.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroJustification {
    votes: u16,

    #[serde(flatten)]
    tendermint_proof: TendermintProof,
}

impl MacroJustification {
    fn from_pbft_proof(
        tendermint_proof_opt: Option<TendermintProof>,
        _validator_slots_opt: Option<&ValidatorSlots>,
    ) -> Option<Self> {
        if let Some(tendermint_proof) = tendermint_proof_opt {
            let votes = tendermint_proof.votes();
            Some(Self {
                votes,
                tendermint_proof,
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slots {
    first_slot_number: u16,

    num_slots: u16,

    public_key: CompressedPublicKey,

    #[serde(with = "crate::serde_helpers::address_friendly")]
    reward_address: Address,
}

impl Slots {
    pub fn from_slots(slots: nimiq_primitives::slot::Slots) -> Vec<Slots> {
        slots
            .combined()
            .into_iter()
            .map(|(slot, first_slot_number)| Slots {
                first_slot_number,
                num_slots: slot.validator_slot.num_slots(),
                public_key: slot.public_key().compressed().clone(),
                reward_address: slot.reward_address().clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroJustification {
    #[serde(with = "serde_with::rust::display_fromstr")]
    signature: CompressedSignature,

    view_change_proof: Option<ViewChangeProof>,
}

impl From<nimiq_block_albatross::MicroJustification> for MicroJustification {
    fn from(justification: nimiq_block_albatross::MicroJustification) -> Self {
        Self {
            signature: justification.signature,
            view_change_proof: justification.view_change_proof,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    slot_number: u16,

    public_key: CompressedPublicKey,

    #[serde(with = "crate::serde_helpers::address_friendly")]
    reward_address: Address,
}

impl Slot {
    pub fn from_producer(blockchain: &Blockchain, block_number: u32, view_number: u32) -> Self {
        // TODO: `get_slot_owner_at` should really return an `Option` or `Result`. This will panic, when there is no
        // slot owner.
        let (slot, slot_number) = blockchain.get_slot_owner_at(block_number, view_number, None);

        Slot {
            slot_number,
            public_key: slot.public_key().compressed().clone(),
            reward_address: slot.reward_address().clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkProof {
    block_number: u32,
    view_number: u32,
    parent_hash: Blake2bHash,
    hashes: [Blake2bHash; 2],
}

impl From<nimiq_block_albatross::ForkProof> for ForkProof {
    fn from(fork_proof: nimiq_block_albatross::ForkProof) -> Self {
        let hashes = [fork_proof.header1.hash(), fork_proof.header2.hash()];

        Self {
            block_number: fork_proof.header1.block_number,
            view_number: fork_proof.header1.view_number,
            parent_hash: fork_proof.header1.parent_hash,
            hashes,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    hash: Blake2bHash,

    block_hash: Blake2bHash,

    block_number: u32,

    timestamp: u64,

    confirmations: u32,

    transaction_index: usize,

    // TODO: `from` and `from_address` can be merged into one Address that is a flattened serialization with a
    // special serilizer and some helper to fix the field names.... I think
    #[serde(with = "crate::serde_helpers::address_hex")]
    from: Address,

    #[serde(with = "crate::serde_helpers::address_friendly")]
    from_address: Address,

    #[serde(with = "crate::serde_helpers::address_hex")]
    to: Address,

    #[serde(with = "crate::serde_helpers::address_friendly")]
    to_address: Address,

    value: Coin,

    fee: Coin,

    #[serde(with = "crate::serde_helpers::hex")]
    data: Vec<u8>,

    flags: u8,

    validity_start_height: u32,
}

impl Transaction {
    pub fn from_blockchain(
        transaction: nimiq_transaction::Transaction,
        transaction_index: usize,
        block_hash: &Blake2bHash,
        block_number: u32,
        timestamp: u64,
        head_height: u32,
    ) -> Self {
        Transaction {
            hash: transaction.hash(),
            block_hash: block_hash.clone(),
            block_number,
            timestamp,
            confirmations: head_height.saturating_sub(block_number),
            transaction_index,
            from: transaction.sender.clone(),
            from_address: transaction.sender,
            to: transaction.recipient.clone(),
            to_address: transaction.recipient,
            value: transaction.value,
            fee: transaction.fee,
            flags: transaction.flags.bits() as u8,
            data: transaction.data,
            validity_start_height: transaction.validity_start_height,
        }
    }
}

impl Block {
    pub fn from_block(
        blockchain: &Blockchain,
        block: nimiq_block_albatross::Block,
        include_transactions: bool,
    ) -> Self {
        let block_hash = block.hash();
        let block_number = block.block_number();
        let batch = policy::batch_at(block_number);
        let epoch = policy::epoch_at(block_number);
        let view_number = block.view_number();
        let timestamp = block.timestamp();

        match block {
            nimiq_block_albatross::Block::Macro(macro_block) => {
                let validator_slots_opt = blockchain
                    .get_block(&macro_block.header.parent_election_hash, true)
                    .and_then(|block| block.body())
                    .and_then(|body| body.unwrap_macro().validators);

                let slots = macro_block
                    .get_slots()
                    .map(|slots| Slots::from_slots(slots));

                Block {
                    ty: BlockType::Macro,
                    hash: block_hash,
                    block_number,
                    view_number,
                    batch,
                    epoch,
                    parent_hash: macro_block.header.parent_hash,
                    seed: macro_block.header.seed,
                    state_root: macro_block.header.state_root,
                    body_root: macro_block.header.body_root,
                    timestamp,
                    additional_fields: BlockAdditionalFields::Macro {
                        is_election_block: policy::is_election_block_at(block_number),
                        parent_election_hash: macro_block.header.parent_election_hash,
                        slots,
                        lost_reward_set: macro_block.body.map(|body| body.lost_reward_set),
                        justification: MacroJustification::from_pbft_proof(
                            macro_block.justification,
                            validator_slots_opt.as_ref(),
                        ),
                    },
                }
            }

            nimiq_block_albatross::Block::Micro(micro_block) => {
                let (fork_proofs, transactions) = if let Some(body) = micro_block.body {
                    (
                        Some(body.fork_proofs.into_iter().map(Into::into).collect()),
                        if include_transactions {
                            let head_height = blockchain.block_number();
                            Some(
                                body.transactions
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, tx)| {
                                        Transaction::from_blockchain(
                                            tx,
                                            index,
                                            &block_hash,
                                            block_number,
                                            timestamp,
                                            head_height,
                                        )
                                    })
                                    .collect(),
                            )
                        } else {
                            None
                        },
                    )
                } else {
                    (None, None)
                };

                Block {
                    ty: BlockType::Micro,
                    hash: block_hash,
                    block_number,
                    view_number,
                    batch,
                    epoch,
                    parent_hash: micro_block.header.parent_hash,
                    seed: micro_block.header.seed,
                    state_root: micro_block.header.state_root,
                    body_root: micro_block.header.body_root,
                    timestamp,
                    additional_fields: BlockAdditionalFields::Micro {
                        extra_data: micro_block.header.extra_data,
                        fork_proofs,
                        transactions,
                        producer: Slot::from_producer(blockchain, block_number, view_number),
                        justification: micro_block.justification.map(Into::into),
                    },
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    id: String,

    #[serde(with = "crate::serde_helpers::address_friendly")]
    address: Address,

    balance: Coin,

    #[serde(rename = "type", with = "crate::serde_helpers::account_type")]
    ty: AccountType,

    #[serde(flatten)]
    account_additional_fields: AccountAdditionalFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum AccountAdditionalFields {
    /// Additional account information for vesting contracts.
    VestingContract {
        /// Hex-encoded 20 byte address of the owner of the vesting contract.
        #[serde(with = "crate::serde_helpers::address_hex")]
        owner: Address,

        /// User friendly address (NQ-address) of the owner of the vesting contract.
        #[serde(with = "crate::serde_helpers::address_friendly")]
        owner_address: Address,

        /// The block that the vesting contracted commenced.
        vesting_start: u32,

        /// The number of blocks after which some part of the vested funds is released.
        vesting_step_blocks: u32,

        /// The amount (in Luna) released every vestingStepBlocks blocks.
        vesting_step_amount: Coin,

        /// The total amount (in smallest unit) that was provided at the contract creation.
        vesting_total_amount: Coin,
    },

    /// Additional account information for HTLC contracts.
    HTLC {
        /// Hex-encoded 20 byte address of the sender of the HTLC.
        #[serde(with = "crate::serde_helpers::address_hex")]
        sender: Address,

        /// User friendly address (NQ-address) of the sender of the HTLC.
        #[serde(with = "crate::serde_helpers::address_friendly")]
        sender_address: Address,

        /// Hex-encoded 20 byte address of the recipient of the HTLC.
        #[serde(with = "crate::serde_helpers::address_hex")]
        recipient: Address,

        /// User friendly address (NQ-address) of the recipient of the HTLC.
        #[serde(with = "crate::serde_helpers::address_friendly")]
        recipient_address: Address,

        /// Hex-encoded 32 byte hash root.
        #[serde(with = "serde_with::rust::display_fromstr")]
        hash_root: AnyHash,

        /// Number of hashes this HTLC is split into
        hash_count: u8,

        /// Block after which the contract can only be used by the original sender to recover funds.
        timeout: u32,

        /// The total amount (in smallest unit) that was provided at the contract creation.
        total_amount: Coin,
    },
    Staking {
        // TODO
    },
}

#[derive(Clone, Debug)]
pub enum OrLatest<T> {
    Value(T),
    Latest,
}

impl<T> Serialize for OrLatest<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            OrLatest::Value(x) => x.serialize(serializer),
            OrLatest::Latest => serializer.serialize_str("latest"),
        }
    }
}

impl<'de, T> Deserialize<'de> for OrLatest<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        unimplemented!()
    }
}

impl<T> Display for OrLatest<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        match self {
            OrLatest::Value(x) => x.fmt(f),
            OrLatest::Latest => write!(f, "latest"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashedSlots {
    pub block_number: u32,

    pub current: BitSet,

    pub previous: BitSet,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stakes {
    pub active_validators: Vec<Validator>,

    pub inactive_validators: Vec<Validator>,

    pub inactive_stakes: Vec<Stake>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stake {
    #[serde(with = "crate::serde_helpers::address_friendly")]
    pub staker_address: Address,

    pub balance: Coin,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub retire_time: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Validator {
    pub public_key: CompressedPublicKey,

    pub balance: Coin,

    #[serde(with = "crate::serde_helpers::address_friendly")]
    pub reward_address: Address,

    pub stakes: Vec<Stake>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub retire_time: Option<u32>,
}

impl Validator {
    pub fn from_validator(
        public_key: CompressedPublicKey,
        validator: &nimiq_account::staking_contract::Validator,
        retire_time: Option<u32>,
    ) -> Self {
        Validator {
            public_key: public_key.clone(),
            balance: validator.balance,
            reward_address: validator.reward_address.clone(),
            stakes: validator
                .active_stake_by_address
                .read()
                .iter()
                .map(|(address, balance)| Stake {
                    staker_address: address.clone(),
                    balance: *balance,
                    retire_time: None,
                })
                .collect(),
            retire_time: None,
        }
    }

    pub fn from_active(
        public_key: CompressedPublicKey,
        validator: &nimiq_account::staking_contract::Validator,
    ) -> Self {
        Self::from_validator(public_key, validator, None)
    }

    pub fn from_inactive(
        public_key: CompressedPublicKey,
        validator: &nimiq_account::staking_contract::InactiveValidator,
    ) -> Self {
        Self::from_validator(
            public_key,
            &validator.validator,
            Some(validator.retire_time),
        )
    }
}
