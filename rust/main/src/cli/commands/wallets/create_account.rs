use crate::cli::get_path;
use anyhow::{anyhow, Result};
use clap::{ArgGroup, Parser};
use rsnano_core::{Account, WalletId};
use rsnano_node::wallets::{Wallets, WalletsExt};
use rsnano_store_lmdb::LmdbEnv;
use std::sync::Arc;

#[derive(Parser)]
#[command(group = ArgGroup::new("input")
    .args(&["data_path", "network"]))]
pub(crate) struct CreateAccountArgs {
    #[arg(long)]
    wallet: String,
    #[arg(long)]
    password: Option<String>,
    #[arg(long, group = "input")]
    data_path: Option<String>,
    #[arg(long, group = "input")]
    network: Option<String>,
}

impl CreateAccountArgs {
    pub(crate) fn create_account(&self) -> Result<()> {
        let path = get_path(&self.data_path, &self.network).join("wallets.ldb");

        let env = Arc::new(LmdbEnv::new(&path)?);

        let wallets = Arc::new(Wallets::new_with_env(env)?);

        let wallet = WalletId::decode_hex(&self.wallet)?;

        let mut password = String::new();
        if let Some(pass) = &self.password {
            password = pass.clone();
        };

        wallets.ensure_wallet_is_unlocked(wallet, &password);

        let public_key = wallets
            .deterministic_insert2(&wallet, false)
            .map_err(|e| anyhow!("Failed to insert wallet: {:?}", e))?;

        println!("Account: {:?}", Account::encode_account(&public_key));

        Ok(())
    }
}
