use crate::{
    ledger::datastore::LedgerHandle,
    transport::{ChannelHandle, EndpointDto},
    NetworkConstantsDto, StatHandle,
};
use rsnano_core::{Account, Amount};
use rsnano_node::{
    config::NetworkConstants,
    representatives::{RegisterRepresentativeResult, Representative, RepresentativeRegister},
};
use std::{
    ops::Deref,
    sync::{Arc, Mutex},
};

use super::{representative::RepresentativeHandle, OnlineRepsHandle};

pub struct RepresentativeRegisterHandle(Arc<Mutex<RepresentativeRegister>>);

impl Deref for RepresentativeRegisterHandle {
    type Target = Arc<Mutex<RepresentativeRegister>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[no_mangle]
pub extern "C" fn rsn_representative_register_create(
    ledger: &LedgerHandle,
    online_reps: &OnlineRepsHandle,
    stats: &StatHandle,
    network_constants: &NetworkConstantsDto,
) -> *mut RepresentativeRegisterHandle {
    Box::into_raw(Box::new(RepresentativeRegisterHandle(Arc::new(
        Mutex::new(RepresentativeRegister::new(
            Arc::clone(ledger),
            Arc::clone(online_reps),
            Arc::clone(stats),
            NetworkConstants::try_from(network_constants)
                .unwrap()
                .protocol_info(),
        )),
    ))))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_register_destroy(
    handle: *mut RepresentativeRegisterHandle,
) {
    drop(Box::from_raw(handle))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_register_update_or_insert(
    handle: &mut RepresentativeRegisterHandle,
    account: *const u8,
    channel: &ChannelHandle,
    old_endpoint: &mut EndpointDto,
) -> u32 {
    let account = Account::from_ptr(account);
    let mut guard = handle.0.lock().unwrap();
    match guard.update_or_insert(account, Arc::clone(channel)) {
        RegisterRepresentativeResult::Inserted => 0,
        RegisterRepresentativeResult::Updated => 1,
        RegisterRepresentativeResult::ChannelChanged(addr) => {
            *old_endpoint = addr.into();
            2
        }
    }
}

#[no_mangle]
pub extern "C" fn rsn_representative_register_is_pr(
    handle: &RepresentativeRegisterHandle,
    channel: &ChannelHandle,
) -> bool {
    handle.0.lock().unwrap().is_pr(channel)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_register_total_weight(
    handle: &RepresentativeRegisterHandle,
    result: *mut u8,
) {
    let weight = handle.lock().unwrap().total_weight();
    weight.copy_bytes(result);
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_register_on_rep_request(
    handle: &mut RepresentativeRegisterHandle,
    channel: &ChannelHandle,
) {
    handle.lock().unwrap().on_rep_request(channel)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_register_cleanup_reps(
    handle: &mut RepresentativeRegisterHandle,
) {
    handle.lock().unwrap().cleanup_reps()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_register_last_request_elapsed_ms(
    handle: &RepresentativeRegisterHandle,
    channel: &ChannelHandle,
) -> i64 {
    handle
        .lock()
        .unwrap()
        .last_request_elapsed(channel)
        .map(|i| i.as_millis() as i64)
        .unwrap_or(-1)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_register_representatives(
    handle: &RepresentativeRegisterHandle,
    max_results: usize,
    min_weight: *const u8,
    min_version: u8,
) -> *mut RepresentativeListHandle {
    let min_weight = Amount::from_ptr(min_weight);
    let min_version = if min_version == 0 {
        None
    } else {
        Some(min_version)
    };

    let resp = handle
        .lock()
        .unwrap()
        .representatives_filter(max_results, min_weight, min_version);

    Box::into_raw(Box::new(RepresentativeListHandle(resp)))
}

pub struct RepresentativeListHandle(Vec<Representative>);

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_list_destroy(handle: *mut RepresentativeListHandle) {
    drop(Box::from_raw(handle))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_list_len(handle: &RepresentativeListHandle) -> usize {
    handle.0.len()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_list_get(
    handle: &RepresentativeListHandle,
    index: usize,
) -> *mut RepresentativeHandle {
    let rep = handle.0.get(index).unwrap().clone();
    Box::into_raw(Box::new(RepresentativeHandle(rep)))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_representative_register_count(
    handle: &RepresentativeRegisterHandle,
) -> usize {
    handle.0.lock().unwrap().representatives_count()
}
