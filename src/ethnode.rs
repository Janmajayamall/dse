use anyhow::Ok;
use ethers::prelude::*;
use std::{sync::Arc, str::FromStr};

use super::commitment;

abigen!(
    WalletContract,
    "./src/wallet.json",
    event_derives(serde::Deserialize, serde::Serialize),
);

#[derive(Clone)]
pub struct EthNode {
    client: Arc<Provider<Http>>,
    wallet: LocalWallet,
    pub timelocked_wallet: types::Address,
}

impl EthNode {
    pub fn new(rpc_endpoint: String, private_key: String, timelocked_wallet: types::Address) -> Result<Self, anyhow::Error> {
        let client = Provider::<Http>::try_from(rpc_endpoint)?;
        let wallet = LocalWallet::from_str(&private_key)?;
        
        Ok(
            Self {
                client: Arc::new(client),
                wallet,
                timelocked_wallet,
            }
        )
    }   

    pub async fn validate_commitment_index(self, index: u64, epoch: u64, wallet_address: Address) {
        let wallet = WalletContract::new(wallet_address, self.client.clone());
        // let value = wallet.is_valid_commitment(index, epoch).call().await?;
        // println!("EthNode: returned {:?}", value)
    }

    pub fn sign_commit_message(&self, commit: &commitment::Commit) -> types::Signature {
        self.wallet.sign_hash(commit.commit_hash(), true)
    }

    pub async fn get_current_epoch(&self) {

    }

    pub fn signer_address(&self) -> types::Address {
        self.wallet.address()
    }
}