use std::{
    ffi::{c_char, c_void, CStr},
    net::{IpAddr, Ipv6Addr, SocketAddr},
    ops::Deref,
    sync::{atomic::Ordering, Arc},
};

use rsnano_core::{utils::system_time_from_nanoseconds, KeyPair, PublicKey};
use rsnano_node::{
    config::NodeConfig,
    messages::{NodeIdHandshakeQuery, NodeIdHandshakeResponse},
    transport::{
        ChannelEnum, TcpChannels, TcpChannelsExtension, TcpChannelsOptions, TcpEndpointAttempt,
    },
    NetworkParams,
};

use crate::{
    bootstrap::{FfiBootstrapServerObserver, RequestResponseVisitorFactoryHandle, TcpServerHandle},
    core::BlockUniquerHandle,
    messages::{HandshakeResponseDto, MessageHandle},
    utils::{
        ptr_into_ipv6addr, ContainerInfoComponentHandle, ContextWrapper, FfiIoContext,
        IoContextHandle, LoggerHandle, LoggerMT,
    },
    voting::VoteUniquerHandle,
    NetworkParamsDto, NodeConfigDto, NodeFlagsHandle, StatHandle, VoidPointerCallback,
};

use super::{
    peer_exclusion::PeerExclusionHandle, ChannelHandle, EndpointDto, NetworkFilterHandle,
    OutboundBandwidthLimiterHandle, SocketHandle, SynCookiesHandle, TcpMessageManagerHandle,
};

pub struct TcpChannelsHandle(Arc<TcpChannels>);

pub type SinkCallback = unsafe extern "C" fn(*mut c_void, *mut MessageHandle, *mut ChannelHandle);

#[repr(C)]
pub struct TcpChannelsOptionsDto {
    pub node_config: *const NodeConfigDto,
    pub logger: *mut LoggerHandle,
    pub publish_filter: *mut NetworkFilterHandle,
    pub io_ctx: *mut IoContextHandle,
    pub network: *mut NetworkParamsDto,
    pub stats: *mut StatHandle,
    pub block_uniquer: *mut BlockUniquerHandle,
    pub vote_uniquer: *mut VoteUniquerHandle,
    pub tcp_message_manager: *mut TcpMessageManagerHandle,
    pub port: u16,
    pub flags: *mut NodeFlagsHandle,
    pub sink_handle: *mut c_void,
    pub sink_callback: SinkCallback,
    pub delete_sink: VoidPointerCallback,
    pub limiter: *mut OutboundBandwidthLimiterHandle,
    pub node_id_prv: *const u8,
    pub syn_cookies: *mut SynCookiesHandle,
}

impl TryFrom<&TcpChannelsOptionsDto> for TcpChannelsOptions {
    type Error = anyhow::Error;

