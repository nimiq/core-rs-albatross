use linked_hash_map::LinkedHashMap;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::{Transaction, TransactionFlags};

/// Struct defining a Mempool filter
#[derive(Debug)]
pub struct MempoolFilter {
    pub(crate) blacklist: LinkedHashMap<Blake2bHash, ()>,
    pub(crate) blacklist_limit: usize,
    pub(crate) rules: MempoolRules,
}

impl MempoolFilter {
    /// Constant defining the default size for the blacklist
    pub const DEFAULT_BLACKLIST_SIZE: usize = 25000;

    /// Creates a new MempoolFilter
    pub fn new(rules: MempoolRules, blacklist_limit: usize) -> Self {
        MempoolFilter {
            blacklist: LinkedHashMap::new(),
            blacklist_limit,
            rules,
        }
    }

    /// Blacklists a new transaction given its hash
    pub fn blacklist(&mut self, hash: Blake2bHash) -> &mut Self {
        while self.blacklist.len() >= self.blacklist_limit {
            self.blacklist.pop_front();
        }
        self.blacklist.insert(hash, ());
        self
    }

    /// Removes a transaction from the blacklist
    pub fn remove(&mut self, hash: &Blake2bHash) -> &mut Self {
        self.blacklist.remove(hash);
        self
    }

    /// Checks whether a transaction is blacklisted
    pub fn blacklisted(&self, hash: &Blake2bHash) -> bool {
        self.blacklist.contains_key(hash)
    }

    /// Checks whether a transaction is accepted according to the general Mempool filter rules
    ///
    /// The following rules are checked in this function:
    /// - tx_fee
    /// - tx_value
    /// - tx_value_total
    /// - tx_fee_per_byte
    /// - contract_fee
    /// - contract_fee
    /// - contract_fee_per_byte
    /// - contract_value
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

    /// Checks whether a transaction is accepted according to the Mempool filter rules for the recipient balance
    pub fn accepts_recipient_balance(
        &self,
        tx: &Transaction,
        old_balance: Coin,
        new_balance: Coin,
    ) -> bool {
        new_balance >= self.rules.recipient_balance
            && (
                // XXX This does not precisely capture Account::is_initial() as it will always classify
                // contracts with zero value as non-existent.
                old_balance != Coin::ZERO
                    || (tx.fee >= self.rules.creation_fee
                        && tx.fee_per_byte() >= self.rules.creation_fee_per_byte
                        && tx.value >= self.rules.creation_value)
            )
    }

    /// Checks whether a transaction is accepted according to the Mempool filter rules for the sender balance
    pub fn accepts_sender_balance(
        &self,
        _tx: &Transaction,
        _old_balance: Coin,
        new_balance: Coin,
    ) -> bool {
        new_balance >= self.rules.sender_balance ||
            // XXX This does not precisely capture Account::is_initial() || Account.is_to_be_pruned()
            // as it will ignore contracts that will not be pruned with zero value.
            new_balance == Coin::ZERO
    }
}

impl Default for MempoolFilter {
    fn default() -> Self {
        MempoolFilter::new(MempoolRules::default(), Self::DEFAULT_BLACKLIST_SIZE)
    }
}

/// Struct defining a Mempool rule
#[derive(Debug, Clone)]
pub struct MempoolRules {
    /// Minimum fee for all transactions
    pub tx_fee: Coin,
    /// Minimum fee per byte for all transactions
    pub tx_fee_per_byte: f64,
    /// Minimum value for all transactions
    pub tx_value: Coin,
    /// Minimum total value (value + fee) for all transactions
    pub tx_value_total: Coin,
    /// Minimum fee for transactions creating a contract
    pub contract_fee: Coin,
    /// Minimum fee per byte for transactions creating a contract
    pub contract_fee_per_byte: f64,
    /// Minimum value for transactions creating a contract
    pub contract_value: Coin,
    /// Minimum fee for transactions creating a new account
    pub creation_fee: Coin,
    /// Minimum fee per byte for transactions creating a new account
    pub creation_fee_per_byte: f64,
    /// Minimum value for transactions creating a new account
    pub creation_value: Coin,
    /// Minimum balance that the recipient account must have after the transaction
    pub recipient_balance: Coin,
    /// Minimum balance that must remain on the sender account after the transaction, if not zero
    pub sender_balance: Coin,
}

impl Default for MempoolRules {
    fn default() -> MempoolRules {
        MempoolRules {
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
