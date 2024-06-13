#include "nano/lib/rsnano.hpp"
#include "nano/lib/rsnanoutils.hpp"
#include "nano/node/messages.hpp"
#include "nano/node/transport/channel.hpp"
#include "nano/node/transport/socket.hpp"
#include "nano/node/transport/tcp_listener.hpp"
#include "nano/node/transport/tcp_server.hpp"
#include "nano/node/transport/traffic_type.hpp"
#include "nano/secure/network_filter.hpp"

#include <nano/crypto_lib/random_pool_shuffle.hpp>
#include <nano/lib/config.hpp>
#include <nano/lib/stats.hpp>
#include <nano/lib/utility.hpp>
#include <nano/node/node.hpp>
#include <nano/node/transport/fake.hpp>
#include <nano/node/transport/inproc.hpp>
#include <nano/node/transport/tcp.hpp>

#include <boost/format.hpp>

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <iterator>
#include <memory>
#include <stdexcept>
#include <unordered_set>

/*
 * tcp_message_manager
 */

nano::tcp_message_manager::tcp_message_manager (unsigned incoming_connections_max_a) :
	handle{ rsnano::rsn_tcp_message_manager_create (incoming_connections_max_a) }
{
}

nano::tcp_message_manager::tcp_message_manager (rsnano::TcpMessageManagerHandle * handle) :
	handle{ handle }
{
}

nano::tcp_message_manager::~tcp_message_manager ()
{
	rsnano::rsn_tcp_message_manager_destroy (handle);
}

/*
 * channel_tcp
 */

namespace
{
rsnano::ChannelHandle * create_tcp_channel_handle (
rsnano::async_runtime & async_rt_a,
nano::outbound_bandwidth_limiter & limiter_a,
nano::network_constants const & network_a,
std::shared_ptr<nano::transport::socket> const & socket_a,
nano::stats const & stats_a,
nano::transport::tcp_channels const & tcp_channels_a,
size_t channel_id)
{
	auto network_dto{ network_a.to_dto () };
	return rsnano::rsn_channel_tcp_create (
	socket_a->handle,
	stats_a.handle,
	tcp_channels_a.handle,
	limiter_a.handle,
	async_rt_a.handle,
	channel_id,
	&network_dto);
}

std::vector<std::shared_ptr<nano::transport::channel>> into_channel_vector (rsnano::ChannelListHandle * list_handle)
{
	auto len = rsnano::rsn_channel_list_len (list_handle);
	std::vector<std::shared_ptr<nano::transport::channel>> result;
	result.reserve (len);
	for (auto i = 0; i < len; ++i)
	{
		auto channel_handle = rsnano::rsn_channel_list_get (list_handle, i);
		result.push_back (std::make_shared<nano::transport::channel_tcp> (channel_handle));
	}
	rsnano::rsn_channel_list_destroy (list_handle);
	return result;
}
}

nano::transport::channel_tcp::channel_tcp (
rsnano::async_runtime & async_rt_a,
nano::outbound_bandwidth_limiter & limiter_a,
nano::network_constants const & network_a,
std::shared_ptr<nano::transport::socket> const & socket_a,
nano::stats const & stats_a,
nano::transport::tcp_channels const & tcp_channels_a,
size_t channel_id) :
	channel (create_tcp_channel_handle (
	async_rt_a,
	limiter_a,
	network_a,
	socket_a,
	stats_a,
	tcp_channels_a,
	channel_id))
{
}

uint8_t nano::transport::channel_tcp::get_network_version () const
{
	return rsnano::rsn_channel_tcp_network_version (handle);
}

nano::tcp_endpoint nano::transport::channel_tcp::get_tcp_remote_endpoint () const
{
	rsnano::EndpointDto ep_dto{};
	rsnano::rsn_channel_tcp_remote_endpoint (handle, &ep_dto);
	return rsnano::dto_to_endpoint (ep_dto);
}

nano::tcp_endpoint nano::transport::channel_tcp::get_local_endpoint () const
{
	rsnano::EndpointDto ep_dto{};
	rsnano::rsn_channel_tcp_local_endpoint (handle, &ep_dto);
	return rsnano::dto_to_endpoint (ep_dto);
}

