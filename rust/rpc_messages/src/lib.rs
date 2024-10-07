mod common;
mod ledger;
mod node;
mod utils;
mod wallets;

pub use common::*;
pub use ledger::*;
pub use node::*;
use serde::{Deserialize, Serialize};
pub use utils::*;
pub use wallets::*;

#[derive(PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RpcCommand {
    AccountInfo(AccountInfoArgs),
    Keepalive(KeepaliveArgs),
    Stop,
    KeyCreate,
    Receive(ReceiveArgs),
    Send(SendArgs),
    WalletAdd(WalletAddArgs),
    AccountCreate(AccountCreateArgs),
    AccountBalance(AccountBalanceArgs),
    AccountsCreate(AccountsCreateArgs),
    AccountRemove(WalletWithAccountArgs),
    AccountMove(AccountMoveArgs),
    AccountList(WalletRpcMessage),
    WalletCreate(WalletCreateArgs),
    WalletContains(WalletWithAccountArgs),
    WalletDestroy(WalletRpcMessage),
    WalletLock(WalletRpcMessage),
    WalletLocked(WalletRpcMessage),
    AccountBlockCount(AccountRpcMessage),
    AccountKey(AccountRpcMessage),
    AccountGet(KeyRpcMessage),
    AccountRepresentative(AccountRpcMessage),
    AccountWeight(AccountRpcMessage),
    AvailableSupply,
    BlockAccount(BlockHashRpcMessage),
    BlockConfirm(BlockHashRpcMessage),
    BlockCount,
    Uptime,
}
