use beserial::{Deserialize, Serialize};
use consensus::base::account::PrunedAccount;
use consensus::base::primitive::Address;
use consensus::base::primitive::hash::{Hash, HashOutput, SerializeContent};
use consensus::base::transaction::Transaction;
use std::io;
use utils::merkle;

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

impl SerializeContent for BlockBody {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { self.serialize(writer) }
}

impl Hash for BlockBody {
    fn hash<H: HashOutput>(&self) -> H {
        let mut vec: Vec<H> = Vec::with_capacity(2 + self.transactions.len() + self.pruned_accounts.len());
        vec.push(self.miner.hash());
        vec.push(self.extra_data.hash());
        for t in &self.transactions {
            vec.push(t.hash());
        }
        for p in &self.pruned_accounts {
            vec.push(p.hash());
        }
        return merkle::compute_root_from_hashes::<H>(&vec);
    }
}
