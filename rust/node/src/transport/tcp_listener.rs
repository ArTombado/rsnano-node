use super::{
    CompositeSocketObserver, EndpointType, ServerSocket, ServerSocketExtensions, Socket,
    SocketBuilder, SocketExtensions, SocketObserver, SynCookies, TcpChannels, TcpServer,
    TcpServerExt, TcpServerObserver, TcpSocketFacadeFactory, TokioSocketFacade,
    TokioSocketFacadeFactory,
};
use crate::{
    block_processing::BlockProcessor,
    bootstrap::{BootstrapInitiator, BootstrapMessageVisitorFactory},
    config::{NodeConfig, NodeFlags},
    stats::{DetailType, Direction, SocketStats, StatType, Stats},
    utils::{first_ipv6_subnet_address, AsyncRuntime, ErrorCode, ThreadPool},
    NetworkParams,
};
use rsnano_core::{utils::Logger, KeyPair};
use rsnano_ledger::Ledger;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6},
    sync::{
        atomic::{AtomicU16, AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};

pub struct TcpListener {
    port: AtomicU16,
    max_inbound_connections: usize,
    config: NodeConfig,
    logger: Arc<dyn Logger>,
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
}

struct TcpListenerData {
    connections: HashMap<usize, Weak<TcpServer>>,
    on: bool,
    listening_socket: Option<Arc<ServerSocket>>, // TODO remove arc
}

impl TcpListener {
    pub fn new(
        port: u16,
        max_inbound_connections: usize,
        config: NodeConfig,
        logger: Arc<dyn Logger>,
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
            max_inbound_connections,
            config,
            logger,
            tcp_channels: Arc::downgrade(&tcp_channels),
            syn_cookies,
            data: Mutex::new(TcpListenerData {
                connections: HashMap::new(),
                on: false,
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
        }
    }

    pub fn stop(&self) {
        let mut conns = HashMap::new();
        {
            let mut guard = self.data.lock().unwrap();
            guard.on = false;
            std::mem::swap(&mut conns, &mut guard.connections);

            if let Some(socket) = guard.listening_socket.take() {
                socket.close();
            }
        }
    }

    pub fn get_realtime_count(&self) -> usize {
        self.realtime_count.load(Ordering::SeqCst)
    }

    pub fn connection_count(&self) -> usize {
        let data = self.data.lock().unwrap();
        data.connections.len()
    }

    pub fn endpoint(&self) -> SocketAddrV6 {
        let guard = self.data.lock().unwrap();
        if guard.on && guard.listening_socket.is_some() {
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, self.port.load(Ordering::SeqCst), 0, 0)
        } else {
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)
        }
    }

    fn remove_connection(&self, connection_id: usize) {
        let mut data = self.data.lock().unwrap();
        data.connections.remove(&connection_id);
    }
}

pub trait TcpListenerExt {
    fn start(
        &self,
        callback: Box<dyn Fn(Arc<Socket>, ErrorCode) -> bool + Send + Sync>,
    ) -> anyhow::Result<()>;
    fn accept_action(&self, ec: ErrorCode, socket: Arc<Socket>);
}

impl TcpListenerExt for Arc<TcpListener> {
    fn start(
        &self,
        callback: Box<dyn Fn(Arc<Socket>, ErrorCode) -> bool + Send + Sync>,
    ) -> anyhow::Result<()> {
        let mut data = self.data.lock().unwrap();
        data.on = true;

        let socket_stats = Arc::new(SocketStats::new(
            Arc::clone(&self.stats),
            Arc::clone(&self.logger),
            self.config.logging.network_timeout_logging(),
        ));

        let socket = SocketBuilder::endpoint_type(
            EndpointType::Server,
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
            socket_facade: Arc::clone(&self.socket_facade),
            connections_per_address: Mutex::new(Default::default()),
            node_flags: self.node_flags.clone(),
            network_params: self.network_params.clone(),
            workers: Arc::clone(&self.workers),
            logger: Arc::clone(&self.logger),
            tcp_socket_facade_factory: Arc::clone(&self.tcp_socket_facade_factory),
            node_config: self.config.clone(),
            stats: Arc::clone(&self.stats),
            socket_observer: Arc::downgrade(&self.socket_observer),
            max_inbound_connections: self.max_inbound_connections,
            runtime: Arc::downgrade(&self.runtime),
        });

