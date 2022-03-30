use anyhow::Ok;
// use ethers::{*};u
use ethers::prelude::*;
use std::{sync::Arc, str::FromStr};

abigen!(
    WalletContract,
    "./src/wallet.json",
    event_derives(serde::Deserialize, serde::Serialize),
);


pub struct EthNode {
    client: Arc<Provider<Http>>,
    wallet: LocalWallet,
    // private key
    // public key

    // JSON RPC

    // other functions
}

impl EthNode {
    pub fn new(rpc_endpoint: String, private_key: String) -> Result<Self, anyhow::Error> {
        let client = Provider::<Http>::try_from(rpc_endpoint)?;
        let wallet = LocalWallet::from_str(&private_key)?;
        
        Ok(
            Self {
                client: Arc::new(client),
                wallet
            }
        )
    }   

    pub async fn validate_commitment_index(self, index: u64, epoch: u64, wallet_address: Address) {
        let wallet = WalletContract::new(wallet_address, self.client.clone());
        // let value = wallet.is_valid_commitment(index, epoch).call().await?;
        // println!("EthNode: returned {:?}", value)
    }

    pub async fn sing_message(self) {
        
    }
}