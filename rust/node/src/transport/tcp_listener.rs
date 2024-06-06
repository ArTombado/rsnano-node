use super::{
    CompositeSocketObserver, ConnectionsPerAddress, Socket, SocketBuilder, SocketEndpoint,
    SocketExtensions, SocketObserver, SynCookies, TcpChannels, TcpConfig, TcpServer, TcpServerExt,
    TcpServerObserver, TcpSocketFacadeFactory, TokioSocketFacade, TokioSocketFacadeFactory,
};
use crate::{
    block_processing::BlockProcessor,
    bootstrap::{BootstrapInitiator, BootstrapMessageVisitorFactory},
    config::{NodeConfig, NodeFlags},
    stats::{DetailType, Direction, SocketStats, StatType, Stats},
    utils::{is_ipv4_mapped, AsyncRuntime, ErrorCode, ThreadPool},
    NetworkParams,
};
use rsnano_core::{
    utils::{ContainerInfo, ContainerInfoComponent},
    KeyPair,
};
use rsnano_ledger::Ledger;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6},
    ops::Deref,
    sync::{
        atomic::{AtomicU16, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, Weak,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tracing::{debug, error, warn};

/// Server side portion of tcp sessions. Listens for new socket connections and spawns tcp_server objects when connected.
pub struct TcpListener {
    port: AtomicU16,
    config: TcpConfig,
    node_config: NodeConfig,
    tcp_channels: Weak<TcpChannels>,
    syn_cookies: Arc<SynCookies>,
    stats: Arc<Stats>,
    runtime: Arc<AsyncRuntime>,
    socket_observer: Arc<dyn SocketObserver>,
    workers: Arc<dyn ThreadPool>,
    tcp_socket_facade_factory: Arc<dyn TcpSocketFacadeFactory>,
    network_params: NetworkParams,
    node_flags: NodeFlags,
    socket_facade: Arc<TokioSocketFacade>,
    data: Mutex<TcpListenerData>,
    ledger: Arc<Ledger>,
    block_processor: Arc<BlockProcessor>,
    bootstrap_initiator: Arc<BootstrapInitiator>,
    node_id: Arc<KeyPair>,
    bootstrap_count: AtomicUsize,
    realtime_count: AtomicUsize,
    cleanup_thread: Mutex<Option<JoinHandle<()>>>,
    condition: Condvar,
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        debug_assert!(self.cleanup_thread.lock().unwrap().is_none());
        debug_assert!(self.data.lock().unwrap().stopped);
        debug_assert_eq!(self.connection_count(), 0);
        debug_assert_eq!(self.attempt_count(), 0);
    }
}

struct TcpListenerData {
    connections: HashMap<usize, Weak<TcpServer>>,
    attempts: Vec<Attempt>,
    stopped: bool,
    listening_socket: Option<Arc<ServerSocket>>, // TODO remove arc
}

struct ServerSocket {
    socket: Arc<Socket>,
    connections_per_address: Mutex<ConnectionsPerAddress>,
}

impl TcpListenerData {
    fn evict_dead_connections(&self) {
        let Some(socket) = &self.listening_socket else {
            return;
        };

        socket
            .connections_per_address
            .lock()
            .unwrap()
            .evict_dead_connections();
    }
}

impl TcpListener {
    pub fn new(
        port: u16,
        config: TcpConfig,
        node_config: NodeConfig,
        tcp_channels: Arc<TcpChannels>,
        syn_cookies: Arc<SynCookies>,
        network_params: NetworkParams,
        node_flags: NodeFlags,
        runtime: Arc<AsyncRuntime>,
        socket_observer: Arc<dyn SocketObserver>,
        stats: Arc<Stats>,
        workers: Arc<dyn ThreadPool>,
        block_processor: Arc<BlockProcessor>,
        bootstrap_initiator: Arc<BootstrapInitiator>,
        ledger: Arc<Ledger>,
        node_id: Arc<KeyPair>,
    ) -> Self {
        let tcp_socket_facade_factory =
            Arc::new(TokioSocketFacadeFactory::new(Arc::clone(&runtime)));
        Self {
            port: AtomicU16::new(port),
            config,
            node_config,
            tcp_channels: Arc::downgrade(&tcp_channels),
            syn_cookies,
            data: Mutex::new(TcpListenerData {
                connections: HashMap::new(),
                attempts: Vec::new(),
                stopped: false,
                listening_socket: None,
            }),
            network_params,
            node_flags,
            runtime: Arc::clone(&runtime),
            socket_facade: Arc::new(TokioSocketFacade::create(runtime)),
            socket_observer,
            tcp_socket_facade_factory,
            stats,
            workers,
            block_processor,
            bootstrap_initiator,
            bootstrap_count: AtomicUsize::new(0),
            realtime_count: AtomicUsize::new(0),
            ledger,
            node_id,
            cleanup_thread: Mutex::new(None),
            condition: Condvar::new(),
        }
    }

    pub fn stop(&self) {
        // Close sockets
        let mut conns = HashMap::new();
        {
            let mut guard = self.data.lock().unwrap();
            guard.stopped = true;
            std::mem::swap(&mut conns, &mut guard.connections);

            if let Some(socket) = guard.listening_socket.take() {
                socket.socket.close_internal();
                self.socket_facade.close_acceptor();
                socket
                    .connections_per_address
                    .lock()
                    .unwrap()
                    .close_connections();
            }
        }
        self.condition.notify_all();

        if let Some(handle) = self.cleanup_thread.lock().unwrap().take() {
            handle.join().unwrap();
        }

        // Close attempts
        // TODO
    }

    /// Connects to the default peering port
    pub fn connect_ip(&self, remote: IpAddr) {
        self.connect(SocketAddr::new(
            remote,
            self.network_params.network.default_node_port,
        ));
    }

    pub fn connect(&self, remote: SocketAddr) {
        todo!()
    }

    pub fn realtime_count(&self) -> usize {
        self.realtime_count.load(Ordering::SeqCst)
    }

    pub fn connection_count(&self) -> usize {
        let mut data = self.data.lock().unwrap();
        self.cleanup(&mut data);
        data.connections.len()
    }

    pub fn attempt_count(&self) -> usize {
        self.data.lock().unwrap().attempts.len()
    }

    pub fn endpoint(&self) -> SocketAddrV6 {
        let guard = self.data.lock().unwrap();
        if !guard.stopped && guard.listening_socket.is_some() {
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, self.port.load(Ordering::SeqCst), 0, 0)
        } else {
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)
        }
    }

    fn remove_connection(&self, connection_id: usize) {
        let mut data = self.data.lock().unwrap();
        data.connections.remove(&connection_id);
    }

    fn limit_reached_for_incoming_subnetwork_connections(
        &self,
        new_connection: &Arc<Socket>,
    ) -> bool {
        let endpoint = new_connection
            .get_remote()
            .expect("new connection has no remote endpoint set");
        if self.node_flags.disable_max_peers_per_subnetwork || is_ipv4_mapped(&endpoint.ip()) {
            // If the limit is disabled, then it is unreachable.
            // If the address is IPv4 we don't check for a network limit, since its address space isn't big as IPv6 /64.
            return false;
        }

        let guard = self.data.lock().unwrap();
        let Some(socket) = &guard.listening_socket else {
            return false;
        };

        let counted_connections = socket
            .connections_per_address
            .lock()
            .unwrap()
            .count_subnetwork_connections(
                endpoint.ip(),
                self.network_params
                    .network
                    .ipv6_subnetwork_prefix_for_limiting,
            );

        counted_connections >= self.network_params.network.max_peers_per_subnetwork
    }

    fn limit_reached_for_incoming_ip_connections(&self, new_connection: &Arc<Socket>) -> bool {
        if self.node_flags.disable_max_peers_per_ip {
            return false;
        }

        let guard = self.data.lock().unwrap();
        let Some(socket) = &guard.listening_socket else {
            return false;
        };

        let ip = new_connection.get_remote().unwrap().ip().clone();
        let counted_connections = socket
            .connections_per_address
            .lock()
            .unwrap()
            .count_connections_for_ip(&ip);

        counted_connections >= self.network_params.network.max_peers_per_ip
    }

    fn run_cleanup(&self) {
        let mut guard = self.data.lock().unwrap();
        while !guard.stopped {
            self.stats.inc(StatType::TcpListener, DetailType::Cleanup);

            self.cleanup(&mut guard);
            self.timeout();

            guard = self
                .condition
                .wait_timeout_while(guard, Duration::from_secs(1), |g| !g.stopped)
                .unwrap()
                .0;
        }
    }

    fn cleanup(&self, data: &mut TcpListenerData) {
        // Erase dead connections
        data.connections.retain(|_, conn| {
            let retain = conn.strong_count() > 0;
            if !retain {
                self.stats.inc(StatType::TcpListener, DetailType::EraseDead);
                debug!("Evicting dead connection");
            }
            retain
        })

        // Erase completed attempts
        // TODO
    }

    fn timeout(&self) {
        // TODO
    }

    pub fn collect_container_info(&self, name: impl Into<String>) -> ContainerInfoComponent {
        ContainerInfoComponent::Composite(
            name.into(),
            vec![ContainerInfoComponent::Leaf(ContainerInfo {
                name: "connections".to_string(),
                count: self.connection_count(),
                sizeof_element: 1,
            })],
        )
    }
}

