use std::ops::Deref;

use log::info;
use nimiq_account::{Account, Accounts, BlockLogger, BlockState, Receipts};
use nimiq_database::{volatile::VolatileEnvironment, WriteTransaction};
use nimiq_keys::Address;
use nimiq_primitives::{
    account::AccountError, coin::Coin, key_nibbles::KeyNibbles, networks::NetworkId,
    policy::Policy, slots::SlashedSlot,
};
use nimiq_test_log::test;
use nimiq_test_utils::transactions::{
    IncomingType, OutgoingType, TransactionsGenerator, ValidatorState,
};
use nimiq_transaction::{inherent::Inherent, Transaction};
use rand::Rng;

pub struct TestCommitRevert {
    accounts: Accounts,
}

impl TestCommitRevert {
    pub fn new() -> Self {
        let env = VolatileEnvironment::new(10).unwrap();
        let accounts = Accounts::new(env.clone());
        TestCommitRevert { accounts }
    }

    pub fn with_initial_state(initial_accounts: &[(Address, Account)]) -> Self {
        let test = Self::new();
        let mut txn = WriteTransaction::new(&test.accounts.env);

        for (address, account) in initial_accounts {
            test.accounts
                .tree
                .put(&mut txn, &KeyNibbles::from(address), account)
                .expect("Failed to put initial accounts");
        }

        txn.commit();

        test
    }

    /// Commits the input, then checks that a revert would bring the accounts trie back to the old state.
    /// This keeps the commit part and returns the result of the commit.
    pub fn commit_and_test(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
        block_logger: &mut BlockLogger,
    ) -> Result<Receipts, AccountError> {
        self.commit_revert(transactions, inherents, block_state, block_logger, true)
    }

    /// Commits and reverts the input, then checks that the accounts trie is back to the old state.
    /// This doesn't change the state of the accounts. Neither commit nor revert are allowed to fail.
    pub fn test(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
    ) {
        self.commit_revert(
            transactions,
            inherents,
            block_state,
            &mut BlockLogger::empty(),
            false,
        )
        .expect("Failed to commit transactions and inherents");
    }

    fn commit_revert(
        &self,
        transactions: &[Transaction],
        inherents: &[Inherent],
        block_state: &BlockState,
        block_logger: &mut BlockLogger,
        keep_commit: bool,
    ) -> Result<Receipts, AccountError> {
        let mut txn = WriteTransaction::new(&self.accounts.env);

        let initial_root_hash = self.accounts.get_root_hash_assert(Some(&txn));

        let receipts =
            self.accounts
                .commit(&mut txn, transactions, inherents, block_state, block_logger)?;

        // Potentially keep the commit.
        if keep_commit {
            txn.commit();
            txn = WriteTransaction::new(&self.accounts.env);
        }

        // We only check whether the revert is consistent.
        // We do not do any checks on the intermediate state.

        self.accounts
            .revert(
                &mut txn,
                transactions,
                inherents,
                block_state,
                receipts.clone(),
                &mut BlockLogger::empty(),
            )
            .expect("Failed to revert transactions and inherents");

        let current_root_hash = self.accounts.get_root_hash_assert(Some(&txn));

        assert_eq!(initial_root_hash, current_root_hash);

        Ok(receipts)
    }
}

impl Deref for TestCommitRevert {
    type Target = Accounts;

    fn deref(&self) -> &Self::Target {
        &self.accounts
    }
}

#[test]
fn can_revert_transactions() {
    let accounts = TestCommitRevert::new();
    let mut generator = TransactionsGenerator::new(
        Accounts::new(accounts.env.clone()),
        NetworkId::UnitAlbatross,
        rand::thread_rng(),
    );

    let block_state = BlockState::new(
        Policy::blocks_per_epoch() + Policy::blocks_per_batch() + 1,
        10,
    );

    for sender in [
        OutgoingType::Basic,
        OutgoingType::Vesting,
        OutgoingType::HTLCRegularTransfer,
        OutgoingType::HTLCEarlyResolve,
        OutgoingType::HTLCTimeoutResolve,
        OutgoingType::DeleteValidator,
        OutgoingType::RemoveStake,
    ] {
        for recipient in [
            IncomingType::Basic,
            IncomingType::CreateVesting,
            IncomingType::CreateHTLC,
            IncomingType::CreateValidator,
            IncomingType::UpdateValidator,
            IncomingType::UnparkValidator,
            IncomingType::DeactivateValidator,
            IncomingType::ReactivateValidator,
            IncomingType::RetireValidator,
            IncomingType::CreateStaker,
            IncomingType::AddStake,
            IncomingType::UpdateStaker,
        ] {
            // Don't send from the staking contract to the staking contract.
            if matches!(
                sender,
                OutgoingType::DeleteValidator | OutgoingType::RemoveStake
            ) && matches!(
                recipient,
                IncomingType::CreateValidator
                    | IncomingType::UpdateValidator
                    | IncomingType::UnparkValidator
                    | IncomingType::DeactivateValidator
                    | IncomingType::ReactivateValidator
                    | IncomingType::RetireValidator
                    | IncomingType::CreateStaker
                    | IncomingType::AddStake
                    | IncomingType::UpdateStaker
            ) {
                continue;
            }

            info!(?sender, ?recipient, "Testing transaction");
            let tx = generator.create_transaction(
                sender,
                recipient,
                Coin::from_u64_unchecked(10),
                Coin::from_u64_unchecked(1),
                &block_state,
            );

            assert_eq!(tx.verify(NetworkId::UnitAlbatross), Ok(()));

            accounts.test(&[tx], &[], &block_state);
        }
    }
}

#[test]
fn can_revert_inherents() {
    let accounts = TestCommitRevert::new();
    let mut generator = TransactionsGenerator::new(
        Accounts::new(accounts.env.clone()),
        NetworkId::UnitAlbatross,
        rand::thread_rng(),
    );

    let block_state = BlockState::new(
        Policy::blocks_per_epoch() + Policy::blocks_per_batch() + 1,
        10,
    );

    let mut rng = rand::thread_rng();

    info!("Testing inherent Reward");
    let inherent = Inherent::Reward {
        target: Address(rng.gen()),
        value: Coin::from_u64_unchecked(10),
    };

    accounts.test(&[], &[inherent], &block_state);

    let (validator_key_pair, _, _) =
        generator.create_validator_and_staker(Coin::from_u64_unchecked(10), ValidatorState::Active);

    info!("Testing inherent Slash");
    let inherent = Inherent::Slash {
        slot: SlashedSlot {
            slot: rng.gen_range(0..Policy::SLOTS),
            validator_address: Address::from(&validator_key_pair),
            event_block: block_state.number - 1,
        },
    };

    accounts.test(&[], &[inherent], &block_state);
}
