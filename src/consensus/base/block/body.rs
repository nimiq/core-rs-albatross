use beserial::{Deserialize, Serialize};
use consensus::base::account::PrunedAccount;
use consensus::base::primitive::Address;
use consensus::base::primitive::hash::{Hash, Hasher, SerializeContent};
use consensus::base::transaction::Transaction;
use utils::merkle;
use std::io;
use consensus::base::primitive::hash::Blake2bHash;
use consensus::base::primitive::hash::Blake2bHasher;

#[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BlockBody {
    pub miner: Address,
    #[beserial(len_type(u8))]
    pub extra_data: Vec<u8>,
    #[beserial(len_type(u16))]
    pub transactions: Vec<Transaction>,
    #[beserial(len_type(u16))]
    pub pruned_accounts: Vec<PrunedAccount>,
}

/*impl<W: io::Write> SerializeContent<W> for BlockBody {
    fn serialize_content(&self, writer: &mut W) -> io::Result<usize> { self.serialize(writer) }
}

impl BlockBody {
    fn hash<H: Hasher>(&self) -> Blake2bHash {
        let vec: Vec<&Hash<H>> = Vec::with_capacity(2 + self.transactions.len() + self.pruned_accounts.len());
        vec.push(&self.miner);
        vec.push(&self.extra_data);
        return merkle::compute_root_ref::<Blake2bHasher, Hash<H>>(&vec);
    }
}*/