pub trait TcpListenerExt {
    /// If we are unable to accept a socket, for any reason, we wait just a little (1ms) before rescheduling the next connection accept.
    /// The intention is to throttle back the connection requests and break up any busy loops that could possibly form and
    /// give the rest of the system a chance to recover.
    fn on_connection_requeue_delayed(
        &self,
        callback: Box<dyn Fn(Arc<Socket>, ErrorCode) -> bool + Send + Sync>,
    );
    fn start(&self) -> anyhow::Result<()>;
    fn start_with(
        &self,
        callback: Box<dyn Fn(Arc<Socket>, ErrorCode) -> bool + Send + Sync>,
    ) -> anyhow::Result<()>;
    fn on_connection(&self, callback: Box<dyn Fn(Arc<Socket>, ErrorCode) -> bool + Send + Sync>);
    fn accept_action(&self, ec: ErrorCode, socket: Arc<Socket>);
    fn as_observer(self) -> Arc<dyn TcpServerObserver>;
}

impl TcpListenerExt for Arc<TcpListener> {
    fn on_connection_requeue_delayed(
        &self,
        callback: Box<dyn Fn(Arc<Socket>, ErrorCode) -> bool + Send + Sync>,
    ) {
        let this_w = Arc::downgrade(self);
        self.workers.add_delayed_task(
            Duration::from_millis(1),
            Box::new(move || {
                if let Some(this_l) = this_w.upgrade() {
                    this_l.on_connection(callback);
                }
            }),
        );
    }
    fn start(&self) -> anyhow::Result<()> {
        let self_w = Arc::downgrade(&self);
        self.start_with(Box::new(move |socket, ec| {
            if let Some(listener) = self_w.upgrade() {
                listener.accept_action(ec, socket);
                true
            } else {
                false
            }
        }))
    }

