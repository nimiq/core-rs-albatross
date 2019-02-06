use beserial::{Deserialize, Serialize, ReadBytesExt, WriteBytesExt, SerializingError};
use keys::Address;
use primitives::block::Block;
use primitives::transaction::Transaction;
use primitives::coin::Coin;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SubscriptionType {
    None = 0,
    Any = 1,
    Addresses = 2,
    MinFee = 3
}


#[derive(Clone, Debug)]
pub enum Subscription {
    None,
    Any,
    Addresses(HashSet<Address>),
    MinFee(Coin),
}


impl Subscription {
    pub fn subscription_type(&self) -> SubscriptionType {
        match self {
            Subscription::None => SubscriptionType::None,
            Subscription::Any => SubscriptionType::Any,
            Subscription::Addresses(_) => SubscriptionType::Addresses,
            Subscription::MinFee(_) => SubscriptionType::MinFee
        }
    }

    pub fn matches_block(&self, block: Block) -> bool {
        match self {
            Subscription::None => false,
            _ => true
        }
    }

    pub fn matches_transaction(&self, transaction: Transaction) -> bool {
        match self {
            Subscription::None => false,
            Subscription::Any => true,
            Subscription::Addresses(addresses) => addresses.contains(&transaction.sender),
            Subscription::MinFee(min_fee) => {
                // TODO: Unchecked type conversion
                // NOTE: If the fee for this block overflows an u64, it's definetly worth mining it ;)
                min_fee.checked_factor(transaction.serialized_size() as u64)
                    .map(|block_fee| transaction.fee >= block_fee)
                    .unwrap_or(true)
            }
        }
    }
}



impl Deserialize for Subscription {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let sub_type: SubscriptionType = Deserialize::deserialize(reader)?;

        match sub_type {
            SubscriptionType::Addresses => {
                // parse number of addresses and cast to usize
                // FIXME We should check for an overflow
                let num_addresses: u16 = Deserialize::deserialize(reader)?;
                let num_addresses = (num_addresses as usize);

                // parse addresses and push them into vector
                let mut addresses = HashSet::with_capacity(num_addresses);
                for i in 0..num_addresses {
                    let address: Address = Deserialize::deserialize(reader)?;
                    addresses.insert(address);
                }

                Ok(Subscription::Addresses(addresses))
            },

            SubscriptionType::MinFee => {
                let min_fee: Coin = Deserialize::deserialize(reader)?;
                Ok(Subscription::MinFee(min_fee))
            },

            SubscriptionType::None => Ok(Subscription::None),
            SubscriptionType::Any => Ok(Subscription::Any)

        }
    }
}



impl Serialize for Subscription {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size: usize = 0;

        // Serialize subscription type
        size += Serialize::serialize(&self.subscription_type(), writer)?;

        match self {
            Subscription::Addresses(addresses) => {
                // Serialize number of addresses
                if addresses.len() > 0xFFFF {
                    return Err(SerializingError::Overflow)
                }
                size += Serialize::serialize(&(addresses.len() as u16), writer)?;

                // Serialize addresses
                for address in addresses {
                    size += Serialize::serialize(address, writer)?;
                }
            },

            Subscription::MinFee(min_fee) => {
                // Serialize minFee
                size += Serialize::serialize(min_fee, writer)?;
            }

            _ => {}
        }

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        1 + (match self {
            // 2 bytes #n addresses, 20 * #n for the addresses
            // XXX This ignores an overflow of number of addresses
            Subscription::Addresses(addresses) => 2 + 20 * addresses.len(),

            // 64 bit minFee value
            Subscription::MinFee(_) => 8,

            _ => 0
        })
    }
}
