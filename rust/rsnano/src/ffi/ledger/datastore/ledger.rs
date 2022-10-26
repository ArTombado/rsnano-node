use crate::{
    core::{Account, Amount, BlockHash},
    ffi::{
        copy_amount_bytes,
        ledger::{GenerateCacheHandle, LedgerCacheHandle, LedgerConstantsDto},
        StatHandle,
    },
    ledger::Ledger,
};
use std::{
    ffi::c_void,
    ops::Deref,
    sync::{atomic::Ordering, Arc},
};

use super::lmdb::{LmdbStoreHandle, TransactionHandle};

pub struct LedgerHandle(Arc<Ledger>);

impl Deref for LedgerHandle {
    type Target = Arc<Ledger>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_create(
    handle: *mut c_void,
    store: *mut LmdbStoreHandle,
    constants: *const LedgerConstantsDto,
    stats: *mut StatHandle,
    generate_cache: *mut GenerateCacheHandle,
) -> *mut LedgerHandle {
    let ledger = Ledger::new(
        handle,
        (*store).deref().to_owned(),
        (&*constants).try_into().unwrap(),
        (*stats).deref().to_owned(),
        &*generate_cache,
    )
    .unwrap();
    Box::into_raw(Box::new(LedgerHandle(Arc::new(ledger))))
}

#[no_mangle]
pub extern "C" fn rsn_ledger_destroy(handle: *mut LedgerHandle) {
    drop(unsafe { Box::from_raw(handle) });
}

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_ledger_block_or_pruned_exists(
    f: LedgerBlockOrPrunedExistsCallback,
) {
    BLOCK_OR_PRUNED_EXISTS_CALLBACK = Some(f);
}

type LedgerBlockOrPrunedExistsCallback = unsafe extern "C" fn(*mut c_void, *const u8) -> bool;
pub(crate) static mut BLOCK_OR_PRUNED_EXISTS_CALLBACK: Option<LedgerBlockOrPrunedExistsCallback> =
    None;

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_pruning_enabled(handle: *mut LedgerHandle) -> bool {
    (*handle).0.pruning_enabled()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_enable_pruning(handle: *mut LedgerHandle) {
    (*handle).0.enable_pruning()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_bootstrap_weight_max_blocks(handle: *mut LedgerHandle) -> u64 {
    (*handle).0.bootstrap_weight_max_blocks()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_set_bootstrap_weight_max_blocks(
    handle: *mut LedgerHandle,
    max: u64,
) {
    (*handle).0.set_bootstrap_weight_max_blocks(max)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_check_bootstrap_weights(handle: *mut LedgerHandle) -> bool {
    (*handle).0.check_bootstrap_weights.load(Ordering::SeqCst)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_set_check_bootstrap_weights(
    handle: *mut LedgerHandle,
    check: bool,
) {
    (*handle)
        .0
        .check_bootstrap_weights
        .store(check, Ordering::SeqCst)
}

#[repr(C)]
pub struct BootstrapWeightsItem {
    pub account: [u8; 32],
    pub weight: [u8; 16],
}

pub struct BootstrapWeightsRawPtr(Vec<BootstrapWeightsItem>);

#[repr(C)]
pub struct BootstrapWeightsDto {
    pub accounts: *const BootstrapWeightsItem,
    pub count: usize,
    pub raw_ptr: *mut BootstrapWeightsRawPtr,
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_bootstrap_weights(
    handle: *mut LedgerHandle,
    result: *mut BootstrapWeightsDto,
) {
    let weights = (*handle).0.bootstrap_weights.lock().unwrap().to_owned();
    let items = weights
        .iter()
        .map(|(k, v)| BootstrapWeightsItem {
            account: *k.as_bytes(),
            weight: v.to_be_bytes(),
        })
        .collect();
    let raw_ptr = Box::new(BootstrapWeightsRawPtr(items));

    (*result).count = raw_ptr.0.len();
    (*result).accounts = raw_ptr.0.as_ptr();
    (*result).raw_ptr = Box::into_raw(raw_ptr);
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_destroy_bootstrap_weights_dto(dto: *mut BootstrapWeightsDto) {
    drop(Box::from_raw((*dto).raw_ptr))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_set_bootstrap_weights(
    handle: *mut LedgerHandle,
    accounts: *const BootstrapWeightsItem,
    count: usize,
) {
    let dtos = std::slice::from_raw_parts(accounts, count);
    let weights = dtos
        .iter()
        .map(|d| {
            (
                Account::from_bytes(d.account),
                u128::from_be_bytes(d.weight),
            )
        })
        .collect();
    *(*handle).0.bootstrap_weights.lock().unwrap() = weights;
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_get_cache_handle(
    handle: *mut LedgerHandle,
) -> *mut LedgerCacheHandle {
    LedgerCacheHandle::new((*handle).0.cache.clone())
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_balance(
    handle: *mut LedgerHandle,
    txn: *mut TransactionHandle,
    hash: *const u8,
    result: *mut u8,
) {
    let balance = (*handle).balance((*txn).as_txn(), &BlockHash::from_ptr(hash));
    copy_amount_bytes(balance, result);
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_balance_safe(
    handle: *mut LedgerHandle,
    txn: *mut TransactionHandle,
    hash: *const u8,
    result: *mut u8,
) -> bool {
    match (*handle).balance_safe((*txn).as_txn(), &BlockHash::from_ptr(hash)) {
        Ok(balance) => {
            copy_amount_bytes(balance, result);
            true
        }
        Err(_) => {
            copy_amount_bytes(Amount::zero(), result);
            false
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_account_balance(
    handle: *mut LedgerHandle,
    txn: *mut TransactionHandle,
    account: *const u8,
    only_confirmed: bool,
    result: *mut u8,
) {
    let balance =
        (*handle)
            .0
            .account_balance((*txn).as_txn(), &Account::from_ptr(account), only_confirmed);
    copy_amount_bytes(balance, result);
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_account_receivable(
    handle: *mut LedgerHandle,
    txn: *mut TransactionHandle,
    account: *const u8,
    only_confirmed: bool,
    result: *mut u8,
) {
    let balance = (*handle).0.account_receivable(
        (*txn).as_txn(),
        &Account::from_ptr(account),
        only_confirmed,
    );
    copy_amount_bytes(balance, result);
}

#[no_mangle]
pub unsafe extern "C" fn rsn_ledger_block_confirmed(
    handle: *mut LedgerHandle,
    txn: *mut TransactionHandle,
    hash: *const u8,
) -> bool {
    (*handle)
        .0
        .block_confirmed((*txn).as_txn(), &BlockHash::from_ptr(hash))
}