    fn start_with(
        &self,
        callback: Box<dyn Fn(Arc<Socket>, ErrorCode) -> bool + Send + Sync>,
    ) -> anyhow::Result<()> {
        error!("Starting TCP listener");
        let mut data = self.data.lock().unwrap();

        let socket_stats = Arc::new(SocketStats::new(Arc::clone(&self.stats)));

        let socket = SocketBuilder::endpoint_type(
            SocketEndpoint::Server,
            Arc::clone(&self.workers),
            Arc::downgrade(&self.runtime),
        )
        .default_timeout(Duration::MAX)
        .silent_connection_tolerance_time(Duration::from_secs(
            self.network_params
                .network
                .silent_connection_tolerance_time_s as u64,
        ))
        .idle_timeout(Duration::from_secs(
            self.network_params.network.idle_timeout_s as u64,
        ))
        .observer(Arc::new(CompositeSocketObserver::new(vec![
            socket_stats,
            Arc::clone(&self.socket_observer),
        ])))
        .build();

        let listening_socket = Arc::new(ServerSocket {
            socket,
            connections_per_address: Mutex::new(Default::default()),
        });

        let ec = self.socket_facade.open(&SocketAddr::new(
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            self.port.load(Ordering::SeqCst),
        ));
        let listening_port = self.socket_facade.listening_port();
        if ec.is_err() {
            error!(
                "Error while binding for incoming TCP/bootstrap on port {}: {:?}",
                listening_port, ec
            );
            bail!("Network: Error while binding for incoming TCP/bootstrap");
        }

        // the user can either specify a port value in the config or it can leave the choice up to the OS:
        // (1): port specified
        // (2): port not specified

        // (1) -- nothing to do
        //
        if self.port.load(Ordering::SeqCst) == listening_port {
        }
        // (2) -- OS port choice happened at TCP socket bind time, so propagate this port value back;
        // the propagation is done here for the `tcp_listener` itself, whereas for `network`, the node does it
        // after calling `tcp_listener.start ()`
        //
        else {
            self.port.store(listening_port, Ordering::SeqCst);
        }
        data.listening_socket = Some(listening_socket);
        drop(data);
        self.on_connection(callback);

        let self_w = Arc::downgrade(self);
        *self.cleanup_thread.lock().unwrap() = Some(
            std::thread::Builder::new()
                .name("TCP listener".to_owned())
                .spawn(move || {
                    if let Some(self_l) = self_w.upgrade() {
                        self_l.run_cleanup();
                    }
                })
                .unwrap(),
        );

        Ok(())
    }

