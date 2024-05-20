use super::{Wallet, WalletActionThread, WalletRepresentatives};
use crate::{
    block_processing::{BlockProcessor, BlockSource},
    cementation::ConfirmingSet,
    config::NodeConfig,
    transport::{BufferDropPolicy, TcpChannels},
    utils::ThreadPool,
    work::DistributedWorkFactory,
    NetworkParams, OnlineReps,
};
use lmdb::{DatabaseFlags, WriteFlags};
use rand::{thread_rng, Rng, RngCore};
use rsnano_core::{
    utils::get_env_or_default_string, work::WorkThresholds, Account, Amount, BlockDetails,
    BlockEnum, BlockHash, Epoch, HackyUnsafeMutBlock, KeyDerivationFunction, Link, NoValue,
    PendingKey, PublicKey, RawKey, Root, StateBlock, WalletId, WorkVersion,
};
use rsnano_ledger::{BlockStatus, Ledger};
use rsnano_messages::{Message, Publish};
use rsnano_store_lmdb::{
    create_backup_file, BinaryDbIterator, DbIterator, Environment, EnvironmentWrapper, LmdbEnv,
    LmdbIteratorImpl, LmdbWalletStore, LmdbWriteTransaction, RwTransaction, Transaction,
};
use std::{
    collections::{HashMap, HashSet},
    fs::Permissions,
    ops::Deref,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use tracing::{info, warn};

#[derive(FromPrimitive)]
pub enum WalletsError {
    None,
    Generic,
    WalletNotFound,
    WalletLocked,
    AccountNotFound,
    InvalidPassword,
    BadPublicKey,
}

pub type WalletsIterator<T> = BinaryDbIterator<[u8; 64], NoValue, LmdbIteratorImpl<T>>;

pub struct Wallets<T: Environment + 'static = EnvironmentWrapper> {
    pub handle: Option<T::Database>,
    pub send_action_ids_handle: Option<T::Database>,
    enable_voting: bool,
    env: Arc<LmdbEnv<T>>,
    pub mutex: Mutex<HashMap<WalletId, Arc<Wallet<T>>>>,
    pub node_config: NodeConfig,
    ledger: Arc<Ledger>,
    last_log: Mutex<Option<Instant>>,
    distributed_work: Arc<DistributedWorkFactory>,
    work_thresholds: WorkThresholds,
    network_params: NetworkParams,
    pub delayed_work: Mutex<HashMap<Account, Root>>,
    workers: Arc<dyn ThreadPool>,
    pub wallet_actions: WalletActionThread<T>,
    block_processor: Arc<BlockProcessor>,
    pub representatives: Mutex<WalletRepresentatives>,
    online_reps: Arc<Mutex<OnlineReps>>,
    kdf: KeyDerivationFunction,
    tcp_channels: Arc<TcpChannels>,
    start_election: Mutex<Option<Box<dyn Fn(Arc<BlockEnum>) + Send + Sync>>>,
    confirming_set: Arc<ConfirmingSet>,
}