void nano::transport::channel_tcp_send_callback (void * context_a, const rsnano::ErrorCodeDto * ec_a, std::size_t size_a)
{
	auto callback_ptr = static_cast<std::function<void (boost::system::error_code const &, std::size_t)> *> (context_a);
	if (*callback_ptr)
	{
		auto ec{ rsnano::dto_to_error_code (*ec_a) };
		(*callback_ptr) (ec, size_a);
	}
}

void nano::transport::delete_send_buffer_callback (void * context_a)
{
	auto callback_ptr = static_cast<std::function<void (boost::system::error_code const &, std::size_t)> *> (context_a);
	delete callback_ptr;
}

void nano::transport::channel_tcp::send (nano::message & message_a, std::function<void (boost::system::error_code const &, std::size_t)> const & callback_a, nano::transport::buffer_drop_policy drop_policy_a, nano::transport::traffic_type traffic_type)
{
	auto callback_pointer = new std::function<void (boost::system::error_code const &, std::size_t)> (callback_a);
	rsnano::rsn_channel_tcp_send (handle, message_a.handle, nano::transport::channel_tcp_send_callback, nano::transport::delete_send_buffer_callback, callback_pointer, static_cast<uint8_t> (drop_policy_a), static_cast<uint8_t> (traffic_type));
}

size_t nano::transport::channel_tcp::socket_id () const
{
	return rsnano::rsn_channel_tcp_socket_id (handle);
}

std::string nano::transport::channel_tcp::to_string () const
{
	return boost::str (boost::format ("%1%") % get_tcp_remote_endpoint ());
}

bool nano::transport::channel_tcp::alive () const
{
	return rsnano::rsn_channel_tcp_is_alive (handle);
}

/*
 * tcp_channels
 */

nano::transport::tcp_channels::tcp_channels (rsnano::TcpChannelsHandle * handle, rsnano::TcpMessageManagerHandle * mgr_handle, rsnano::NetworkFilterHandle * filter_handle) :
	handle{ handle },
	tcp_message_manager{ mgr_handle },
	publish_filter{ std::make_shared<nano::network_filter> (filter_handle) }
{
}

nano::transport::tcp_channels::~tcp_channels ()
{
	rsnano::rsn_tcp_channels_destroy (handle);
}

std::size_t nano::transport::tcp_channels::size () const
{
	return rsnano::rsn_tcp_channels_channel_count (handle);
}

float nano::transport::tcp_channels::size_sqrt () const
{
	return rsnano::rsn_tcp_channels_len_sqrt (handle);
}

// Simulating with sqrt_broadcast_simulate shows we only need to broadcast to sqrt(total_peers) random peers in order to successfully publish to everyone with high probability
std::size_t nano::transport::tcp_channels::fanout (float scale) const
{
	return rsnano::rsn_tcp_channels_fanout (handle, scale);
}

std::deque<std::shared_ptr<nano::transport::channel>> nano::transport::tcp_channels::list (std::size_t count_a, uint8_t minimum_version_a)
{
	auto list_handle = rsnano::rsn_tcp_channels_random_channels (handle, count_a, minimum_version_a);
	auto vec = into_channel_vector (list_handle);
	std::deque<std::shared_ptr<nano::transport::channel>> result;
	std::move (std::begin (vec), std::end (vec), std::back_inserter (result));
	return result;
}

std::deque<std::shared_ptr<nano::transport::channel>> nano::transport::tcp_channels::random_fanout (float scale)
{
	auto list_handle = rsnano::rsn_tcp_channels_random_fanout (handle, scale);
	auto vec = into_channel_vector (list_handle);
	std::deque<std::shared_ptr<nano::transport::channel>> result;
	std::move (std::begin (vec), std::end (vec), std::back_inserter (result));
	return result;
}

void nano::transport::tcp_channels::flood_message (nano::message & msg, float scale)
{
	rsnano::rsn_tcp_channels_flood_message (handle, msg.handle, scale);
}

std::shared_ptr<nano::transport::channel_tcp> nano::transport::tcp_channels::find_channel (nano::tcp_endpoint const & endpoint_a) const
{
	std::shared_ptr<nano::transport::channel_tcp> result;
	auto endpoint_dto{ rsnano::endpoint_to_dto (endpoint_a) };
	auto channel_handle = rsnano::rsn_tcp_channels_find_channel (handle, &endpoint_dto);
	if (channel_handle)
	{
		result = std::make_shared<nano::transport::channel_tcp> (channel_handle);
	}
	return result;
}