        let ec = self.socket_facade.open(&SocketAddr::new(
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            self.port.load(Ordering::SeqCst),
        ));
        let listening_port = self.socket_facade.listening_port();
        if ec.is_err() {
            self.logger.always_log(&format!(
                "Network: Error while binding for incoming TCP/bootstrap on port {}: {:?}",
                listening_port, ec
            ));
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
        let this_l = Arc::clone(self);
        let socket_clone = Arc::clone(&listening_socket);
        self.socket_facade.post(Box::new(move || {
            if !this_l.socket_facade.is_acceptor_open() {
                this_l.logger.always_log("Network: Acceptor is not open");
                return;
            }

            let socket_stats = Arc::new(SocketStats::new(
                Arc::clone(&this_l.stats),
                Arc::clone(&this_l.logger),
                this_l.config.logging.network_timeout_logging(),
            ));

            // Prepare new connection
            let new_connection = SocketBuilder::endpoint_type(
                EndpointType::Server,
                Arc::clone(&this_l.workers),
                Arc::downgrade(&this_l.runtime),
            )
            .default_timeout(Duration::from_secs(
                this_l.config.tcp_io_timeout_s as u64,
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

            let socket_clone = Arc::clone(&socket_clone);
            let connection_clone = Arc::clone(&new_connection);
            this_l.socket_facade.async_accept(
                &new_connection,
                Box::new(move |remote_endpoint, ec| {
                    let SocketAddr::V6(remote_endpoint) = remote_endpoint else {panic!("not a v6 address")};
                    let this_l = socket_clone;
                    connection_clone.set_remote(remote_endpoint);
                    this_l.evict_dead_connections();

                    if this_l.connections_per_address.lock().unwrap().count_connections() >= this_l.max_inbound_connections {
                        this_l.logger.try_log ("Network: max_inbound_connections reached, unable to open new connection");
                        this_l.stats.inc (StatType::Tcp, DetailType::TcpAcceptFailure, Direction::In);
                        this_l.on_connection_requeue_delayed (callback);
                        return;
                    }

                    if this_l.limit_reached_for_incoming_ip_connections (&connection_clone) {
                        let remote_ip_address = connection_clone.get_remote().unwrap().ip().clone();
                        let log_message = format!("Network: max connections per IP (max_peers_per_ip) was reached for {}, unable to open new connection", remote_ip_address);
                        this_l.logger.try_log(&log_message);
                        this_l.stats.inc (StatType::Tcp, DetailType::TcpMaxPerIp, Direction::In);
                        this_l.on_connection_requeue_delayed (callback);
                        return;
                    }

                    if this_l.limit_reached_for_incoming_subnetwork_connections (&connection_clone) {
                        let remote_ip_address = connection_clone.get_remote().unwrap().ip().clone();
                        let remote_subnet = first_ipv6_subnet_address(&remote_ip_address, this_l.network_params.network.max_peers_per_subnetwork as u8);
                        let log_message = format!("Network: max connections per subnetwork (max_peers_per_subnetwork) was reached for subnetwork {} (remote IP: {}), unable to open new connection",
                            remote_subnet, remote_ip_address);
                        this_l.logger.try_log(&log_message);
                        this_l.stats.inc(StatType::Tcp, DetailType::TcpMaxPerSubnetwork, Direction::In);
                        this_l.on_connection_requeue_delayed (callback);
                        return;
                    }
                   			if ec.is_ok() {
                    				// Make sure the new connection doesn't idle. Note that in most cases, the callback is going to start
                    				// an IO operation immediately, which will start a timer.
                    				connection_clone.start ();
                    				connection_clone.set_timeout (Duration::from_secs(this_l.network_params.network.idle_timeout_s as u64));
                    				this_l.stats.inc (StatType::Tcp, DetailType::TcpAcceptSuccess, Direction::In);
                                    this_l.connections_per_address.lock().unwrap().insert(&connection_clone);
                                    if let Some(observer) = this_l.socket_observer.upgrade(){
                    				    observer.socket_accepted(Arc::clone(&connection_clone));
                                    }
                    				if callback (connection_clone, ec)
                    				{
                    					this_l.on_connection (callback);
                    					return;
                    				}
                    				this_l.logger.always_log ("Network: Stopping to accept connections");
                    				return;
                   			}

                    			// accept error
                    			this_l.logger.try_log (&format!("Network: Unable to accept connection: {:?}", ec));
                    			this_l.stats.inc (StatType::Tcp, DetailType::TcpAcceptFailure, Direction::In);

                    			if is_temporary_error (ec)
                    			{
                    				// if it is a temporary error, just retry it
                    				this_l.on_connection_requeue_delayed (callback);
                    				return;
                    			}

                    			// if it is not a temporary error, check how the listener wants to handle this error
                    			if callback(connection_clone, ec)
                    			{
                    				this_l.on_connection_requeue_delayed (callback);
                    				return;
                    			}

                    			// No requeue if we reach here, no incoming socket connections will be handled
                    			this_l.logger.always_log ("Network: Stopping to accept connections");
                }),
            );
        }));

        data.listening_socket = Some(listening_socket);
        Ok(())
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
                Arc::clone(&self.logger),
                Arc::clone(&self.syn_cookies),
                Arc::clone(&self.stats),
                self.network_params.network.clone(),
                Arc::clone(&self.node_id),
                Arc::clone(&self.ledger),
                Arc::clone(&self.workers),
                Arc::clone(&self.block_processor),
                Arc::clone(&self.bootstrap_initiator),
                self.node_flags.clone(),
                self.config.logging.clone(),
            ));
            let observer = Arc::clone(&self);
            let server = Arc::new(TcpServer::new(
                Arc::clone(&self.runtime),
                socket,
                Arc::new(self.config.clone()),
                Arc::clone(&self.logger),
                observer,
                Arc::clone(&tcp_channels.publish_filter),
                Arc::new(self.network_params.clone()),
                Arc::clone(&self.stats),
                Arc::clone(&tcp_channels.tcp_message_manager),
                message_visitor_factory,
                true,
            ));

            let mut data = self.data.lock().unwrap();
            data.connections
                .insert(server.unique_id(), Arc::downgrade(&server));
            server.start();
        } else {
            self.stats
                .inc(StatType::Tcp, DetailType::TcpExcluded, Direction::In);
            if self.config.logging.network_rejected_logging() {
                self.logger.try_log(&format!(
                    "Rejected connection from excluded peer {}",
                    remote
                ));
            }
        }
    }
}

impl TcpServerObserver for TcpListener {
    fn bootstrap_server_timeout(&self, connection_id: usize) {
        if self.config.logging.bulk_pull_logging() {
            self.logger
                .try_log("Closing incoming tcp / bootstrap server by timeout");
        }
        self.remove_connection(connection_id)
    }

    fn boostrap_server_exited(
        &self,
        socket_type: super::SocketType,
        connection_id: usize,
        endpoint: SocketAddrV6,
    ) {
        if self.config.logging.bulk_pull_logging() {
            self.logger.try_log("Exiting incoming TCP/bootstrap server");
        }
        if socket_type == super::SocketType::Bootstrap {
            self.bootstrap_count.fetch_sub(1, Ordering::SeqCst);
        } else if socket_type == super::SocketType::Realtime {
            self.realtime_count.fetch_sub(1, Ordering::SeqCst);
            // Clear temporary channel
            if let Some(tcp_channels) = self.tcp_channels.upgrade() {
                tcp_channels.erase_temporary_channel(&endpoint);
            }
        }
        self.remove_connection(connection_id);
    }

    fn get_bootstrap_count(&self) -> usize {
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