impl<T: Environment + 'static> Wallets<T> {
    pub fn new(
        enable_voting: bool,
        env: Arc<LmdbEnv<T>>,
        ledger: Arc<Ledger>,
        node_config: &NodeConfig,
        kdf_work: u32,
        work: WorkThresholds,
        distributed_work: Arc<DistributedWorkFactory>,
        network_params: NetworkParams,
        workers: Arc<dyn ThreadPool>,
        block_processor: Arc<BlockProcessor>,
        online_reps: Arc<Mutex<OnlineReps>>,
        tcp_channels: Arc<TcpChannels>,
        confirming_set: Arc<ConfirmingSet>,
    ) -> anyhow::Result<Self> {
        let kdf = KeyDerivationFunction::new(kdf_work);
        let mut wallets = Self {
            handle: None,
            send_action_ids_handle: None,
            enable_voting,
            mutex: Mutex::new(HashMap::new()),
            env,
            node_config: node_config.clone(),
            ledger: Arc::clone(&ledger),
            last_log: Mutex::new(None),
            distributed_work,
            work_thresholds: work.clone(),
            network_params,
            delayed_work: Mutex::new(HashMap::new()),
            workers,
            wallet_actions: WalletActionThread::new(),
            block_processor,
            representatives: Mutex::new(WalletRepresentatives::new(
                node_config.vote_minimum,
                Arc::clone(&ledger),
            )),
            online_reps,
            kdf: kdf.clone(),
            tcp_channels,
            start_election: Mutex::new(None),
            confirming_set,
        };
        let mut txn = wallets.env.tx_begin_write();
        wallets.initialize(&mut txn)?;
        {
            let mut guard = wallets.mutex.lock().unwrap();
            let wallet_ids = wallets.get_wallet_ids(&txn);
            for id in wallet_ids {
                assert!(!guard.contains_key(&id));
                let representative = node_config.random_representative();
                let text = PathBuf::from(id.encode_hex());
                let wallet = Wallet::new(
                    Arc::clone(&ledger),
                    work.clone(),
                    &mut txn,
                    node_config.password_fanout as usize,
                    kdf.clone(),
                    representative,
                    &text,
                )?;

                guard.insert(id, Arc::new(wallet));
            }

            // Backup before upgrade wallets
            let mut backup_required = false;
            if node_config.backup_before_upgrade {
                let txn = wallets.env.tx_begin_read();
                for wallet in guard.values() {
                    if wallet.store.version(&txn) != LmdbWalletStore::<T>::VERSION_CURRENT {
                        backup_required = true;
                        break;
                    }
                }
            }
            if backup_required {
                create_backup_file(&wallets.env)?;
            }
            // TODO port more here...
        }

        Ok(wallets)
    }

    pub fn start(&self) {
        self.wallet_actions.start();
    }

    pub fn stop(&self) {
        self.wallet_actions.stop();
    }

    pub fn set_start_election_callback(&self, callback: Box<dyn Fn(Arc<BlockEnum>) + Send + Sync>) {
        *self.start_election.lock().unwrap() = Some(callback);
    }

    pub fn initialize(&mut self, txn: &mut LmdbWriteTransaction<T>) -> anyhow::Result<()> {
        self.handle = Some(unsafe { txn.rw_txn_mut().create_db(None, DatabaseFlags::empty())? });
        self.send_action_ids_handle = Some(unsafe {
            txn.rw_txn_mut()
                .create_db(Some("send_action_ids"), DatabaseFlags::empty())?
        });
        Ok(())
    }

    pub fn voting_reps_count(&self) -> u64 {
        self.representatives.lock().unwrap().voting_reps()
    }

    pub fn get_store_it(
        &self,
        txn: &dyn Transaction<Database = T::Database, RoCursor = T::RoCursor>,
        hash: &str,
    ) -> WalletsIterator<T> {
        let hash_bytes: [u8; 64] = hash.as_bytes().try_into().unwrap();
        WalletsIterator::new(LmdbIteratorImpl::<T>::new(
            txn,
            self.handle.unwrap(),
            Some(&hash_bytes),
            true,
        ))
    }

    pub fn get_wallet_ids(
        &self,
        txn: &dyn Transaction<Database = T::Database, RoCursor = T::RoCursor>,
    ) -> Vec<WalletId> {
        let mut wallet_ids = Vec::new();
        let beginning = RawKey::from(0).encode_hex();
        let mut i = self.get_store_it(txn, &beginning);
        while let Some((k, _)) = i.current() {
            let text = std::str::from_utf8(k).unwrap();
            wallet_ids.push(WalletId::decode_hex(text).unwrap());
            i.next();
        }
        wallet_ids
    }

    pub fn get_block_hash(
        &self,
        txn: &dyn Transaction<Database = T::Database, RoCursor = T::RoCursor>,
        id: &str,
    ) -> anyhow::Result<Option<BlockHash>> {
        match txn.get(self.send_action_ids_handle.unwrap(), id.as_bytes()) {
            Ok(bytes) => Ok(Some(
                BlockHash::from_slice(bytes).ok_or_else(|| anyhow!("invalid block hash"))?,
            )),
            Err(lmdb::Error::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_block_hash(
        &self,
        txn: &mut LmdbWriteTransaction<T>,
        id: &str,
        hash: &BlockHash,
    ) -> anyhow::Result<()> {
        txn.rw_txn_mut().put(
            self.send_action_ids_handle.unwrap(),
            id.as_bytes(),
            hash.as_bytes(),
            WriteFlags::empty(),
        )?;
        Ok(())
    }

    pub fn clear_send_ids(&self, txn: &mut LmdbWriteTransaction<T>) {
        txn.clear_db(self.send_action_ids_handle.unwrap()).unwrap();
    }

    pub fn foreach_representative<F>(&self, mut action: F)
    where
        F: FnMut(&Account, &RawKey),
    {
        if self.node_config.enable_voting {
            let mut action_accounts_l: Vec<(PublicKey, RawKey)> = Vec::new();
            {
                let transaction_l = self.env.tx_begin_read();
                let ledger_txn = self.ledger.read_txn();
                let lock = self.mutex.lock().unwrap();
                for (wallet_id, wallet) in lock.iter() {
                    let representatives_l = wallet.representatives.lock().unwrap().clone();
                    for account in representatives_l {
                        if wallet.store.exists(&transaction_l, &account) {
                            if !self.ledger.weight_exact(&ledger_txn, account).is_zero() {
                                if wallet.store.valid_password(&transaction_l) {
                                    let prv = wallet
                                        .store
                                        .fetch(&transaction_l, &account)
                                        .expect("could not fetch account from wallet");

                                    action_accounts_l.push((account, prv));
                                } else {
                                    let mut last_log_guard = self.last_log.lock().unwrap();
                                    let should_log = match last_log_guard.as_ref() {
                                        Some(i) => i.elapsed() >= Duration::from_secs(60),
                                        None => true,
                                    };
                                    if should_log {
                                        *last_log_guard = Some(Instant::now());
                                        warn!("Representative locked inside wallet {}", wallet_id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for (pub_key, prv_key) in action_accounts_l {
                action(&pub_key, &prv_key);
            }
        }
    }

    pub fn work_cache_blocking(&self, wallet: &Wallet<T>, account: &Account, root: &Root) {
        if self.distributed_work.work_generation_enabled() {
            let difficulty = self.work_thresholds.threshold_base(WorkVersion::Work1);
            if let Some(work) = self.distributed_work.make_blocking(
                WorkVersion::Work1,
                *root,
                difficulty,
                Some(*account),
            ) {
                let mut tx = self.env.tx_begin_write();
                if wallet.live() && wallet.store.exists(&tx, account) {
                    wallet.work_update(&mut tx, account, root, work);
                }
            } else {
                warn!(
                    "Could not precache work for root {} due to work generation failure",
                    root
                );
            }
        }
    }

    fn get_wallet<'a>(
        guard: &'a HashMap<WalletId, Arc<Wallet<T>>>,
        wallet_id: &WalletId,
    ) -> Result<&'a Arc<Wallet<T>>, WalletsError> {
        guard.get(wallet_id).ok_or(WalletsError::WalletNotFound)
    }

    pub fn insert_watch(
        &self,
        wallet_id: &WalletId,
        accounts: &[Account],
    ) -> Result<(), WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        let mut tx = self.env.tx_begin_write();
        if !wallet.store.valid_password(&tx) {
            return Err(WalletsError::WalletLocked);
        }

        for account in accounts {
            if wallet.store.insert_watch(&mut tx, account).is_err() {
                return Err(WalletsError::BadPublicKey);
            }
        }

        Ok(())
    }

    pub fn valid_password(&self, wallet_id: &WalletId) -> Result<bool, WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        let tx = self.env.tx_begin_read();
        Ok(wallet.store.valid_password(&tx))
    }

    pub fn attempt_password(
        &self,
        wallet_id: &WalletId,
        password: impl AsRef<str>,
    ) -> Result<(), WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        let tx = self.env.tx_begin_write();
        if wallet.store.attempt_password(&tx, password.as_ref()) {
            Ok(())
        } else {
            Err(WalletsError::InvalidPassword)
        }
    }

    pub fn lock(&self, wallet_id: &WalletId) -> Result<(), WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        wallet.store.lock();
        Ok(())
    }

    pub fn rekey(
        &self,
        wallet_id: &WalletId,
        password: impl AsRef<str>,
    ) -> Result<(), WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        let mut tx = self.env.tx_begin_write();
        if !wallet.store.valid_password(&tx) {
            return Err(WalletsError::WalletLocked);
        }

        wallet
            .store
            .rekey(&mut tx, password.as_ref())
            .map_err(|_| WalletsError::Generic)
    }

    pub fn set_observer(&self, observer: Box<dyn Fn(bool) + Send>) {
        self.wallet_actions.set_observer(observer);
    }

    pub fn compute_reps(&self) {
        let wallets_guard = self.mutex.lock().unwrap();
        let mut reps_guard = self.representatives.lock().unwrap();
        reps_guard.clear();
        let half_principal_weight = self.online_reps.lock().unwrap().minimum_principal_weight() / 2;
        let tx = self.env.tx_begin_read();
        for (_, wallet) in wallets_guard.iter() {
            let mut representatives = HashSet::new();
            let mut it = wallet.store.begin(&tx);
            while let Some((&account, _)) = it.current() {
                if reps_guard.check_rep(account, half_principal_weight) {
                    representatives.insert(account);
                }
                it.next();
            }
            *wallet.representatives.lock().unwrap() = representatives;
        }
    }

    pub fn exists(&self, account: &Account) -> bool {
        let guard = self.mutex.lock().unwrap();
        let tx = self.env.tx_begin_read();
        guard
            .values()
            .any(|wallet| wallet.store.exists(&tx, account))
    }

    pub fn reload(&self) {
        let mut guard = self.mutex.lock().unwrap();
        let mut tx = self.env.tx_begin_write();
        let mut stored_items = HashSet::new();
        let wallet_ids = self.get_wallet_ids(&tx);
        for id in wallet_ids {
            // New wallet
            if !guard.contains_key(&id) {
                let text = PathBuf::from(id.encode_hex());
                let representative = self.node_config.random_representative();
                if let Ok(wallet) = Wallet::new(
                    Arc::clone(&self.ledger),
                    self.work_thresholds.clone(),
                    &mut tx,
                    self.node_config.password_fanout as usize,
                    self.kdf.clone(),
                    representative,
                    &text,
                ) {
                    guard.insert(id, Arc::new(wallet));
                }
            }
            // List of wallets on disk
            stored_items.insert(id);
        }
        // Delete non existing wallets from memory
        let mut deleted_items = Vec::new();
        for &id in guard.keys() {
            if !stored_items.contains(&id) {
                deleted_items.push(id);
            }
        }
        for i in &deleted_items {
            guard.remove(i);
        }
    }

    pub fn destroy(&self, id: &WalletId) {
        let mut guard = self.mutex.lock().unwrap();
        let mut tx = self.env.tx_begin_write();
        // action_mutex should be locked after transactions to prevent deadlocks in deterministic_insert () & insert_adhoc ()
        let _action_guard = self.wallet_actions.lock_safe();
        let wallet = guard.remove(id).unwrap();
        wallet.store.destroy(&mut tx);
    }

    pub fn remove_account(
        &self,
        wallet_id: &WalletId,
        account: &Account,
    ) -> Result<(), WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        let mut tx = self.env.tx_begin_write();
        if !wallet.store.valid_password(&tx) {
            return Err(WalletsError::WalletLocked);
        }
        if wallet.store.find(&tx, account).is_end() {
            return Err(WalletsError::AccountNotFound);
        }
        wallet.store.erase(&mut tx, account);
        Ok(())
    }

    pub fn work_set(
        &self,
        wallet_id: &WalletId,
        account: &Account,
        work: u64,
    ) -> Result<(), WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        let mut tx = self.env.tx_begin_write();
        if wallet.store.find(&tx, account).is_end() {
            return Err(WalletsError::AccountNotFound);
        }
        wallet.store.work_put(&mut tx, account, work);
        Ok(())
    }

    pub fn move_accounts(
        &self,
        source_id: &WalletId,
        target_id: &WalletId,
        accounts: &[Account],
    ) -> anyhow::Result<()> {
        let guard = self.mutex.lock().unwrap();
        let source = guard
            .get(source_id)
            .ok_or_else(|| anyhow!("source not found"))?;
        let mut tx = self.env.tx_begin_write();
        let target = guard
            .get(target_id)
            .ok_or_else(|| anyhow!("target not found"))?;
        target.store.move_keys(&mut tx, &source.store, accounts)
    }

    pub fn backup(&self, path: &Path) -> anyhow::Result<()> {
        let guard = self.mutex.lock().unwrap();
        let tx = self.env.tx_begin_read();
        for (id, wallet) in guard.iter() {
            std::fs::create_dir_all(path)?;
            std::fs::set_permissions(path, Permissions::from_mode(0o700))?;
            let mut backup_path = PathBuf::from(path);
            backup_path.push(format!("{}.json", id));
            wallet.store.write_backup(&tx, &backup_path)?;
        }
        Ok(())
    }

    pub fn deterministic_index_get(&self, wallet_id: &WalletId) -> Result<u32, WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        let tx = self.env.tx_begin_read();
        Ok(wallet.store.deterministic_index_get(&tx))
    }

    fn prepare_send(
        &self,
        tx: &dyn Transaction<Database = T::Database, RoCursor = T::RoCursor>,
        wallet: &Arc<Wallet<T>>,
        source: Account,
        account: Account,
        amount: Amount,
        mut work: u64,
    ) -> (Option<BlockEnum>, bool, bool, BlockDetails) {
        let block_tx = self.ledger.read_txn();
        let mut details = BlockDetails::new(Epoch::Epoch0, true, false, false);
        let mut block = None;
        if wallet.store.valid_password(tx) {
            let balance = self.ledger.account_balance(&block_tx, &source, false);
            if !balance.is_zero() && balance >= amount {
                let info = self.ledger.account_info(&block_tx, &source).unwrap();
                let prv_key = wallet.store.fetch(tx, &source).unwrap();
                if work == 0 {
                    work = wallet.store.work_get(tx, &source).unwrap_or_default();
                }
                let state_block = BlockEnum::State(StateBlock::new(
                    source,
                    info.head,
                    info.representative,
                    balance - amount,
                    account.into(),
                    &prv_key,
                    &source,
                    work,
                ));
                block = Some(state_block);
                details = BlockDetails::new(info.epoch, true, false, false);
            }
        }

        let error = false;
        let cached_block = false;
        (block, error, cached_block, details)
    }

    fn prepare_send_with_id(
        &self,
        tx: &mut LmdbWriteTransaction<T>,
        id: &str,
        wallet: &Arc<Wallet<T>>,
        source: Account,
        account: Account,
        amount: Amount,
        mut work: u64,
    ) -> (Option<BlockEnum>, bool, bool, BlockDetails) {
        let block_tx = self.ledger.read_txn();
        let mut details = BlockDetails::new(Epoch::Epoch0, true, false, false);

        let mut block = match self.get_block_hash(tx, id) {
            Ok(Some(hash)) => Some(self.ledger.get_block(&block_tx, &hash).unwrap()),
            Ok(None) => None,
            _ => {
                return (None, true, false, details);
            }
        };

        let cached_block: bool;
        let mut error = false;

        if let Some(block) = &block {
            cached_block = true;
            let msg = Message::Publish(Publish::new(block.clone()));
            self.tcp_channels
                .flood_message2(&msg, BufferDropPolicy::NoLimiterDrop, 1.0);
        } else {
            cached_block = false;
            if wallet.store.valid_password(tx) {
                let balance = self.ledger.account_balance(&block_tx, &source, false);
                if !balance.is_zero() && balance >= amount {
                    let info = self.ledger.account_info(&block_tx, &source).unwrap();
                    let prv_key = wallet.store.fetch(tx, &source).unwrap();
                    if work == 0 {
                        work = wallet.store.work_get(tx, &source).unwrap_or_default();
                    }
                    let state_block = BlockEnum::State(StateBlock::new(
                        source,
                        info.head,
                        info.representative,
                        balance - amount,
                        account.into(),
                        &prv_key,
                        &source,
                        work,
                    ));
                    details = BlockDetails::new(info.epoch, true, false, false);
                    if self.set_block_hash(tx, id, &state_block.hash()).is_err() {
                        error = true;
                    } else {
                        block = Some(state_block);
                    }
                }
            }
        }

        (block, error, cached_block, details)
    }

    pub fn work_get(&self, wallet_id: &WalletId, account: &Account) -> u64 {
        let guard = self.mutex.lock().unwrap();
        let tx = self.env.tx_begin_read();
        let Some(wallet) = guard.get(&wallet_id) else {
            return 1;
        };
        wallet.store.work_get(&tx, account).unwrap_or(1)
    }

    pub fn work_get2(&self, wallet_id: &WalletId, account: &Account) -> Result<u64, WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let tx = self.env.tx_begin_read();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        if wallet.store.find(&tx, account).is_end() {
            return Err(WalletsError::AccountNotFound);
        }
        Ok(wallet.store.work_get(&tx, account).unwrap_or(1))
    }

    pub fn get_accounts(&self, max_results: usize) -> Vec<Account> {
        let mut accounts = Vec::new();
        let guard = self.mutex.lock().unwrap();
        let tx = self.env.tx_begin_read();
        for wallet in guard.values() {
            let mut it = wallet.store.begin(&tx);
            while let Some((&account, _)) = it.current() {
                if accounts.len() >= max_results {
                    break;
                }
                accounts.push(account);
                it.next();
            }
        }
        accounts
    }

    pub fn get_accounts_of_wallet(
        &self,
        wallet_id: &WalletId,
    ) -> Result<Vec<Account>, WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        let tx = self.env.tx_begin_read();
        let mut it = wallet.store.begin(&tx);
        let mut accounts = Vec::new();
        while let Some((&account, _)) = it.current() {
            accounts.push(account);
            it.next();
        }
        Ok(accounts)
    }

    pub fn fetch(&self, wallet_id: &WalletId, account: &Account) -> Result<RawKey, WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Self::get_wallet(&guard, wallet_id)?;
        let tx = self.env.tx_begin_read();
        if !wallet.store.valid_password(&tx) {
            return Err(WalletsError::WalletLocked);
        }
        if wallet.store.find(&tx, account).is_end() {
            return Err(WalletsError::AccountNotFound);
        }
        wallet
            .store
            .fetch(&tx, account)
            .map_err(|_| WalletsError::Generic)
    }

    pub fn import(&self, wallet_id: WalletId, json: &str) -> anyhow::Result<()> {
        let _guard = self.mutex.lock().unwrap();
        let mut tx = self.env.tx_begin_write();
        let _wallet = Wallet::new_from_json(
            Arc::clone(&self.ledger),
            self.work_thresholds.clone(),
            &mut tx,
            self.node_config.password_fanout as usize,
            self.kdf.clone(),
            &PathBuf::from(wallet_id.to_string()),
            json,
        )?;
        Ok(())
    }

    pub fn import_replace(
        &self,
        wallet_id: WalletId,
        json: &str,
        password: &str,
    ) -> anyhow::Result<()> {
        let guard = self.mutex.lock().unwrap();
        let existing = guard
            .get(&wallet_id)
            .ok_or_else(|| anyhow!("wallet not found"))?;
        let mut tx = self.env.tx_begin_write();
        let id = WalletId::from_bytes(thread_rng().gen());
        let temp = LmdbWalletStore::new_from_json(
            1,
            self.kdf.clone(),
            &mut tx,
            &PathBuf::from(id.to_string()),
            json,
        )?;

        let result = if temp.attempt_password(&tx, password) {
            existing.store.import(&mut tx, &temp)
        } else {
            Err(anyhow!("bad password"))
        };
        temp.destroy(&mut tx);
        result
    }
}