std::vector<std::shared_ptr<nano::transport::channel>> nano::transport::tcp_channels::random_channels (std::size_t count_a, uint8_t min_version) const
{
	auto list_handle = rsnano::rsn_tcp_channels_random_channels (handle, count_a, min_version);
	return into_channel_vector (list_handle);
}

void nano::transport::tcp_channels::random_fill (std::array<nano::endpoint, 8> & target_a) const
{
	std::array<rsnano::EndpointDto, 8> dtos;
	rsnano::rsn_tcp_channels_random_fill (handle, dtos.data ());
	auto j{ target_a.begin () };
	for (auto i{ dtos.begin () }, n{ dtos.end () }; i != n; ++i, ++j)
	{
		*j = rsnano::dto_to_udp_endpoint (*i);
	}
}

uint16_t nano::transport::tcp_channels::port () const
{
	return rsnano::rsn_tcp_channels_port (handle);
}

std::size_t nano::transport::tcp_channels::get_next_channel_id ()
{
	return rsnano::rsn_tcp_channels_get_next_channel_id (handle);
}

std::shared_ptr<nano::transport::channel_tcp> nano::transport::tcp_channels::find_node_id (nano::account const & node_id_a)
{
	std::shared_ptr<nano::transport::channel_tcp> result;
	auto channel_handle = rsnano::rsn_tcp_channels_find_node_id (handle, node_id_a.bytes.data ());
	if (channel_handle)
	{
		result = std::make_shared<nano::transport::channel_tcp> (channel_handle);
	}
	return result;
}

bool nano::transport::tcp_channels::not_a_peer (nano::endpoint const & endpoint_a, bool allow_local_peers)
{
	auto endpoint_dto{ rsnano::udp_endpoint_to_dto (endpoint_a) };
	return rsnano::rsn_tcp_channels_not_a_peer (handle, &endpoint_dto, allow_local_peers);
}

void nano::transport::tcp_channels::purge (std::chrono::system_clock::time_point const & cutoff_a)
{
	uint64_t cutoff_ns = std::chrono::duration_cast<std::chrono::nanoseconds> (cutoff_a.time_since_epoch ()).count ();
	rsnano::rsn_tcp_channels_purge (handle, cutoff_ns);
}

namespace
{
void message_received_callback (void * context, const rsnano::ErrorCodeDto * ec_dto, rsnano::MessageHandle * msg_handle)
{
	auto callback = static_cast<std::function<void (boost::system::error_code, std::unique_ptr<nano::message>)> *> (context);
	auto ec = rsnano::dto_to_error_code (*ec_dto);
	std::unique_ptr<nano::message> message;
	if (msg_handle != nullptr)
	{
		message = rsnano::message_handle_to_message (rsnano::rsn_message_clone (msg_handle));
	}
	(*callback) (ec, std::move (message));
}

void delete_callback_context (void * context)
{
	auto callback = static_cast<std::function<void (boost::system::error_code, std::unique_ptr<nano::message>)> *> (context);
	delete callback;
}
}
namespace
{
void delete_new_channel_callback (void * context)
{
	auto callback = static_cast<std::function<void (std::shared_ptr<nano::transport::channel>)> *> (context);
	delete callback;
}

void call_new_channel_callback (void * context, rsnano::ChannelHandle * channel_handle)
{
	auto callback = static_cast<std::function<void (std::shared_ptr<nano::transport::channel>)> *> (context);
	auto channel = std::make_shared<nano::transport::channel_tcp> (channel_handle);
	(*callback) (channel);
}
}

std::shared_ptr<nano::transport::channel> nano::transport::channel_handle_to_channel (rsnano::ChannelHandle * handle)
{
	auto channel_type = static_cast<nano::transport::transport_type> (rsnano::rsn_channel_type (handle));
	switch (channel_type)
	{
		case nano::transport::transport_type::tcp:
			return make_shared<nano::transport::channel_tcp> (handle);
		case nano::transport::transport_type::loopback:
			return make_shared<nano::transport::inproc::channel> (handle);
		case nano::transport::transport_type::fake:
			return make_shared<nano::transport::fake::channel> (handle);
		default:
			throw std::runtime_error ("unknown transport type");
	}
}