    fn try_from(value: &TcpChannelsOptionsDto) -> Result<Self, Self::Error> {
        unsafe {
            let context_wrapper = ContextWrapper::new(value.sink_handle, value.delete_sink);
            let callback = value.sink_callback;
            let sink = Box::new(move |msg, channel| {
                callback(
                    context_wrapper.get_context(),
                    MessageHandle::new(msg),
                    ChannelHandle::new(channel),
                )
            });

            Ok(Self {
                node_config: NodeConfig::try_from(&*value.node_config)?,
                logger: Arc::new(LoggerMT::new(Box::from_raw(value.logger))),
                publish_filter: (*value.publish_filter).0.clone(),
                io_ctx: Arc::new(FfiIoContext::new((*value.io_ctx).raw_handle())),
                network: NetworkParams::try_from(&*value.network)?,
                stats: (*value.stats).0.clone(),
                block_uniquer: (*value.block_uniquer).deref().clone(),
                vote_uniquer: (*value.vote_uniquer).deref().clone(),
                tcp_message_manager: (*value.tcp_message_manager).deref().clone(),
                port: value.port,
                flags: (*value.flags).0.lock().unwrap().clone(),
                sink,
                limiter: (*value.limiter).0.clone(),
                node_id: KeyPair::from_priv_key_bytes(std::slice::from_raw_parts(
                    value.node_id_prv,
                    32,
                ))
                .unwrap(),
                syn_cookies: (*value.syn_cookies).0.clone(),
            })
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_create(
    options: &TcpChannelsOptionsDto,
) -> *mut TcpChannelsHandle {
    Box::into_raw(Box::new(TcpChannelsHandle(Arc::new(TcpChannels::new(
        TcpChannelsOptions::try_from(options).unwrap(),
    )))))
}

#[no_mangle]
pub extern "C" fn rsn_tcp_channels_set_port(handle: &mut TcpChannelsHandle, port: u16) {
    handle.0.port.store(port, Ordering::SeqCst)
}

pub type NewChannelCallback = unsafe extern "C" fn(*mut c_void, *mut ChannelHandle);

#[no_mangle]
pub extern "C" fn rsn_tcp_channels_stop(handle: &mut TcpChannelsHandle) {
    handle.0.stop();
}

#[no_mangle]
pub extern "C" fn rsn_tcp_channels_max_ip_connections(
    handle: &mut TcpChannelsHandle,
    endpoint: &EndpointDto,
) -> bool {
    handle.0.max_ip_connections(&endpoint.into())
}

#[no_mangle]
pub extern "C" fn rsn_tcp_channels_on_new_channel(
    handle: &mut TcpChannelsHandle,
    callback_handle: *mut c_void,
    call_callback: NewChannelCallback,
    delete_callback: VoidPointerCallback,
) {
    let context_wrapper = ContextWrapper::new(callback_handle, delete_callback);
    let callback = Arc::new(move |channel| {
        let ctx = context_wrapper.get_context();
        unsafe { call_callback(ctx, ChannelHandle::new(channel)) };
    });
    handle.0.on_new_channel(callback)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_destroy(handle: *mut TcpChannelsHandle) {
    drop(Box::from_raw(handle))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_erase_attempt(
    handle: *mut TcpChannelsHandle,
    endpoint: &EndpointDto,
) {
    (*handle)
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .attempts
        .remove(&endpoint.into());
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_get_attempt_count_by_ip_address(
    handle: *mut TcpChannelsHandle,
    ipv6_bytes: *const u8,
) -> usize {
    (*handle)
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .attempts
        .count_by_address(&ptr_into_ipv6addr(ipv6_bytes))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_get_attempt_count_by_subnetwork(
    handle: *mut TcpChannelsHandle,
    ipv6_bytes: *const u8,
) -> usize {
    (*handle)
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .attempts
        .count_by_subnetwork(&ptr_into_ipv6addr(ipv6_bytes))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_add_attempt(
    handle: *mut TcpChannelsHandle,
    endpoint: &EndpointDto,
) -> bool {
    let attempt = TcpEndpointAttempt::new(endpoint.into());
    let mut guard = (*handle).0.tcp_channels.lock().unwrap();
    guard.attempts.insert(attempt.into())
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_attempts_count(handle: *mut TcpChannelsHandle) -> usize {
    let guard = (*handle).0.tcp_channels.lock().unwrap();
    guard.attempts.len()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_purge(handle: *mut TcpChannelsHandle, cutoff_ns: u64) {
    let cutoff = system_time_from_nanoseconds(cutoff_ns);
    let mut guard = (*handle).0.tcp_channels.lock().unwrap();
    guard.purge(cutoff)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_erase_channel_by_node_id(
    handle: &mut TcpChannelsHandle,
    node_id: *const u8,
) {
    handle
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .channels
        .remove_by_node_id(&PublicKey::from_ptr(node_id))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_erase_channel_by_endpoint(
    handle: &mut TcpChannelsHandle,
    endpoint: &EndpointDto,
) {
    handle
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .channels
        .remove_by_endpoint(&SocketAddr::from(endpoint));
}

#[no_mangle]
pub extern "C" fn rsn_tcp_channels_channel_exists(
    handle: &mut TcpChannelsHandle,
    endpoint: &EndpointDto,
) -> bool {
    handle
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .channels
        .exists(&SocketAddr::from(endpoint))
}

#[no_mangle]
pub extern "C" fn rsn_tcp_channels_channel_count(handle: &mut TcpChannelsHandle) -> usize {
    handle.0.tcp_channels.lock().unwrap().channels.len()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_insert(
    handle: &mut TcpChannelsHandle,
    channel: &ChannelHandle,
    socket: &SocketHandle,
    server: *const TcpServerHandle,
) -> bool {
    let server = if server.is_null() {
        None
    } else {
        Some((*server).0.clone())
    };
    handle.0.insert(&channel.0, &socket.0, server).is_err()
}

#[no_mangle]
pub extern "C" fn rsn_tcp_channels_bootstrap_peer(
    handle: &mut TcpChannelsHandle,
    result: &mut EndpointDto,
) {
    let peer = handle.0.tcp_channels.lock().unwrap().bootstrap_peer();
    *result = peer.into();
}

#[no_mangle]
pub extern "C" fn rsn_tcp_channels_close_channels(handle: &mut TcpChannelsHandle) {
    handle.0.tcp_channels.lock().unwrap().close_channels();
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_count_by_ip(
    handle: &mut TcpChannelsHandle,
    ip: *const u8,
) -> usize {
    let address_bytes: [u8; 16] = std::slice::from_raw_parts(ip, 16).try_into().unwrap();
    let ip_address = Ipv6Addr::from(address_bytes);
    handle
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .channels
        .count_by_ip(&ip_address)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_count_by_subnet(
    handle: &mut TcpChannelsHandle,
    subnet: *const u8,
) -> usize {
    let address_bytes: [u8; 16] = std::slice::from_raw_parts(subnet, 16).try_into().unwrap();
    let subnet = Ipv6Addr::from(address_bytes);
    handle
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .channels
        .count_by_subnet(&subnet)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_list_channels(
    handle: &mut TcpChannelsHandle,
    min_version: u8,
    include_temporary_channels: bool,
) -> *mut ChannelListHandle {
    let channels = handle
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .list(min_version, include_temporary_channels);
    Box::into_raw(Box::new(ChannelListHandle(channels)))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_keepalive_list(
    handle: &mut TcpChannelsHandle,
) -> *mut ChannelListHandle {
    let channels = handle.0.tcp_channels.lock().unwrap().keepalive_list();
    Box::into_raw(Box::new(ChannelListHandle(channels)))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_update_channel(
    handle: &mut TcpChannelsHandle,
    endpoint: &EndpointDto,
) {
    handle
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .update(&endpoint.into())
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_set_last_packet_sent(
    handle: &mut TcpChannelsHandle,
    endpoint: &EndpointDto,
    time_ns: u64,
) {
    handle
        .0
        .tcp_channels
        .lock()
        .unwrap()
        .set_last_packet_sent(&endpoint.into(), system_time_from_nanoseconds(time_ns));
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_not_a_peer(
    handle: &mut TcpChannelsHandle,
    endpoint: &EndpointDto,
    allow_local_peers: bool,
) -> bool {
    handle.0.not_a_peer(&endpoint.into(), allow_local_peers)
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_find_channel(
    handle: &mut TcpChannelsHandle,
    endpoint: &EndpointDto,
) -> *mut ChannelHandle {
    match handle.0.find_channel(&endpoint.into()) {
        Some(channel) => ChannelHandle::new(channel),
        None => std::ptr::null_mut(),
    }
}
#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_random_channels(
    handle: &mut TcpChannelsHandle,
    count: usize,
    min_version: u8,
    include_temporary_channels: bool,
) -> *mut ChannelListHandle {
    let channels = handle
        .0
        .random_channels(count, min_version, include_temporary_channels);
    Box::into_raw(Box::new(ChannelListHandle(channels)))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_get_peers(
    handle: &mut TcpChannelsHandle,
) -> *mut EndpointListHandle {
    let peers = handle.0.get_peers();
    Box::into_raw(Box::new(EndpointListHandle(peers)))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_get_first_channel(
    handle: &mut TcpChannelsHandle,
) -> *mut ChannelHandle {
    ChannelHandle::new(handle.0.get_first_channel().unwrap())
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_find_node_id(
    handle: &mut TcpChannelsHandle,
    node_id: *const u8,
) -> *mut ChannelHandle {
    let node_id = PublicKey::from_ptr(node_id);
    match handle.0.find_node_id(&node_id) {
        Some(channel) => ChannelHandle::new(channel),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_collect_container_info(
    handle: &TcpChannelsHandle,
    name: *const c_char,
) -> *mut ContainerInfoComponentHandle {
    let container_info = (*handle)
        .0
        .collect_container_info(CStr::from_ptr(name).to_str().unwrap().to_owned());
    Box::into_raw(Box::new(ContainerInfoComponentHandle(container_info)))
}

#[no_mangle]
pub extern "C" fn rsn_tcp_channels_erase_temporary_channel(
    handle: &TcpChannelsHandle,
    endpoint: &EndpointDto,
) {
    handle.0.erase_temporary_channel(&endpoint.into())
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_random_fill(
    handle: &TcpChannelsHandle,
    endpoints: *mut EndpointDto,
) {
    let endpoints = std::slice::from_raw_parts_mut(endpoints, 8);
    let null_endpoint = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0);
    let mut tmp = [null_endpoint; 8];
    handle.0.random_fill(&mut tmp);
    endpoints
        .iter_mut()
        .zip(&tmp)
        .for_each(|(dto, ep)| *dto = ep.into());
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_set_observer(
    handle: &mut TcpChannelsHandle,
    observer: *mut c_void,
) {
    let observer = Arc::new(FfiBootstrapServerObserver::new(observer));
    handle.0.set_observer(observer);
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_set_message_visitor(
    handle: &mut TcpChannelsHandle,
    visitor_factory: &RequestResponseVisitorFactoryHandle,
) {
    handle
        .0
        .set_message_visitor_factory(visitor_factory.0.clone())
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_create_tcp_server(
    handle: &TcpChannelsHandle,
    channel: &ChannelHandle,
    socket: &SocketHandle,
) -> *mut TcpServerHandle {
    let ChannelEnum::Tcp(channel_tcp) = channel.0.as_ref() else { panic!("not a tcp channel")};
    TcpServerHandle::new(
        handle
            .0
            .tcp_channels
            .lock()
            .unwrap()
            .tcp_server_factory
            .create_tcp_server(channel_tcp, socket.0.clone()),
    )
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_get_next_channel_id(handle: &TcpChannelsHandle) -> usize {
    handle.0.get_next_channel_id()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_process_message(
    handle: &TcpChannelsHandle,
    message: &MessageHandle,
    endpoint: &EndpointDto,
    node_id: *const u8,
    socket: &SocketHandle,
) {
    let node_id = PublicKey::from_ptr(node_id);
    handle
        .0
        .process_message(message.0.as_ref(), &endpoint.into(), node_id, &socket.0);
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_excluded_peers(
    handle: &TcpChannelsHandle,
) -> *mut PeerExclusionHandle {
    Box::into_raw(Box::new(PeerExclusionHandle(
        handle.0.excluded_peers.clone(),
    )))
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_prepare_handshake_response(
    handle: &TcpChannelsHandle,
    cookie: *const u8,
    v2: bool,
    response: &mut HandshakeResponseDto,
) {
    let query_payload = NodeIdHandshakeQuery {
        cookie: std::slice::from_raw_parts(cookie, 32).try_into().unwrap(),
    };
    let response_payload = handle.0.prepare_handshake_response(&query_payload, v2);
    *response = response_payload.into();
}

#[no_mangle]
pub unsafe extern "C" fn rsn_tcp_channels_verify_handshake_response(
    handle: &TcpChannelsHandle,
    response: &HandshakeResponseDto,
    endpoint: &EndpointDto,
) -> bool {
    let response = NodeIdHandshakeResponse::from(response);
    let endpoint = SocketAddr::from(endpoint);
    handle.0.verify_handshake_response(&response, &endpoint)
}

pub struct EndpointListHandle(Vec<SocketAddr>);

#[no_mangle]
pub unsafe extern "C" fn rsn_endpoint_list_len(handle: &EndpointListHandle) -> usize {
    handle.0.len()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_endpoint_list_get(
    handle: &EndpointListHandle,
    index: usize,
    result: &mut EndpointDto,
) {
    *result = handle.0.get(index).unwrap().into();
}

#[no_mangle]
pub unsafe extern "C" fn rsn_endpoint_list_destroy(handle: *mut EndpointListHandle) {
    drop(Box::from_raw(handle))
}

pub struct ChannelListHandle(Vec<Arc<ChannelEnum>>);

#[no_mangle]
pub unsafe extern "C" fn rsn_channel_list_len(handle: *mut ChannelListHandle) -> usize {
    (*handle).0.len()
}

#[no_mangle]
pub unsafe extern "C" fn rsn_channel_list_get(
    handle: *mut ChannelListHandle,
    index: usize,
) -> *mut ChannelHandle {
    ChannelHandle::new((*handle).0[index].clone())
}

#[no_mangle]
pub unsafe extern "C" fn rsn_channel_list_destroy(handle: *mut ChannelListHandle) {
    drop(Box::from_raw(handle))
}