const GENERATE_PRIORITY: Amount = Amount::MAX;
const HIGH_PRIORITY: Amount = Amount::raw(u128::MAX - 1);

pub trait WalletsExt<T: Environment = EnvironmentWrapper> {
    fn deterministic_insert(
        &self,
        wallet: &Arc<Wallet<T>>,
        tx: &mut LmdbWriteTransaction<T>,
        generate_work: bool,
    ) -> PublicKey;

    fn deterministic_insert_at(
        &self,
        wallet_id: &WalletId,
        index: u32,
        generate_work: bool,
    ) -> Result<PublicKey, WalletsError>;

    fn deterministic_insert2(
        &self,
        wallet_id: &WalletId,
        generate_work: bool,
    ) -> Result<PublicKey, WalletsError>;

    fn insert_adhoc(&self, wallet: &Arc<Wallet<T>>, key: &RawKey, generate_work: bool) -> Account;

    fn insert_adhoc2(
        &self,
        wallet_id: &WalletId,
        key: &RawKey,
        generate_work: bool,
    ) -> Result<Account, WalletsError>;

    fn work_ensure(&self, wallet: &Arc<Wallet<T>>, account: Account, root: Root);

    fn action_complete(
        &self,
        wallet: Arc<Wallet<T>>,
        block: Option<Arc<BlockEnum>>,
        account: Account,
        generate_work: bool,
        details: &BlockDetails,
    ) -> anyhow::Result<()>;

