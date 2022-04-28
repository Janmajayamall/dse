use anyhow::Ok;
use ethers::prelude::*;
use std::{str::FromStr, sync::Arc};

abigen!(
    WalletContract,
    "./src/wallet.json",
    event_derives(serde::Deserialize, serde::Serialize),
);

#[derive(Clone)]
pub struct EthNode {
    pub client: Arc<Provider<Http>>,
    pub wallet: LocalWallet,
    pub timelocked_wallet: types::Address,
}

impl EthNode {
    pub fn new(
        rpc_endpoint: String,
        private_key: String,
        timelocked_wallet: types::Address,
    ) -> Result<Self, anyhow::Error> {
        let client = Provider::<Http>::try_from(rpc_endpoint)?;
        let wallet = LocalWallet::from_str(&private_key)?;

        Ok(Self {
            client: Arc::new(client),
            wallet,
            timelocked_wallet,
        })
    }

    pub async fn is_valid_commit_range(
        self,
        index: u64,
        epoch: u64,
        wallet_address: Address,
    ) -> bool {
        let wallet = WalletContract::new(wallet_address, self.client.clone());

        // TODO Call `is_valid_commit_range` and return the value
        true
    }

    pub fn sign_message(&self, message: types::H256) -> types::Signature {
        self.wallet.sign_hash(message, true)
    }

    pub async fn get_current_epoch(&self, wallet_address: &Address) -> U256 {
        let wallet = WalletContract::new(wallet_address.clone(), self.client.clone());

        // TODO get epoch from API

        U256::default()
    }

    pub fn self_address(&self) -> Address {
        self.wallet.address()
    }

    pub async fn owner_address(&self, wallet_address: &Address) -> Address {
        Default::default()
    }
}
