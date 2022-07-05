#pragma once

#include <nano/node/common.hpp>
#include <nano/node/socket.hpp>

#include <atomic>
#include <queue>

namespace rsnano
{
class BootstrapServerHandle;
}

namespace nano
{
class bootstrap_server;
class node_config;

namespace transport
{
	class tcp_channels;
}

class bootstrap_server_observer
{
public:
	virtual void bootstrap_server_timeout (std::uintptr_t inner_ptr) = 0;
	virtual void boostrap_server_exited (nano::socket::type_t type_a, std::uintptr_t inner_ptr, nano::tcp_endpoint const &) = 0;
	virtual std::size_t get_bootstrap_count () = 0;
	virtual void inc_bootstrap_count () = 0;
};

/**
 * Server side portion of bootstrap sessions. Listens for new socket connections and spawns bootstrap_server objects when connected.
 */
class bootstrap_listener final : public nano::bootstrap_server_observer
{
public:
	bootstrap_listener (uint16_t, nano::node &);
	void start ();
	void stop ();
	void accept_action (boost::system::error_code const &, std::shared_ptr<nano::socket> const &);
	std::size_t connection_count ();
	void erase_connection (std::uintptr_t conn_ptr);

	std::size_t get_bootstrap_count () override;
	void inc_bootstrap_count () override;
	void dec_bootstrap_count ();

	std::size_t get_realtime_count ();
	void inc_realtime_count ();
	void dec_realtime_count ();

	void bootstrap_server_timeout (std::uintptr_t inner_ptr) override;
	void boostrap_server_exited (nano::socket::type_t type_a, std::uintptr_t inner_ptr_a, nano::tcp_endpoint const & endpoint_a) override;

	nano::mutex mutex;
	std::unordered_map<std::uintptr_t, std::weak_ptr<nano::bootstrap_server>> connections;
	nano::tcp_endpoint endpoint ();
	nano::node & node;
	std::shared_ptr<nano::server_socket> listening_socket;
	bool on{ false };
	uint16_t port;

private:
	std::atomic<std::size_t> bootstrap_count{ 0 };
	std::atomic<std::size_t> realtime_count{ 0 };
};

std::unique_ptr<container_info_component> collect_container_info (bootstrap_listener & bootstrap_listener, std::string const & name);

class message;

class bootstrap_server_lock
{
public:
	bootstrap_server_lock (rsnano::BootstrapServerLockHandle * handle_a, rsnano::BootstrapServerHandle * server_a);
	bootstrap_server_lock (bootstrap_server_lock const &);
	bootstrap_server_lock (bootstrap_server_lock && other_a);
	~bootstrap_server_lock ();

	void unlock ();
	void lock ();

	rsnano::BootstrapServerLockHandle * handle;

private:
	rsnano::BootstrapServerHandle * server;
};

class locked_bootstrap_server_requests
{
public:
	locked_bootstrap_server_requests (nano::bootstrap_server_lock lock_a);
	locked_bootstrap_server_requests (nano::locked_bootstrap_server_requests &&);
	locked_bootstrap_server_requests (nano::locked_bootstrap_server_requests const &) = delete;
	nano::message * release_front_request ();

private:
	nano::bootstrap_server_lock lock;
};

class request_response_visitor_factory
{
public:
	request_response_visitor_factory (std::shared_ptr<nano::node> node_a);
	std::unique_ptr<nano::message_visitor> create_visitor (std::shared_ptr<nano::bootstrap_server> connection_a, nano::locked_bootstrap_server_requests & lock_a);

private:
	std::shared_ptr<nano::node> node;
};

/**
 * Owns the server side of a bootstrap connection. Responds to bootstrap messages sent over the socket.
 */
class bootstrap_server final : public std::enable_shared_from_this<nano::bootstrap_server>
{
public:
	bootstrap_server (std::shared_ptr<nano::socket> const &, std::shared_ptr<nano::node> const &);
	bootstrap_server (nano::bootstrap_server const &) = delete;
	~bootstrap_server ();
	nano::bootstrap_server_lock create_lock ();
	void stop ();
	void receive ();
	void receive_header_action (boost::system::error_code const &, std::size_t);
	void receive_bulk_pull_action (boost::system::error_code const &, std::size_t, nano::message_header const &);
	void receive_bulk_pull_account_action (boost::system::error_code const &, std::size_t, nano::message_header const &);
	void receive_frontier_req_action (boost::system::error_code const &, std::size_t, nano::message_header const &);
	void receive_keepalive_action (boost::system::error_code const &, std::size_t, nano::message_header const &);
	void receive_publish_action (boost::system::error_code const &, std::size_t, nano::message_header const &);
	void receive_confirm_req_action (boost::system::error_code const &, std::size_t, nano::message_header const &);
	void receive_confirm_ack_action (boost::system::error_code const &, std::size_t, nano::message_header const &);
	void receive_node_id_handshake_action (boost::system::error_code const &, std::size_t, nano::message_header const &);
	void receive_telemetry_ack_action (boost::system::error_code const & ec, std::size_t size_a, nano::message_header const & header_a);
	void add_request (std::unique_ptr<nano::message>);
	void finish_request ();
	void finish_request_async ();
	void timeout ();
	void push_request (std::unique_ptr<nano::message> msg);
	bool requests_empty ();
	//---------------------------------------------------------------
	// requests wrappers:
	bool is_request_queue_empty (nano::bootstrap_server_lock & lock_a);
	std::unique_ptr<nano::message> requests_front (nano::bootstrap_server_lock & lock_a);
	void requests_pop (nano::bootstrap_server_lock & lock_a);
	void push_request_locked (std::unique_ptr<nano::message> message_a, nano::bootstrap_server_lock & lock_a);
	//---------------------------------------------------------------

	bool make_bootstrap_connection ();
	bool is_realtime_connection ();
	bool is_stopped () const;
	std::uintptr_t inner_ptr () const;
	nano::account get_remote_node_id () const;
	void set_remote_node_id (nano::account account_a);
	nano::tcp_endpoint get_remote_endpoint () const;
	std::shared_ptr<nano::socket> const get_socket () const;

private:
	void run_next (nano::bootstrap_server_lock & lock_a);

	std::shared_ptr<std::vector<uint8_t>> receive_buffer;
	std::shared_ptr<nano::socket> const socket;
	std::shared_ptr<nano::network_filter> publish_filter;
	std::shared_ptr<nano::thread_pool> workers;
	boost::asio::io_context & io_ctx;
	std::shared_ptr<nano::request_response_visitor_factory> request_response_visitor_factory;

	// Remote enpoint used to remove response channel even after socket closing
	nano::tcp_endpoint remote_endpoint{ boost::asio::ip::address_v6::any (), 0 };
	nano::account remote_node_id{};
	std::chrono::steady_clock::time_point last_telemetry_req{ std::chrono::steady_clock::time_point () };
	std::shared_ptr<nano::bootstrap_server_observer> observer;
	std::shared_ptr<nano::logger_mt> logger;
	std::shared_ptr<nano::stat> stats;
	std::shared_ptr<nano::node_config> config;
	nano::network_params & network_params;
	bool disable_bootstrap_bulk_pull_server{ false };
	bool disable_tcp_realtime{ false };

public:
	rsnano::BootstrapServerHandle * handle;
};
}