    fn ongoing_compute_reps(&self);

    fn change_seed(
        &self,
        wallet: &Arc<Wallet<T>>,
        tx: &mut LmdbWriteTransaction<T>,
        prv_key: &RawKey,
        count: u32,
    ) -> PublicKey;

    fn send_action(
        &self,
        wallet: &Arc<Wallet<T>>,
        source: Account,
        account: Account,
        amount: Amount,
        work: u64,
        generate_work: bool,
        id: Option<String>,
    ) -> Option<BlockEnum>;

    fn change_action(
        &self,
        wallet: &Arc<Wallet<T>>,
        source: Account,
        representative: Account,
        work: u64,
        generate_work: bool,
    ) -> Option<BlockEnum>;

    fn receive_action(
        &self,
        wallet: &Arc<Wallet<T>>,
        send_hash: BlockHash,
        representative: Account,
        amount: Amount,
        account: Account,
        work: u64,
        generate_work: bool,
    ) -> Option<BlockEnum>;

    fn receive_async(
        &self,
        wallet: Arc<Wallet<T>>,
        hash: BlockHash,
        representative: Account,
        amount: Amount,
        account: Account,
        action: Box<dyn Fn(Option<BlockEnum>) + Send + Sync>,
        work: u64,
        generate_work: bool,
    );