    fn on_connection(&self, callback: Box<dyn Fn(Arc<Socket>, ErrorCode) -> bool + Send + Sync>) {
        let listening_socket = {
            let guard = self.data.lock().unwrap();
            let Some(s) = &guard.listening_socket else {
                return;
            };
            Arc::clone(s)
        };
        let this_w = Arc::downgrade(self);
        self.socket_facade.post(Box::new(move || {
            let Some(this_l) = this_w.upgrade() else {return;};
            if !this_l.socket_facade.is_acceptor_open() {
                error!("Socket acceptor is not open");
                return;
            }

            let socket_stats = Arc::new(SocketStats::new(
                Arc::clone(&this_l.stats),
            ));

            // Prepare new connection
            let new_connection = SocketBuilder::endpoint_type(
                SocketEndpoint::Server,
                Arc::clone(&this_l.workers),
                Arc::downgrade(&this_l.runtime),
            )
            .default_timeout(Duration::from_secs(
                this_l.node_config.tcp_io_timeout_s as u64,
            ))
            .idle_timeout(Duration::from_secs(
                this_l
                    .network_params
                    .network
                    .silent_connection_tolerance_time_s as u64,
            ))
            .silent_connection_tolerance_time(Duration::from_secs(
                this_l
                    .network_params
                    .network
                    .silent_connection_tolerance_time_s as u64,
            ))
            .observer(Arc::new(CompositeSocketObserver::new(vec![
                socket_stats,
                Arc::clone(&this_l.socket_observer),
            ])))
            .build();

            let listening_socket_clone = Arc::clone(&listening_socket);
            let connection_clone = Arc::clone(&new_connection);
            let this_clone_w = Arc::downgrade(&this_l);
            this_l.socket_facade.async_accept(
                &new_connection,
                Box::new(move |remote_endpoint, ec| {
                    let Some(this_clone) = this_clone_w.upgrade() else { return; };
                    let SocketAddr::V6(remote_endpoint) = remote_endpoint else {panic!("not a v6 address")};
                    let socket_l = listening_socket_clone;
                    connection_clone.set_remote(remote_endpoint);

                    let data = this_clone.data.lock().unwrap();
                    data.evict_dead_connections();
                    drop(data);

                    if socket_l.connections_per_address.lock().unwrap().count_connections() >= this_clone.config.max_inbound_connections {
                        this_clone.stats.inc_dir (StatType::TcpListener, DetailType::AcceptFailure, Direction::In);
                        debug!("Max_inbound_connections reached, unable to open new connection");

                        this_clone.on_connection_requeue_delayed (callback);
                        return;
                    }

                    if this_clone.limit_reached_for_incoming_ip_connections (&connection_clone) {
                        let remote_ip_address = connection_clone.get_remote().unwrap().ip().clone();
                        this_clone.stats.inc_dir (StatType::TcpListener, DetailType::MaxPerIp, Direction::In);
                        debug!("Max connections per IP reached (ip: {}), unable to open new connection", remote_ip_address);
                        this_clone.on_connection_requeue_delayed (callback);
                        return;
                    }

                    if this_clone.limit_reached_for_incoming_subnetwork_connections (&connection_clone) {
                        let remote_ip_address = connection_clone.get_remote().unwrap().ip().clone();
                        this_clone.stats.inc_dir(StatType::TcpListener, DetailType::MaxPerSubnetwork, Direction::In);
                        debug!("Max connections per subnetwork reached (ip: {}), unable to open new connection",
                            remote_ip_address);
                        this_clone.on_connection_requeue_delayed (callback);
                        return;
                    }
                   			if ec.is_ok() {
                    				// Make sure the new connection doesn't idle. Note that in most cases, the callback is going to start
                    				// an IO operation immediately, which will start a timer.
                    				connection_clone.start ();
                    				connection_clone.set_timeout (Duration::from_secs(this_clone.network_params.network.idle_timeout_s as u64));
                    				this_clone.stats.inc_dir (StatType::TcpListener, DetailType::AcceptSuccess, Direction::In);
                                    socket_l.connections_per_address.lock().unwrap().insert(&connection_clone);
                                    this_clone.socket_observer.socket_accepted(Arc::clone(&connection_clone));
                    				if callback (connection_clone, ec)
                    				{
                    					this_clone.on_connection (callback);
                    					return;
                    				}
                    				warn!("Stopping to accept connections");
                    				return;
                   			}

                    			// accept error
                    			this_clone.stats.inc_dir (StatType::TcpListener, DetailType::AcceptFailure, Direction::In);
                    			error!("Unable to accept connection: ({:?})", ec);

                    			if is_temporary_error (ec)
                    			{
                    				// if it is a temporary error, just retry it
                    				this_clone.on_connection_requeue_delayed (callback);
                    				return;
                    			}

                    			// if it is not a temporary error, check how the listener wants to handle this error
                    			if callback(connection_clone, ec)
                    			{
                    				this_clone.on_connection_requeue_delayed (callback);
                    				return;
                    			}

                    			// No requeue if we reach here, no incoming socket connections will be handled
                    			warn!("Stopping to accept connections");
                }),
            );
        }));
    }

