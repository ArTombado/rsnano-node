use crate::AccountBalanceDto;
use anyhow::{bail, Result};
use reqwest::Client;
pub use reqwest::Url;
use rsnano_core::{Account, Amount, JsonBlock, PublicKey, RawKey, WalletId};
use rsnano_rpc_messages::*;
use serde::Serialize;
use serde_json::{from_str, from_value, Value};
use std::{net::Ipv6Addr, time::Duration};

pub struct NanoRpcClient {
    url: Url,
    client: Client,
}

impl NanoRpcClient {
    pub fn new(url: Url) -> Self {
        Self {
            url,
            client: reqwest::ClientBuilder::new()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
        }
    }

    pub async fn account_get(&self, key: PublicKey) -> Result<AccountRpcMessage> {
        let cmd = RpcCommand::account_get(key);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_balance(
        &self,
        account: Account,
        include_only_confirmed: Option<bool>,
    ) -> Result<AccountBalanceDto> {
        let cmd = RpcCommand::account_balance(account, include_only_confirmed);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_create(
        &self,
        wallet: WalletId,
        index: Option<u32>,
        work: Option<bool>,
    ) -> Result<AccountRpcMessage> {
        let cmd = RpcCommand::account_create(wallet, index, work);
        let result = self.rpc_request(&cmd).await?;
        Ok(from_value(result)?)
    }

    pub async fn accounts_create(
        &self,
        wallet: WalletId,
        count: u32,
        work: Option<bool>,
    ) -> Result<AccountsRpcMessage> {
        let cmd = RpcCommand::accounts_create(wallet, count, work);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_remove(&self, wallet: WalletId, account: Account) -> Result<BoolDto> {
        let cmd = RpcCommand::account_remove(wallet, account);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_move(
        &self,
        wallet: WalletId,
        source: WalletId,
        account: Vec<Account>,
    ) -> Result<BoolDto> {
        let cmd = RpcCommand::account_move(wallet, source, account);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_list(&self, wallet: WalletId) -> Result<AccountsRpcMessage> {
        let cmd = RpcCommand::account_list(wallet);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn wallet_create(&self, seed: Option<RawKey>) -> Result<WalletCreateDto> {
        let cmd = RpcCommand::wallet_create(seed);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn wallet_contains(&self, wallet: WalletId, account: Account) -> Result<BoolDto> {
        let cmd = RpcCommand::wallet_contains(wallet, account);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn wallet_destroy(&self, wallet: WalletId) -> Result<BoolDto> {
        let cmd = RpcCommand::wallet_destroy(wallet);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn wallet_lock(&self, wallet: WalletId) -> Result<BoolDto> {
        let cmd = RpcCommand::wallet_lock(wallet);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn wallet_locked(&self, wallet: WalletId) -> Result<BoolDto> {
        let cmd = RpcCommand::wallet_locked(wallet);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn stop(&self) -> Result<SuccessDto> {
        let cmd = RpcCommand::stop();
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_block_count(&self, account: Account) -> Result<U64RpcMessage> {
        let cmd = RpcCommand::account_block_count(account);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_key(&self, account: Account) -> Result<KeyRpcMessage> {
        let cmd = RpcCommand::account_key(account);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_representative(&self, account: Account) -> Result<AccountRpcMessage> {
        let cmd = RpcCommand::account_representative(account);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_weight(&self, account: Account) -> Result<AmountDto> {
        let cmd = RpcCommand::account_weight(account);
        let result = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn account_info(&self, account: Account) -> Result<AccountInfoDto> {
        let cmd = RpcCommand::account_info(account);
        let result = self.rpc_request(&cmd).await?;
        Ok(from_value(result)?)
    }

    pub async fn receive_block(
        &self,
        wallet: WalletId,
        destination: Account,
        block: impl Into<JsonBlock>,
    ) -> Result<()> {
        let request = RpcCommand::Receive(ReceiveArgs {
            wallet,
            account: destination,
            block: block.into(),
        });
        self.rpc_request(&request).await?;
        Ok(())
    }

    pub async fn send_block(
        &self,
        wallet: WalletId,
        source: Account,
        destination: Account,
    ) -> Result<JsonBlock> {
        let request = RpcCommand::Send(SendArgs {
            wallet,
            source,
            destination,
            amount: Amount::raw(1),
        });
        let json = self.rpc_request(&request).await?;
        let block = json["block"].as_str().unwrap().to_owned();
        Ok(from_str(&block)?)
    }

    pub async fn send_receive(
        &self,
        wallet: WalletId,
        source: Account,
        destination: Account,
    ) -> Result<()> {
        let block = self.send_block(wallet, source, destination).await?;
        self.receive_block(wallet, destination, block).await
    }

    pub async fn keepalive(&self, port: u16) -> Result<()> {
        let request = RpcCommand::keepalive(Ipv6Addr::LOCALHOST, port);
        self.rpc_request(&request).await?;
        Ok(())
    }

    pub async fn key_create(&self) -> Result<KeyPairDto> {
        let cmd = RpcCommand::KeyCreate;
        let json = self.rpc_request(&cmd).await?;
        Ok(from_value(json)?)
    }

    pub async fn wallet_add(
        &self,
        wallet: WalletId,
        prv_key: RawKey,
        work: Option<bool>,
    ) -> Result<AccountRpcMessage> {
        let cmd = RpcCommand::wallet_add(wallet, prv_key, work);
        let json = self.rpc_request(&cmd).await?;
        Ok(serde_json::from_value(json)?)
    }

    pub async fn stop_rpc(&self) -> Result<()> {
        self.rpc_request(&RpcCommand::Stop).await?;
        Ok(())
    }

    async fn rpc_request<T>(&self, request: &T) -> Result<serde_json::Value>
    where
        T: Serialize,
    {
        let result = self
            .client
            .post(self.url.clone())
            .json(request)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;

        if let Some(error) = result.get("error") {
            bail!("node returned error: {}", error);
        }

        Ok(result)
    }
}