    fn receive_sync(
        &self,
        wallet: Arc<Wallet<T>>,
        block: &BlockEnum,
        representative: Account,
        amount: Amount,
    ) -> Result<(), ()>;

    fn search_receivable(
        &self,
        wallet: &Arc<Wallet<T>>,
        wallet_tx: &dyn Transaction<Database = T::Database, RoCursor = T::RoCursor>,
    ) -> Result<(), ()>;

    fn receive_confirmed(&self, hash: BlockHash, destinaton: Account);
    fn search_receivable_all(&self);
    fn search_receivable_wallet(&self, wallet_id: WalletId) -> Result<(), WalletsError>;

    fn enter_password(&self, wallet_id: WalletId, password: &str) -> Result<(), WalletsError>;

    fn enter_password_wallet(
        &self,
        wallet: &Arc<Wallet<T>>,
        wallet_tx: &dyn Transaction<Database = T::Database, RoCursor = T::RoCursor>,
        password: &str,
    ) -> Result<(), ()>;

    fn enter_initial_password(&self, wallet: &Arc<Wallet<T>>);
    fn create(&self, wallet_id: WalletId);
    fn change_async_wallet(
        &self,
        wallet: Arc<Wallet<T>>,
        source: Account,
        representative: Account,
        action: Box<dyn Fn(Option<BlockEnum>) + Send + Sync>,
        work: u64,
        generate_work: bool,
    );

    fn change_sync_wallet(
        &self,
        wallet: Arc<Wallet<T>>,
        source: Account,
        representative: Account,
    ) -> Result<(), ()>;
}

impl<T: Environment> WalletsExt<T> for Arc<Wallets<T>> {
    fn deterministic_insert(
        &self,
        wallet: &Arc<Wallet<T>>,
        tx: &mut LmdbWriteTransaction<T>,
        generate_work: bool,
    ) -> PublicKey {
        if !wallet.store.valid_password(tx) {
            return PublicKey::zero();
        }
        let key = wallet.store.deterministic_insert(tx);
        if generate_work {
            self.work_ensure(wallet, key, key.into());
        }
        let half_principal_weight = self.online_reps.lock().unwrap().minimum_principal_weight() / 2;
        let mut reps = self.representatives.lock().unwrap();
        if reps.check_rep(key, half_principal_weight) {
            wallet.representatives.lock().unwrap().insert(key);
        }
        key
    }

