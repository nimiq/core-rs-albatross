use collections::LimitHashSet;
use nimiq_hash::Blake2bHash;
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

    pub fn blacklist(&mut self, hash: Blake2bHash) -> &mut Self {
        self.blacklist.insert(hash);
        self
    }

    pub fn remove(&mut self, hash: &Blake2bHash) -> &mut Self {
        self.blacklist.remove(hash);
        self
    }

    pub fn blacklisted(&self, hash: &Blake2bHash) -> bool {
        self.blacklist.contains(hash)
    }

    pub fn accepts_transaction(&self, tx: &Transaction) -> bool {
        tx.fee >= self.rules.tx_fee &&
             tx.value >= self.rules.tx_value &&
             // Unchecked addition of coins.
             tx.value + tx.fee >= self.rules.tx_value_total &&
             tx.fee_per_byte() >= self.rules.tx_fee_per_byte && (
                !tx.flags.contains(TransactionFlags::CONTRACT_CREATION) || (
                    tx.fee >= self.rules.contract_fee ||
                        tx.fee_per_byte() >= self.rules.contract_fee_per_byte ||
                        tx.value >= self.rules.contract_value
                )
         )
    }

    pub fn accepts_recipient_balance(&self, tx: &Transaction, old_balance: Coin, new_balance: Coin) -> bool {
        new_balance >= self.rules.recipient_balance
            && (
                // XXX This does not precisely capture Account::is_initial() as it will always classify
                // contracts with zero value as non-existent.
                old_balance != Coin::ZERO
                    || (tx.fee >= self.rules.creation_fee && tx.fee_per_byte() >= self.rules.creation_fee_per_byte && tx.value >= self.rules.creation_value)
            )
    }

    pub fn accepts_sender_balance(&self, _tx: &Transaction, _old_balance: Coin, new_balance: Coin) -> bool {
        new_balance >= self.rules.sender_balance ||
            // XXX This does not precisely capture Account::is_initial() || Account.is_to_be_pruned()
            // as it will ignore contracts that will not be pruned with zero value.
            new_balance == Coin::ZERO
    }
}

impl Default for MempoolFilter {
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
