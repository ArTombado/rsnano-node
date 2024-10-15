mod ledger;
mod node;
mod utils;
mod wallets;

pub use ledger::*;
pub use node::*;
pub use utils::*;
pub use wallets::*;

use serde::{Deserialize, Serialize};

#[derive(PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RpcCommand {
    AccountInfo(AccountInfoArgs),
    Keepalive(AddressWithPortArg),
    Stop,
    KeyCreate,
    Receive(ReceiveArgs),
    Send(SendArgs),
    WalletAdd(WalletAddArgs),
    AccountCreate(AccountCreateArgs),
    AccountBalance(AccountBalanceArgs),
    AccountsCreate(AccountsCreateArgs),
    AccountRemove(AccountRemoveArgs),
    AccountMove(AccountMoveArgs),
    AccountList(AccountListArgs),
    WalletCreate(WalletCreateArgs),
    WalletContains(WalletContainsArgs),
    WalletDestroy(WalletDestroyArgs),
    WalletLock(WalletLockArgs),
    WalletLocked(WalletLockedArgs),
    AccountBlockCount(AccountBlockCountArgs),
    AccountKey(AccountKeyArgs),
    AccountGet(AccountGetArgs),
    AccountRepresentative(AccountRepresentativeArgs),
    AccountWeight(AccountWeightArgs),
    AvailableSupply,
    BlockAccount(BlockAccountArgs),
    BlockConfirm(BlockConfirmArgs),
    BlockCount,
    Uptime,
    FrontierCount,
    ValidateAccountNumber(ValidateAccountNumberArgs),
    NanoToRaw(NanoToRawArgs),
    RawToNano(RawToNanoArgs),
    WalletAddWatch(WalletAddWatchArgs),
    WalletRepresentative(WalletRpcMessage),
    WorkSet(WorkSetArgs),
    WorkGet(WalletWithAccountArgs),
    WalletWorkGet(WalletRpcMessage),
    AccountsFrontiers(AccountsFrontiersArgs),
    WalletFrontiers(WalletRpcMessage),
    Frontiers(FrontiersArgs),
    WalletInfo(WalletRpcMessage),
    WalletExport(WalletRpcMessage),
    PasswordChange(WalletWithPasswordArgs),
    PasswordEnter(WalletWithPasswordArgs),
    PasswordValid(WalletRpcMessage),
    DeterministicKey(DeterministicKeyArgs),
    KeyExpand(KeyExpandArgs),
    Peers(PeersArgs),
    PopulateBacklog,
    Representatives(RepresentativesArgs),
    AccountsRepresentatives(AccountsRepresentativesArgs),
    StatsClear,
    UncheckedClear,
    Unopened(UnopenedArgs),
    NodeId,
    SearchReceivableAll,
    ReceiveMinimum,
    WalletChangeSeed(WalletChangeSeedArgs),
    Delegators(DelegatorsArgs),
    DelegatorsCount(DelegatorsCountArgs),
    BlockHash(BlockHashArgs),
    AccountsBalances(AccountsBalancesArgs),
    BlockInfo(BlockInfoArgs),
    Blocks(BlocksArgs),
    BlocksInfo(BlocksInfoArgs),
    Chain(ChainArgs),
    Successors(ChainArgs),
    ConfirmationActive(ConfirmationActiveArgs),
    ConfirmationQuorum(ConfirmationQuorumArgs),
    WorkValidate(WorkValidateArgs),
    AccountHistory(AccountHistoryArgs),
    Sign(SignArgs),
    Process(ProcessArgs),
    WorkCancel(WorkCancelArgs),
    Bootstrap(BootstrapArgs),
    BootstrapAny(BootstrapAnyArgs),
    BoostrapLazy(BootsrapLazyArgs),
    WalletReceivable(WalletReceivableArgs),
    WalletRepresentativeSet(WalletRepresentativeSetArgs),
    SearchReceivable(WalletRpcMessage),
    WalletRepublish(WalletWithCountArgs),
    WalletBalances(WalletBalancesArgs),
    WalletHistory(WalletHistoryArgs),
    WalletLedger(WalletLedgerArgs),
    AccountsReceivable(AccountsReceivableArgs),
    Receivable(ReceivableArgs),
    ReceivableExists(ReceivableExistsArgs),
    RepresentativesOnline(RepresentativesOnlineArgs),
    Unchecked(UncheckedArgs),
    UncheckedGet(UncheckedGetArgs),
    UncheckedKeys(UncheckedKeysArgs),
    ConfirmationInfo(ConfirmationInfoArgs),
    Ledger(LedgerArgs),
    WorkGenerate(WorkGenerateArgs),
    Republish(RepublishArgs),
    BlockCreate(BlockCreateArgs),
}
