pub mod account;
pub mod block;
pub mod blockchain;
pub mod mempool;
pub mod primitive;
pub mod transaction;


use beserial::{Deserialize, Serialize, ReadBytesExt, WriteBytesExt, SerializingError};
use crate::consensus::base::primitive::Address;



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
    Addresses(Vec<Address>),
    MinFee(u64),
}


impl Subscription {
    pub fn get_subscription_type(&self) -> SubscriptionType {
        match self {
            Subscription::None => SubscriptionType::None,
            Subscription::Any => SubscriptionType::Any,
            Subscription::Addresses(_) => SubscriptionType::Addresses,
            Subscription::MinFee(_) => SubscriptionType::MinFee
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
                let mut addresses = Vec::with_capacity(num_addresses);
                for i in 0..num_addresses {
                    let address: Address = Deserialize::deserialize(reader)?;
                    addresses.push(address)
                }

                Ok(Subscription::Addresses(addresses))
            },

            SubscriptionType::MinFee => {
                let min_fee: u64 = Deserialize::deserialize(reader)?;
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
        size += Serialize::serialize(&self.get_subscription_type(), writer)?;

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