    fn accept_action(&self, _ec: ErrorCode, socket: Arc<Socket>) {
        let Some(remote) = socket.get_remote() else {
            return;
        };
        let Some(tcp_channels) = self.tcp_channels.upgrade() else {
            return;
        };
        if !tcp_channels
            .excluded_peers
            .lock()
            .unwrap()
            .is_excluded(&remote)
        {
            let message_visitor_factory = Arc::new(BootstrapMessageVisitorFactory::new(
                Arc::clone(&self.runtime),
                Arc::clone(&self.syn_cookies),
                Arc::clone(&self.stats),
                self.network_params.network.clone(),
                Arc::clone(&self.node_id),
                Arc::clone(&self.ledger),
                Arc::clone(&self.workers),
                Arc::clone(&self.block_processor),
                Arc::clone(&self.bootstrap_initiator),
                self.node_flags.clone(),
            ));
            let observer = Arc::downgrade(&self);
            let server = Arc::new(TcpServer::new(
                Arc::clone(&self.runtime),
                socket,
                Arc::new(self.node_config.clone()),
                observer,
                Arc::clone(&tcp_channels.publish_filter),
                Arc::new(self.network_params.clone()),
                Arc::clone(&self.stats),
                Arc::clone(&tcp_channels.tcp_message_manager),
                message_visitor_factory,
                true,
                Arc::clone(&self.syn_cookies),
                self.node_id.deref().clone(),
            ));

            let mut data = self.data.lock().unwrap();
            data.connections
                .insert(server.unique_id(), Arc::downgrade(&server));
            server.start();
        } else {
            self.stats
                .inc_dir(StatType::TcpListener, DetailType::Excluded, Direction::In);
            debug!("Rejected connection from excluded peer {}", remote);
        }
    }

    fn as_observer(self) -> Arc<dyn TcpServerObserver> {
        self
    }
}

impl TcpServerObserver for TcpListener {
    fn bootstrap_server_timeout(&self, _connection_id: usize) {
        debug!("Closing TCP server due to timeout");
    }

    fn boostrap_server_exited(
        &self,
        socket_type: super::SocketType,
        _connection_id: usize,
        endpoint: SocketAddrV6,
    ) {
        debug!("Exiting server: {}", endpoint);
        if socket_type == super::SocketType::Bootstrap {
            self.bootstrap_count.fetch_sub(1, Ordering::SeqCst);
        } else if socket_type == super::SocketType::Realtime {
            self.realtime_count.fetch_sub(1, Ordering::SeqCst);
            // Clear temporary channel
            if let Some(tcp_channels) = self.tcp_channels.upgrade() {
                tcp_channels.erase_temporary_channel(&endpoint);
            }
        }
    }

    fn bootstrap_count(&self) -> usize {
        self.bootstrap_count.load(Ordering::SeqCst)
    }

    fn inc_bootstrap_count(&self) {
        self.bootstrap_count.fetch_add(1, Ordering::SeqCst);
    }

    fn dec_bootstrap_count(&self) {
        self.bootstrap_count.fetch_sub(1, Ordering::SeqCst);
    }

    fn inc_realtime_count(&self) {
        self.realtime_count.fetch_add(1, Ordering::SeqCst);
    }

    fn dec_realtime_count(&self) {
        self.realtime_count.fetch_sub(1, Ordering::SeqCst);
    }
}

fn is_temporary_error(ec: ErrorCode) -> bool {
    return ec.val == 11 // would block
                        || ec.val ==  4; // interrupted system call
}

struct Attempt {
    endpoint: SocketAddr,
    start: Instant,
}
