use account::Account;
use collections::LimitHashSet;
use nimiq_hash::{Blake2bHash, Hash};
use primitives::coin::Coin;
use transaction::{Transaction, TransactionFlags};

#[derive(Debug)]
pub struct MempoolFilter {
    blacklist: LimitHashSet<Blake2bHash>,
    rules: Rules,
}

impl MempoolFilter {
    pub const DEFAULT_BLACKLIST_SIZE: usize = 25000;

    pub fn new(rules: Rules, blacklist_limit: usize) -> Self {
        MempoolFilter {
            blacklist: LimitHashSet::new(blacklist_limit),
            rules,
        }
    }

    pub fn blacklist(&mut self, tx: &Transaction) -> &mut Self {
        self.blacklist.insert(tx.hash());
        self
    }

    pub fn remove(&mut self, tx: &Transaction) -> &mut Self {
        let hash: Blake2bHash = tx.hash();
        self.blacklist.remove(&hash);
        self
    }

    pub fn blacklisted(&self, tx: &Transaction) -> bool {
        let hash: Blake2bHash = tx.hash();
        self.blacklist.contains(&hash)
    }

    pub fn accepts_transaction(&self, tx: &Transaction) -> bool {
         tx.fee >= self.rules.tx_fee &&
             tx.value >= self.rules.tx_value &&
             tx.value + tx.fee >= self.rules.tx_value_total &&
             tx.fee_per_byte() >= self.rules.tx_fee_per_byte && (
                !tx.flags.contains(TransactionFlags::CONTRACT_CREATION) || (
                    tx.fee >= self.rules.contract_fee ||
                        tx.fee_per_byte() >= self.rules.contract_fee_per_byte ||
                        tx.value >= self.rules.contract_value
                )
         )
    }

    pub fn accepts_recipient_account(&self, tx: &Transaction, old_account: &Account, new_account: &Account) -> bool {
        new_account.balance() >= self.rules.recipient_balance && (
            !old_account.is_initial() ||
                (tx.fee >= self.rules.creation_fee &&
                    tx.fee_per_byte() >= self.rules.creation_fee_per_byte &&
                    tx.value >= self.rules.creation_value
                )
        )
    }

    pub fn accepts_sender_account(&self, _tx: &Transaction, _old_account: &Account, new_account: &Account) -> bool {
        new_account.balance() >= self.rules.sender_balance ||
            new_account.is_initial() ||
            new_account.is_to_be_pruned()
    }
}

impl Default for MempoolFilter{
    fn default() -> Self {
        MempoolFilter::new(Rules::default(), Self::DEFAULT_BLACKLIST_SIZE)
    }
}


#[derive(Debug, Clone)]
pub struct Rules {
    pub tx_fee: Coin,
    pub tx_fee_per_byte: f64,
    pub tx_value: Coin,
    pub tx_value_total: Coin,
    pub contract_fee: Coin,
    pub contract_fee_per_byte: f64,
    pub contract_value: Coin,
    pub creation_fee: Coin,
    pub creation_fee_per_byte: f64,
    pub creation_value: Coin,
    pub recipient_balance: Coin,
    pub sender_balance: Coin,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            tx_fee: Coin::ZERO,
            tx_fee_per_byte: 0.0,
            tx_value: Coin::ZERO,
            tx_value_total: Coin::ZERO,
            contract_fee: Coin::ZERO,
            contract_fee_per_byte: 0.0,
            contract_value: Coin::ZERO,
            creation_fee: Coin::ZERO,
            creation_fee_per_byte: 0.0,
            creation_value: Coin::ZERO,
            sender_balance: Coin::ZERO,
            recipient_balance: Coin::ZERO,
        }
    }
}
