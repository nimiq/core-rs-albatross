use nimiq_keys::Address;
use nimiq_primitives::{account::AccountType, policy::Policy};
use nimiq_serde::Serialize;
use nimiq_transaction::{
    account::{
        bridge_contract::{OutgoingBridgeTransactionData, OutgoingTransaction},
        staking_contract::OutgoingStakingTransactionData,
    },
    SignatureProof,
};

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub enum Sender {
    Basic {
        address: Address,
    },
    Htlc {
        address: Address,
    },
    Vesting {
        address: Address,
    },
    Staking {
        data: OutgoingStakingTransactionData,
    },
    Oracle {
        address: Address,
    },
    Bridge {
        address: Address,
        data: OutgoingBridgeTransactionData,
    },
}

impl Sender {
    pub fn new_basic(address: Address) -> Self {
        Sender::Basic { address }
    }

    pub fn new_htlc(address: Address) -> Self {
        Sender::Htlc { address }
    }

    pub fn new_vesting(address: Address) -> Self {
        Sender::Vesting { address }
    }

    pub fn new_staking_builder() -> StakingSenderBuilder {
        StakingSenderBuilder::new()
    }

    /// Creates a sender that withdraws the deposit of the oracle contract at `address`.
    pub fn new_oracle(address: Address) -> Self {
        Sender::Oracle { address }
    }

    /// Creates a sender that releases funds from the bridge contract at `address` against the
    /// given `burn_proof`. The burn proof still has to be signed using a [`BridgeProofBuilder`].
    ///
    /// [`BridgeProofBuilder`]: crate::proof::bridge_contract::BridgeProofBuilder
    pub fn new_bridge(address: Address, burn_proof: OutgoingTransaction) -> Self {
        Sender::Bridge {
            address,
            data: OutgoingBridgeTransactionData {
                burn_proof,
                proof: SignatureProof::default(),
            },
        }
    }

    pub fn account_type(&self) -> AccountType {
        match self {
            Sender::Basic { .. } => AccountType::Basic,
            Sender::Htlc { .. } => AccountType::HTLC,
            Sender::Vesting { .. } => AccountType::Vesting,
            Sender::Staking { .. } => AccountType::Staking,
            Sender::Oracle { .. } => AccountType::Oracle,
            Sender::Bridge { .. } => AccountType::Bridge,
        }
    }

    /// Returns the recipient address if this is not a contract creation.
    pub fn address(&self) -> Address {
        match self {
            Sender::Basic { address }
            | Sender::Htlc { address }
            | Sender::Vesting { address }
            | Sender::Oracle { address }
            | Sender::Bridge { address, .. } => address.clone(),
            Sender::Staking { .. } => Policy::STAKING_CONTRACT_ADDRESS,
        }
    }

    /// Returns the data field for the transaction.
    pub fn data(&self) -> Vec<u8> {
        match self {
            Sender::Basic { .. }
            | Sender::Htlc { .. }
            | Sender::Vesting { .. }
            | Sender::Oracle { .. } => vec![],
            Sender::Staking { data } => data.serialize_to_vec(),
            Sender::Bridge { data, .. } => data.serialize_to_vec(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StakingSenderBuilder {
    data: Option<OutgoingStakingTransactionData>,
}

impl StakingSenderBuilder {
    /// Creates a new `StakingRecipientBuilder`.
    pub fn new() -> Self {
        Self {
            data: Default::default(),
        }
    }

    pub fn delete_validator(mut self) -> Self {
        self.data = Some(OutgoingStakingTransactionData::DeleteValidator);
        self
    }

    pub fn remove_stake(mut self) -> Self {
        self.data = Some(OutgoingStakingTransactionData::RemoveStake);
        self
    }

    pub fn generate(self) -> Option<Sender> {
        Some(Sender::Staking { data: self.data? })
    }
}
