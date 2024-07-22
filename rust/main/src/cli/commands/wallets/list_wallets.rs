use crate::cli::get_path;
use anyhow::{anyhow, Result};
use clap::{ArgGroup, Parser};
use rsnano_core::Account;
use rsnano_node::wallets::{Wallets, WalletsExt};
use rsnano_store_lmdb::LmdbEnv;
use std::sync::Arc;

#[derive(Parser)]
#[command(group = ArgGroup::new("input")
    .args(&["data_path", "network"]))]
pub(crate) struct ListWalletsArgs {
    #[arg(long)]
    password: Option<String>,
    #[arg(long, group = "input")]
    data_path: Option<String>,
    #[arg(long, group = "input")]
    network: Option<String>,
}

impl ListWalletsArgs {
    pub(crate) fn list_wallets(&self) -> Result<()> {
        let path = get_path(&self.data_path, &self.network).join("wallets.ldb");

        let env = Arc::new(LmdbEnv::new(&path)?);

        let wallets = Arc::new(Wallets::new_with_env(env.clone())?);

        let mut txn = env.tx_begin_read();

        let password = self.password.clone().unwrap_or_default();

        let wallet_ids = wallets.get_wallet_ids(&mut txn);

        for wallet_id in wallet_ids {
            wallets.ensure_wallet_is_unlocked(wallet_id, &password);
            println!("{:?}", wallet_id);
            let accounts = wallets
                .get_accounts_of_wallet(&wallet_id)
                .map_err(|e| anyhow!("Failed to get accounts of wallets: {:?}", e))?;
            if !accounts.is_empty() {
                for account in accounts {
                    println!("{:?}", Account::encode_account(&account));
                }
            }
        }

        Ok(())
    }
}
