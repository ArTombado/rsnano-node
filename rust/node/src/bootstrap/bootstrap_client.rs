use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use tokio::task::spawn_blocking;

use crate::{
    messages::Message,
    transport::{
        BufferDropPolicy, ChannelEnum, ChannelTcp, Socket, SocketExtensions, TrafficType,
        WriteCallback,
    },
    utils::{AsyncRuntime, ErrorCode},
};

use super::bootstrap_limits;

pub trait BootstrapClientObserver {
    fn bootstrap_client_closed(&self);
    fn to_weak(&self) -> Box<dyn BootstrapClientObserverWeakPtr>;
}

pub trait BootstrapClientObserverWeakPtr {
    fn upgrade(&self) -> Option<Arc<dyn BootstrapClientObserver>>;
}

pub struct BootstrapClient {
    async_rt: Arc<AsyncRuntime>,
    observer: Box<dyn BootstrapClientObserverWeakPtr>,
    channel: Arc<ChannelEnum>,
    socket: Arc<Socket>,
    receive_buffer: Arc<Mutex<Vec<u8>>>,
    block_count: AtomicU64,
    block_rate: AtomicU64,
    pending_stop: AtomicBool,
    hard_stop: AtomicBool,
    start_time: Mutex<Instant>,
}

impl BootstrapClient {
    pub fn new(
        async_rt: Arc<AsyncRuntime>,
        observer: Arc<dyn BootstrapClientObserver>,
        channel: Arc<ChannelEnum>,
        socket: Arc<Socket>,
    ) -> Self {
        if let ChannelEnum::Tcp(tcp) = channel.as_ref() {
            tcp.set_remote_endpoint();
        }
        Self {
            async_rt,
            observer: observer.to_weak(),
            channel,
            socket,
            receive_buffer: Arc::new(Mutex::new(vec![0; 256])),
            block_count: AtomicU64::new(0),
            block_rate: AtomicU64::new(0f64.to_bits()),
            pending_stop: AtomicBool::new(false),
            hard_stop: AtomicBool::new(false),
            start_time: Mutex::new(Instant::now()),
        }
    }

    pub fn sample_block_rate(&self) -> f64 {
        let elapsed = {
            let elapsed_seconds = self.elapsed().as_secs_f64();
            if elapsed_seconds > bootstrap_limits::BOOTSTRAP_MINIMUM_ELAPSED_SECONDS_BLOCKRATE {
                elapsed_seconds
            } else {
                bootstrap_limits::BOOTSTRAP_MINIMUM_ELAPSED_SECONDS_BLOCKRATE
            }
        };
        let new_block_rate = self.block_count.load(Ordering::SeqCst) as f64 / elapsed;
        self.block_rate
            .store((new_block_rate).to_bits(), Ordering::SeqCst);
        new_block_rate
    }

    pub fn elapsed(&self) -> Duration {
        self.start_time.lock().unwrap().elapsed()
    }

    pub fn set_start_time(&self) {
        let mut lock = self.start_time.lock().unwrap();
        *lock = Instant::now();
    }

    pub fn get_channel(&self) -> &Arc<ChannelEnum> {
        &self.channel
    }

    pub fn get_socket(&self) -> &Arc<Socket> {
        &self.socket
    }

    //TODO delete and use async read() directly
    pub fn read_async(&self, size: usize, callback: Box<dyn FnOnce(ErrorCode, usize) + Send>) {
        let socket = Arc::clone(&self.socket);
        let buffer = Arc::clone(&self.receive_buffer);
        self.async_rt.tokio.spawn(async move {
            let result = socket.read_raw(buffer, size).await;
            spawn_blocking(Box::new(move || match result {
                Ok(()) => callback(ErrorCode::new(), size),
                Err(_) => callback(ErrorCode::fault(), 0),
            }));
        });
    }

    pub async fn read(&self, size: usize) -> anyhow::Result<()> {
        self.socket
            .read_raw(Arc::clone(&self.receive_buffer), size)
            .await
    }

    pub fn receive_buffer(&self) -> Vec<u8> {
        self.receive_buffer.lock().unwrap().clone()
    }

    pub fn receive_buffer_len(&self) -> usize {
        self.receive_buffer.lock().unwrap().len()
    }

    fn tcp_channel(&self) -> &ChannelTcp {
        match self.channel.as_ref() {
            ChannelEnum::Tcp(tcp) => tcp,
            _ => panic!("not a tcp channel!"),
        }
    }

    pub fn send_buffer(
        &self,
        buffer: &Arc<Vec<u8>>,
        callback: Option<WriteCallback>,
        policy: BufferDropPolicy,
        traffic_type: TrafficType,
    ) {
        self.tcp_channel()
            .send_buffer(buffer, callback, policy, traffic_type);
    }

    pub fn send(
        &self,
        message: &dyn Message,
        callback: Option<WriteCallback>,
        drop_policy: BufferDropPolicy,
        traffic_type: TrafficType,
    ) {
        self.tcp_channel()
            .send(message, callback, drop_policy, traffic_type);
    }

    pub fn inc_block_count(&self) -> u64 {
        self.block_count.fetch_add(1, Ordering::SeqCst)
    }

    pub fn block_count(&self) -> u64 {
        self.block_count.load(Ordering::SeqCst)
    }

    pub fn block_rate(&self) -> f64 {
        f64::from_bits(self.block_rate.load(Ordering::SeqCst))
    }

    pub fn pending_stop(&self) -> bool {
        self.pending_stop.load(Ordering::SeqCst)
    }

    pub fn hard_stop(&self) -> bool {
        self.hard_stop.load(Ordering::SeqCst)
    }

    pub fn stop(&self, force: bool) {
        self.pending_stop.store(true, Ordering::SeqCst);
        if force {
            self.hard_stop.store(true, Ordering::SeqCst);
        }
    }

    pub fn close_socket(&self) {
        self.socket.close();
    }

    pub fn set_timeout(&self, timeout: Duration) {
        self.socket.set_timeout(timeout);
    }

    pub fn remote_endpoint(&self) -> SocketAddr {
        self.socket
            .get_remote()
            .unwrap_or_else(|| SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0))
    }

    pub fn channel_string(&self) -> String {
        self.tcp_channel().to_string()
    }

    pub fn tcp_endpoint(&self) -> SocketAddr {
        self.tcp_channel().remote_endpoint()
    }
}

impl Drop for BootstrapClient {
    fn drop(&mut self) {
        if let Some(observer) = self.observer.upgrade() {
            observer.bootstrap_client_closed();
        }
    }
}
