use super::{
    channel::{as_tcp_channel, ChannelHandle, ChannelType},
    socket::SocketHandle,
    EndpointDto,
};
use crate::{
    ffi::{messages::MessageHandle, DestroyCallback},
    messages::Message,
    transport::{ChannelTcp, ChannelTcpObserver, TcpChannelData},
};
use std::{
    ffi::c_void,
    net::SocketAddr,
    ops::Deref,
    sync::{Arc, MutexGuard},
};

#[no_mangle]
/// observer is `weak_ptr<channel_tcp_observer> *`
pub unsafe extern "C" fn rsn_channel_tcp_create(
    now: u64,
    socket: *mut SocketHandle,
    observer: *mut c_void,
) -> *mut ChannelHandle {
    let observer = ChannelTcpObserverWeakPtr::new(observer);
    Box::into_raw(Box::new(ChannelHandle::new(Arc::new(ChannelType::Tcp(
        ChannelTcp::new((*socket).deref(), now, observer),
    )))))
}

pub struct TcpChannelLockHandle(MutexGuard<'static, TcpChannelData>);

#[no_mangle]
pub unsafe extern "C" fn rsn_channel_tcp_lock(
    handle: *mut ChannelHandle,
) -> *mut TcpChannelLockHandle {
    let tcp = as_tcp_channel(handle);
    Box::into_raw(Box::new(TcpChannelLockHandle(std::mem::transmute::<
        MutexGuard<TcpChannelData>,
        MutexGuard<'static, TcpChannelData>,
    >(tcp.lock()))))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_channel_tcp_unlock(handle: *mut TcpChannelLockHandle) {
    drop(Box::from_raw(handle))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_channel_tcp_socket(handle: *mut ChannelHandle) -> *mut SocketHandle {
    let tcp = as_tcp_channel(handle);
    match tcp.socket() {
        Some(s) => SocketHandle::new(s),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsn_channel_tcp_network_version(handle: *mut ChannelHandle) -> u8 {
    let tcp = as_tcp_channel(handle);
    tcp.network_version()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_channel_tcp_network_set_version(
    handle: *mut ChannelHandle,
    version: u8,
) {
    let tcp = as_tcp_channel(handle);
    tcp.set_network_version(version)
}

pub struct FfiChannelTcpObserver {
    /// is a `shared_ptr<channel_tcp_observer> *`
    handle: *mut c_void,
}

impl FfiChannelTcpObserver {
    pub fn new(handle: *mut c_void) -> Self {
        Self { handle }
    }
}

impl ChannelTcpObserver for FfiChannelTcpObserver {
    fn data_sent(&self, endpoint: &SocketAddr) {
        let dto = EndpointDto::from(endpoint);
        unsafe {
            DATA_SENT.expect("DATA_SENT missing")(self.handle, &dto);
        }
    }

    fn host_unreachable(&self) {
        unsafe {
            HOST_UNREACHABLE.expect("HOST_UNREACHABLE missing")(self.handle);
        }
    }

    fn message_sent(&self, message: &dyn Message) {
        unsafe {
            MESSAGE_SENT.expect("MESSAGE_SENT missing")(
                self.handle,
                MessageHandle::new(message.clone_box()),
            );
        }
    }

    fn message_dropped(&self, message: &dyn Message, buffer_size: usize) {
        unsafe {
            MESSAGE_DROPPED.expect("MESSAGE_DROPPED missing")(
                self.handle,
                MessageHandle::new(message.clone_box()),
                buffer_size,
            );
        }
    }

    fn no_socket_drop(&self) {
        unsafe {
            NO_SOCKET_DROP.expect("NO_SOCKET_DROP missing")(self.handle);
        }
    }

    fn write_drop(&self) {
        unsafe {
            WRITE_DROP.expect("WRITE_DROP missing")(self.handle);
        }
    }
}

impl Drop for FfiChannelTcpObserver {
    fn drop(&mut self) {
        unsafe {
            DESTROY_OBSERVER.expect("DESTROY_OBSERVER missing")(self.handle);
        }
    }
}

pub type ChannelTcpObserverDataSentCallback = unsafe extern "C" fn(*mut c_void, *const EndpointDto);
static mut DATA_SENT: Option<ChannelTcpObserverDataSentCallback> = None;

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_channel_tcp_observer_data_sent(
    f: ChannelTcpObserverDataSentCallback,
) {
    DATA_SENT = Some(f);
}

pub type ChannelTcpObserverCallback = unsafe extern "C" fn(*mut c_void);
static mut HOST_UNREACHABLE: Option<ChannelTcpObserverCallback> = None;

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_channel_tcp_observer_host_unreachable(
    f: ChannelTcpObserverCallback,
) {
    HOST_UNREACHABLE = Some(f);
}

pub type ChannelTcpObserverMessageSentCallback =
    unsafe extern "C" fn(*mut c_void, message: *mut MessageHandle);
static mut MESSAGE_SENT: Option<ChannelTcpObserverMessageSentCallback> = None;

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_channel_tcp_observer_message_sent(
    f: ChannelTcpObserverMessageSentCallback,
) {
    MESSAGE_SENT = Some(f);
}

pub type ChannelTcpObserverMessageDroppedCallback =
    unsafe extern "C" fn(*mut c_void, message: *mut MessageHandle, usize);
static mut MESSAGE_DROPPED: Option<ChannelTcpObserverMessageDroppedCallback> = None;

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_channel_tcp_observer_message_dropped(
    f: ChannelTcpObserverMessageDroppedCallback,
) {
    MESSAGE_DROPPED = Some(f);
}

static mut NO_SOCKET_DROP: Option<ChannelTcpObserverCallback> = None;

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_channel_tcp_observer_no_socket_drop(
    f: ChannelTcpObserverCallback,
) {
    NO_SOCKET_DROP = Some(f);
}

static mut WRITE_DROP: Option<ChannelTcpObserverCallback> = None;

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_channel_tcp_observer_write_drop(
    f: ChannelTcpObserverCallback,
) {
    WRITE_DROP = Some(f);
}

static mut DESTROY_OBSERVER: Option<DestroyCallback> = None;

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_channel_tcp_observer_destroy(f: DestroyCallback) {
    DESTROY_OBSERVER = Some(f);
}

pub struct ChannelTcpObserverWeakPtr {
    /// `weak_ptr<channel_tcp_observer> *`
    handle: *mut c_void,
}

impl ChannelTcpObserverWeakPtr {
    pub fn new(handle: *mut c_void) -> Self {
        Self { handle }
    }
    pub fn lock(&self) -> Option<Arc<dyn ChannelTcpObserver>> {
        let shared_ptr_handle =
            unsafe { LOCK_OBSERVER.expect("LOCK_OBSERVER missing")(self.handle) };
        if shared_ptr_handle.is_null() {
            None
        } else {
            Some(Arc::new(FfiChannelTcpObserver::new(shared_ptr_handle)))
        }
    }
}

impl Drop for ChannelTcpObserverWeakPtr {
    fn drop(&mut self) {
        unsafe { DROP_WEAK_PTR.expect("DROP_WEAK_PTR missing")(self.handle) }
    }
}

/// input is a `weak_ptr<channel_tcp_observer> *`
/// output is a `shared_ptr<channel_tcp_observer> *` or `nullptr`
pub type ChannelTcpObserverLockWeakCallback = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
static mut LOCK_OBSERVER: Option<ChannelTcpObserverLockWeakCallback> = None;

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_channel_tcp_observer_lock(
    f: ChannelTcpObserverLockWeakCallback,
) {
    LOCK_OBSERVER = Some(f);
}

static mut DROP_WEAK_PTR: Option<DestroyCallback> = None;

#[no_mangle]
pub unsafe extern "C" fn rsn_callback_channel_tcp_observer_drop_weak(f: DestroyCallback) {
    DROP_WEAK_PTR = Some(f);
}