    fn deterministic_insert_at(
        &self,
        wallet_id: &WalletId,
        index: u32,
        generate_work: bool,
    ) -> Result<PublicKey, WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Wallets::get_wallet(&guard, wallet_id)?;
        let mut tx = self.env.tx_begin_write();
        if !wallet.store.valid_password(&tx) {
            return Err(WalletsError::WalletLocked);
        }
        let account = wallet.store.deterministic_insert_at(&mut tx, index);
        if generate_work {
            self.work_ensure(wallet, account, account.into());
        }
        Ok(account)
    }

    fn deterministic_insert2(
        &self,
        wallet_id: &WalletId,
        generate_work: bool,
    ) -> Result<PublicKey, WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Wallets::get_wallet(&guard, wallet_id)?;
        let mut tx = self.env.tx_begin_write();
        if !wallet.store.valid_password(&tx) {
            return Err(WalletsError::WalletLocked);
        }
        Ok(self.deterministic_insert(wallet, &mut tx, generate_work))
    }

    fn insert_adhoc(&self, wallet: &Arc<Wallet<T>>, key: &RawKey, generate_work: bool) -> Account {
        let mut tx = self.env.tx_begin_write();
        if !wallet.store.valid_password(&tx) {
            return PublicKey::zero();
        }
        let key = wallet.store.insert_adhoc(&mut tx, key);
        let block_tx = self.ledger.read_txn();
        if generate_work {
            self.work_ensure(wallet, key, self.ledger.latest_root(&block_tx, &key));
        }
        let half_principal_weight = self.online_reps.lock().unwrap().minimum_principal_weight() / 2;
        // Makes sure that the representatives container will
        // be in sync with any added keys.
        tx.commit();
        let mut rep_guard = self.representatives.lock().unwrap();
        if rep_guard.check_rep(key, half_principal_weight) {
            wallet.representatives.lock().unwrap().insert(key);
        }
        key
    }

    fn insert_adhoc2(
        &self,
        wallet_id: &WalletId,
        key: &RawKey,
        generate_work: bool,
    ) -> Result<Account, WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Wallets::get_wallet(&guard, wallet_id)?;
        let mut tx = self.env.tx_begin_read();
        if !wallet.store.valid_password(&tx) {
            return Err(WalletsError::WalletLocked);
        }
        tx.reset();
        Ok(self.insert_adhoc(wallet, key, generate_work))
    }

    fn work_ensure(&self, wallet: &Arc<Wallet<T>>, account: Account, root: Root) {
        let precache_delay = if self.network_params.network.is_dev_network() {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(10)
        };
        self.delayed_work.lock().unwrap().insert(account, root);
        let self_clone = Arc::clone(self);
        let wallet = Arc::clone(wallet);
        self.workers.add_delayed_task(
            precache_delay,
            Box::new(move || {
                let mut guard = self_clone.delayed_work.lock().unwrap();
                if let Some(&existing) = guard.get(&account) {
                    if existing == root {
                        guard.remove(&account);
                        let self_clone_2 = Arc::clone(&self_clone);
                        self_clone.wallet_actions.queue_wallet_action(
                            GENERATE_PRIORITY,
                            wallet,
                            Box::new(move |w| {
                                self_clone_2.work_cache_blocking(&w, &account, &root);
                            }),
                        );
                    }
                }
            }),
        );
    }

    fn action_complete(
        &self,
        wallet: Arc<Wallet<T>>,
        block: Option<Arc<BlockEnum>>,
        account: Account,
        generate_work: bool,
        details: &BlockDetails,
    ) -> anyhow::Result<()> {
        // Unschedule any work caching for this account
        self.delayed_work.lock().unwrap().remove(&account);
        let Some(block) = block else {
            return Ok(());
        };
        let hash = block.hash();
        let required_difficulty = self
            .network_params
            .work
            .threshold2(block.work_version(), details);
        let mut_block = unsafe { block.undefined_behavior_mut() };
        if self.network_params.work.difficulty_block(mut_block) < required_difficulty {
            info!(
                "Cached or provided work for block {} account {} is invalid, regenerating...",
                block.hash(),
                account.encode_account()
            );
            self.distributed_work
                .make_blocking_block(mut_block, required_difficulty)
                .ok_or_else(|| anyhow!("no work generated"))?;
        }
        let result = self.block_processor.add_blocking(block, BlockSource::Local);

        if !matches!(result, Some(BlockStatus::Progress)) {
            bail!("block processor failed: {:?}", result);
        }

        if generate_work {
            // Pregenerate work for next block based on the block just created
            self.work_ensure(&wallet, account, hash.into());
        }
        Ok(())
    }

    fn ongoing_compute_reps(&self) {
        self.compute_reps();

        // Representation drifts quickly on the test network but very slowly on the live network
        let compute_delay = if self.network_params.network.is_dev_network() {
            Duration::from_millis(10)
        } else if self.network_params.network.is_test_network() {
            test_scan_wallet_reps_delay()
        } else {
            Duration::from_secs(60 * 15)
        };

        let self_l = Arc::clone(self);
        self.workers.add_delayed_task(
            compute_delay,
            Box::new(move || {
                self_l.ongoing_compute_reps();
            }),
        );
    }

    fn change_seed(
        &self,
        wallet: &Arc<Wallet<T>>,
        tx: &mut LmdbWriteTransaction<T>,
        prv_key: &RawKey,
        mut count: u32,
    ) -> PublicKey {
        wallet.store.set_seed(tx, prv_key);
        let mut account = self.deterministic_insert(wallet, tx, true);
        if count == 0 {
            count = wallet.deterministic_check(tx, 0);
        }
        for _ in 0..count {
            // Disable work generation to prevent weak CPU nodes stuck
            account = self.deterministic_insert(wallet, tx, false);
        }
        account
    }

    fn send_action(
        &self,
        wallet: &Arc<Wallet<T>>,
        source: Account,
        account: Account,
        amount: Amount,
        work: u64,
        generate_work: bool,
        id: Option<String>,
    ) -> Option<BlockEnum> {
        let (mut block, error, cached_block, details) = match id {
            Some(id) => {
                let mut tx = self.env.tx_begin_write();
                self.prepare_send_with_id(&mut tx, &id, wallet, source, account, amount, work)
            }
            None => {
                let tx = self.env.tx_begin_read();
                self.prepare_send(&tx, wallet, source, account, amount, work)
            }
        };

        if let Some(b) = &block {
            if !error && !cached_block {
                let block_arc = Arc::new(b.clone());
                if self
                    .action_complete(
                        Arc::clone(wallet),
                        Some(Arc::clone(&block_arc)),
                        source,
                        generate_work,
                        &details,
                    )
                    .is_err()
                {
                    // Return null block after work generation or ledger process error
                    block = None;
                } else {
                    // block arc gets changed by block_processor! So we have to copy it back.
                    block = Some(block_arc.deref().clone());
                }
            }
        }

        block
    }

    fn change_action(
        &self,
        wallet: &Arc<Wallet<T>>,
        source: Account,
        representative: Account,
        mut work: u64,
        generate_work: bool,
    ) -> Option<BlockEnum> {
        let mut epoch = Epoch::Epoch0;
        let mut block = None;
        {
            let wallet_tx = self.env.tx_begin_read();
            let block_tx = self.ledger.read_txn();
            if !wallet.store.valid_password(&wallet_tx) {
                return None;
            }

            let existing = wallet.store.find(&wallet_tx, &source);
            if !existing.is_end() && self.ledger.latest(&block_tx, &source).is_some() {
                let info = self.ledger.account_info(&block_tx, &source).unwrap();
                let prv = wallet.store.fetch(&wallet_tx, &source).unwrap();
                if work == 0 {
                    work = wallet
                        .store
                        .work_get(&wallet_tx, &source)
                        .unwrap_or_default();
                }
                let state_block = BlockEnum::State(StateBlock::new(
                    source,
                    info.head,
                    representative,
                    info.balance,
                    Link::zero(),
                    &prv,
                    &source,
                    work,
                ));
                block = Some(state_block);
                epoch = info.epoch;
            }
        }

        if let Some(b) = block {
            let details = BlockDetails::new(epoch, false, false, false);
            let arc_block = Arc::new(b);
            if self
                .action_complete(
                    Arc::clone(&wallet),
                    Some(Arc::clone(&arc_block)),
                    source,
                    generate_work,
                    &details,
                )
                .is_err()
            {
                // Return null block after work generation or ledger process error
                block = None;
            } else {
                // block arc gets changed by block_processor! So we have to copy it back.
                block = Some(arc_block.deref().clone())
            }
        }
        block
    }

    fn receive_action(
        &self,
        wallet: &Arc<Wallet<T>>,
        send_hash: BlockHash,
        representative: Account,
        amount: Amount,
        account: Account,
        mut work: u64,
        generate_work: bool,
    ) -> Option<BlockEnum> {
        if amount < self.node_config.receive_minimum {
            warn!(
                "Not receiving block {} due to minimum receive threshold",
                send_hash
            );
            return None;
        }

        let mut block = None;
        let mut epoch = Epoch::Epoch0;
        let block_tx = self.ledger.read_txn();
        let wallet_tx = self.env.tx_begin_read();
        if self
            .ledger
            .block_or_pruned_exists_txn(&block_tx, &send_hash)
        {
            if let Some(pending_info) = self
                .ledger
                .pending_info(&block_tx, &PendingKey::new(account, send_hash))
            {
                if let Ok(prv) = wallet.store.fetch(&wallet_tx, &account) {
                    if work == 0 {
                        work = wallet
                            .store
                            .work_get(&wallet_tx, &account)
                            .unwrap_or_default();
                    }
                    if let Some(info) = self.ledger.account_info(&block_tx, &account) {
                        block = Some(BlockEnum::State(StateBlock::new(
                            account,
                            info.head,
                            info.representative,
                            info.balance + pending_info.amount,
                            send_hash.into(),
                            &prv,
                            &account,
                            work,
                        )));
                        epoch = std::cmp::max(info.epoch, pending_info.epoch);
                    } else {
                        block = Some(BlockEnum::State(StateBlock::new(
                            account,
                            BlockHash::zero(),
                            representative,
                            pending_info.amount,
                            send_hash.into(),
                            &prv,
                            &account,
                            work,
                        )));
                        epoch = pending_info.epoch;
                    }
                } else {
                    warn!("Unable to receive, wallet locked");
                }
            } else {
                // Ledger doesn't have this marked as available to receive anymore
            }
        } else {
            // Ledger doesn't have this block anymore.
        }

        if let Some(b) = block {
            let details = BlockDetails::new(epoch, false, true, false);
            let arc_block = Arc::new(b);
            if self
                .action_complete(
                    Arc::clone(wallet),
                    Some(Arc::clone(&arc_block)),
                    account,
                    generate_work,
                    &details,
                )
                .is_err()
            {
                // Return null block after work generation or ledger process error
                block = None;
            } else {
                // block arc gets changed by block_processor! So we have to copy it back.
                block = Some(arc_block.deref().clone())
            }
        }

        block
    }

    fn receive_async(
        &self,
        wallet: Arc<Wallet<T>>,
        hash: BlockHash,
        representative: Account,
        amount: Amount,
        account: Account,
        action: Box<dyn Fn(Option<BlockEnum>) + Send + Sync>,
        work: u64,
        generate_work: bool,
    ) {
        let self_l = Arc::clone(self);
        self.wallet_actions.queue_wallet_action(
            amount,
            wallet,
            Box::new(move |wallet| {
                let block = self_l.receive_action(
                    &wallet,
                    hash,
                    representative,
                    amount,
                    account,
                    work,
                    generate_work,
                );
                action(block);
            }),
        );
    }

    fn receive_sync(
        &self,
        wallet: Arc<Wallet<T>>,
        block: &BlockEnum,
        representative: Account,
        amount: Amount,
    ) -> Result<(), ()> {
        let result = Arc::new((Condvar::new(), Mutex::new((false, false)))); // done, result
        let result_clone = Arc::clone(&result);
        self.receive_async(
            wallet,
            block.hash(),
            representative,
            amount,
            block.destination().unwrap(),
            Box::new(move |block| {
                *result_clone.1.lock().unwrap() = (true, block.is_some());
                result_clone.0.notify_all();
            }),
            0,
            true,
        );
        let mut guard = result.1.lock().unwrap();
        guard = result.0.wait_while(guard, |i| !i.0).unwrap();
        if guard.1 {
            Ok(())
        } else {
            Err(())
        }
    }

    fn search_receivable(
        &self,
        wallet: &Arc<Wallet<T>>,
        wallet_tx: &dyn Transaction<
            Database = <T as Environment>::Database,
            RoCursor = <T as Environment>::RoCursor,
        >,
    ) -> Result<(), ()> {
        if !wallet.store.valid_password(wallet_tx) {
            info!("Stopping search, wallet is locked");
            return Err(());
        }

        info!("Beginning receivable block search");

        let mut it = wallet.store.begin(wallet_tx);
        while let Some((account, wallet_value)) = it.current() {
            let block_tx = self.ledger.read_txn();
            // Don't search pending for watch-only accounts
            if !wallet_value.key.is_zero() {
                for (key, info) in self.ledger.account_receivable_upper_bound(
                    &block_tx,
                    *account,
                    BlockHash::zero(),
                ) {
                    let hash = key.send_block_hash;
                    let amount = info.amount;
                    if self.node_config.receive_minimum <= amount {
                        info!(
                            "Found a receivable block {} for account {}",
                            hash,
                            info.source.encode_account()
                        );
                        if self.ledger.block_confirmed(&block_tx, &hash) {
                            let representative = wallet.store.representative(wallet_tx);
                            // Receive confirmed block
                            self.receive_async(
                                Arc::clone(wallet),
                                hash,
                                representative,
                                amount,
                                *account,
                                Box::new(|_| {}),
                                0,
                                true,
                            );
                        } else if !self.confirming_set.exists(&hash) {
                            let block = self.ledger.get_block(&block_tx, &hash);
                            if let Some(block) = block {
                                // Request confirmation for block which is not being processed yet
                                let guard = self.start_election.lock().unwrap();
                                if let Some(callback) = guard.as_ref() {
                                    callback(Arc::new(block));
                                }
                            }
                        }
                    }
                }
            }

            it.next();
        }

        info!("Receivable block search phase completed");
        Ok(())
    }

    fn receive_confirmed(&self, hash: BlockHash, destination: Account) {
        //std::unordered_map<nano::wallet_id, std::shared_ptr<nano::wallet>> wallets_l;
        let (wallet_tx, wallets) = {
            let guard = self.mutex.lock().unwrap();
            (self.env.tx_begin_read(), guard.clone())
        };

        for (_id, wallet) in wallets {
            if wallet.store.exists(&wallet_tx, &destination) {
                let representative = wallet.store.representative(&wallet_tx);
                let pending = self
                    .ledger
                    .pending_info(&self.ledger.read_txn(), &PendingKey::new(destination, hash));
                if let Some(pending) = pending {
                    let amount = pending.amount;
                    self.receive_async(
                        wallet,
                        hash,
                        representative,
                        amount,
                        destination,
                        Box::new(|_| {}),
                        0,
                        true,
                    );
                } else {
                    if !self.ledger.block_or_pruned_exists(&hash) {
                        warn!("Confirmed block is missing:  {}", hash);
                        debug_assert!(false);
                    } else {
                        warn!("Block %1% has already been received: {}", hash);
                    }
                }
            }
        }
    }

    fn search_receivable_all(&self) {
        let wallets = self.mutex.lock().unwrap().clone();
        let wallet_tx = self.env.tx_begin_read();
        for (_, wallet) in wallets {
            let _ = self.search_receivable(&wallet, &wallet_tx);
        }
    }

    fn search_receivable_wallet(&self, wallet_id: WalletId) -> Result<(), WalletsError> {
        let guard = self.mutex.lock().unwrap();
        if let Some(wallet) = guard.get(&wallet_id) {
            let tx = self.env.tx_begin_read();
            if wallet.store.valid_password(&tx) {
                let _ = self.search_receivable(wallet, &tx);
                Ok(())
            } else {
                Err(WalletsError::WalletLocked)
            }
        } else {
            Err(WalletsError::WalletNotFound)
        }
    }

    fn enter_password(&self, wallet_id: WalletId, password: &str) -> Result<(), WalletsError> {
        let guard = self.mutex.lock().unwrap();
        let wallet = Wallets::get_wallet(&guard, &wallet_id)?;
        let tx = self.env.tx_begin_write();
        self.enter_password_wallet(wallet, &tx, password)
            .map_err(|_| WalletsError::InvalidPassword)
    }

    fn enter_password_wallet(
        &self,
        wallet: &Arc<Wallet<T>>,
        wallet_tx: &dyn Transaction<
            Database = <T as Environment>::Database,
            RoCursor = <T as Environment>::RoCursor,
        >,
        password: &str,
    ) -> Result<(), ()> {
        if !wallet.store.attempt_password(wallet_tx, password) {
            warn!("Invalid password, wallet locked");
            Err(())
        } else {
            info!("Wallet unlocked");
            let self_l = Arc::clone(self);
            self.wallet_actions.queue_wallet_action(
                HIGH_PRIORITY,
                Arc::clone(wallet),
                Box::new(move |wallet| {
                    // Wallets must survive node lifetime
                    let tx = self_l.env.tx_begin_read();
                    let _ = self_l.search_receivable(&wallet, &tx);
                }),
            );
            Ok(())
        }
    }

    fn enter_initial_password(&self, wallet: &Arc<Wallet<T>>) {
        let password = wallet.store.password();
        if password.is_zero() {
            let mut tx = self.env.tx_begin_write();
            if wallet.store.valid_password(&tx) {
                // Newly created wallets have a zero key
                let _ = wallet.store.rekey(&mut tx, "");
            } else {
                let _ = self.enter_password_wallet(wallet, &tx, "");
            }
        }
    }

    fn create(&self, wallet_id: WalletId) {
        let mut guard = self.mutex.lock().unwrap();
        debug_assert!(!guard.contains_key(&wallet_id));
        let wallet = {
            let mut tx = self.env.tx_begin_write();
            let Ok(wallet) = Wallet::new(
                Arc::clone(&self.ledger),
                self.work_thresholds.clone(),
                &mut tx,
                self.node_config.password_fanout as usize,
                self.kdf.clone(),
                self.node_config.random_representative(),
                &PathBuf::from(wallet_id.to_string()),
            ) else {
                return;
            };
            Arc::new(wallet)
        };
        guard.insert(wallet_id, Arc::clone(&wallet));
        self.enter_initial_password(&wallet);
    }

    fn change_async_wallet(
        &self,
        wallet: Arc<Wallet<T>>,
        source: Account,
        representative: Account,
        action: Box<dyn Fn(Option<BlockEnum>) + Send + Sync>,
        work: u64,
        generate_work: bool,
    ) {
        let self_l = Arc::clone(self);
        self.wallet_actions.queue_wallet_action(
            HIGH_PRIORITY,
            wallet,
            Box::new(move |wallet| {
                let block =
                    self_l.change_action(&wallet, source, representative, work, generate_work);
                action(block);
            }),
        );
    }

    fn change_sync_wallet(
        &self,
        wallet: Arc<Wallet<T>>,
        source: Account,
        representative: Account,
    ) -> Result<(), ()> {
        let result = Arc::new((Condvar::new(), Mutex::new((false, false)))); // done, result
        let result_clone = Arc::clone(&result);
        self.change_async_wallet(
            wallet,
            source,
            representative,
            Box::new(move |block| {
                *result_clone.1.lock().unwrap() = (true, block.is_some());
                result_clone.0.notify_all();
            }),
            0,
            true,
        );
        let mut guard = result.1.lock().unwrap();
        guard = result.0.wait_while(guard, |i| !i.0).unwrap();
        if guard.1 {
            Ok(())
        } else {
            Err(())
        }
    }
}

fn test_scan_wallet_reps_delay() -> Duration {
    let test_env = get_env_or_default_string("NANO_TEST_WALLET_SCAN_REPS_DELAY", "900000"); // 15 minutes by default
    Duration::from_millis(test_env.parse().unwrap())
}
