use account::Account;
use collections::LimitHashSet;
use keys::Address;
use nimiq_hash::{Blake2bHash, Hash};
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use transaction::{Transaction, TransactionFlags};



#[derive(Debug)]
pub struct MempoolFilter {
    blacklist: LimitHashSet<Blake2bHash>,
    rules: Rules,
}

impl MempoolFilter {

    pub const DEFAULT_RULES: Rules = Rules {
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
    };

    pub const DEFAULT_BLACKLIST_SIZE: usize = 25000;

    pub fn new(rules: Rules, blacklist_limit: usize) -> Self {
        let b: LimitHashSet<Blake2bHash> = LimitHashSet::new(blacklist_limit);
        MempoolFilter {blacklist: b, rules}
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

    pub fn accepts_transaction(& self, tx: &Transaction) -> bool {
         if tx.fee < self.rules.tx_fee ||
             tx.value < self.rules.tx_value ||
             tx.value + tx.fee < self.rules.tx_value_total ||
             tx.fee_per_byte() < self.rules.tx_fee_per_byte {
             return false;
         } else {
             match tx.flags {
                 TransactionFlags::CONTRACT_CREATION => {
                     if (tx.fee < self.rules.contract_fee) ||
                         (tx.fee_per_byte() < self.rules.contract_fee_per_byte) ||
                         (tx.value < self.rules.contract_value) {
                         return false
                     } else {
                         return  true
                     }
                 },
                 _ => return true,
             }
         }
    }

    pub fn accepts_recipient_account(&self, tx: &Transaction, old_account: &Account, new_account: &Account) -> bool {
        new_account.balance() >= self.rules.recipient_balance && (
            !old_account.is_initial() ||
                (tx.fee >= self.rules.creation_fee &&
                    tx.fee_per_byte() >= self.rules.creation_fee_per_byte &&
                    tx.value >= self.rules.creation_value ))
    }

    pub fn accepts_sender_account(&self, tx: &Transaction, old_account: &Account, new_account: &Account) -> bool {
        new_account.balance() >= self.rules.sender_balance ||
            new_account.is_initial() ||
            new_account.is_to_be_pruned()
    }
}

#[derive(Debug)]
pub struct Rules {
    tx_fee: Coin,
    tx_fee_per_byte: f64,
    tx_value: Coin,
    tx_value_total: Coin,
    contract_fee: Coin,
    contract_fee_per_byte: f64,
    contract_value: Coin,
    creation_fee: Coin,
    creation_fee_per_byte: f64,
    creation_value: Coin,
    recipient_balance: Coin,
    sender_balance: Coin,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_can_blacklist_transactions() {
        let s: Rules = MempoolFilter::DEFAULT_RULES;

        let mut f = MempoolFilter::new(s, 25000);

        let tx = Transaction::new_basic(
            Address::from([32u8; Address::SIZE]),
            Address::from([213u8; Address::SIZE]),
            Coin::from(100),
            Coin::from(1),
            123,
            NetworkId::Main,
        );

        f.blacklist(&tx);
        assert!(f.blacklisted(&tx));
        f.remove(&tx);
        assert!(!f.blacklisted(&tx));
    }

    #[test]
    fn it_accepts_and_rejects_transactions() {
        let mut s: Rules = MempoolFilter::DEFAULT_RULES;
        s.tx_fee = Coin::from(1);

        let f = MempoolFilter::new(s, 25000);

        let mut tx = Transaction::new_basic(
            Address::from([32u8; Address::SIZE]),
            Address::from([213u8; Address::SIZE]),
            Coin::from(0),
            Coin::from(0),
            0,
            NetworkId::Main,
        );

        assert!(!f.accepts_transaction(&tx));
        tx.fee = Coin::from(1);
        assert!(f.accepts_transaction(&tx));
    }
}
