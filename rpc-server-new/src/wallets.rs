use std::collections::HashMap;

use nimiq_wallet::WalletAccount;
use nimiq_utils::otp::Unlocked;
use nimiq_keys::Address;


#[derive(Default)]
pub struct UnlockedWallets {
    pub unlocked_wallets: HashMap<Address, Unlocked<WalletAccount>>,
}

impl UnlockedWallets {
    pub fn insert(&mut self, wallet: Unlocked<WalletAccount>) {
        log::info!("Unlocking {:?}", &wallet.address);
        self.unlocked_wallets.insert(wallet.address.clone(), wallet);
    }

    pub fn get(&self, address: &Address) -> Option<&WalletAccount> {
        log::info!("Accessing {:?}", address);
        self.unlocked_wallets
            .get(address)
            .map(|unlocked| Unlocked::unlocked_data(unlocked))
    }

    pub fn remove(&mut self, address: &Address) -> Option<Unlocked<WalletAccount>> {
        self.unlocked_wallets.remove(address)
    }
}
