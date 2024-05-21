#include "nano/lib/rsnano.hpp"
#include "nano/lib/rsnanoutils.hpp"
#include "nano/node/transport/tcp.hpp"

#include <nano/node/node.hpp>
#include <nano/node/repcrawler.hpp>
#include <nano/secure/ledger.hpp>

#include <boost/format.hpp>

#include <chrono>
#include <memory>
#include <stdexcept>

nano::representative::representative (nano::account account_a, std::shared_ptr<nano::transport::channel> const & channel_a) :
	handle{ rsnano::rsn_representative_create (account_a.bytes.data (), channel_a->handle) }
{
}

nano::representative::representative (rsnano::RepresentativeHandle * handle_a) :
	handle{ handle_a }
{
}

nano::representative::representative (representative const & other_a) :
	handle{ rsnano::rsn_representative_clone (other_a.handle) }
{
}

nano::representative::~representative ()
{
	rsnano::rsn_representative_destroy (handle);
}

nano::representative & nano::representative::operator= (nano::representative const & other_a)
{
	rsnano::rsn_representative_destroy (handle);
	handle = rsnano::rsn_representative_clone (other_a.handle);
	return *this;
}

nano::account nano::representative::get_account () const
{
	nano::account account;
	rsnano::rsn_representative_account (handle, account.bytes.data ());
	return account;
}

std::shared_ptr<nano::transport::channel> nano::representative::get_channel () const
{
	return nano::transport::channel_handle_to_channel (rsnano::rsn_representative_channel (handle));
}

void nano::representative::set_channel (std::shared_ptr<nano::transport::channel> new_channel)
{
	rsnano::rsn_representative_set_channel (handle, new_channel->handle);
}

//------------------------------------------------------------------------------
// representative_register
//------------------------------------------------------------------------------

nano::representative_register::representative_register (rsnano::RepresentativeRegisterHandle * handle) :
	handle{ handle }
{
}

nano::representative_register::representative_register (nano::node & node_a)
{
	auto network_dto{ node_a.config->network_params.network.to_dto () };
	handle = rsnano::rsn_representative_register_create (
	node_a.ledger.handle,
	node_a.online_reps.get_handle (),
	node_a.stats->handle,
	&network_dto);
}

nano::representative_register::~representative_register ()
{
	rsnano::rsn_representative_register_destroy (handle);
}

nano::representative_register::insert_result nano::representative_register::update_or_insert (nano::account account_a, std::shared_ptr<nano::transport::channel> const & channel_a)
{
	rsnano::EndpointDto endpoint_dto;
	auto result_code = rsnano::rsn_representative_register_update_or_insert (handle, account_a.bytes.data (), channel_a->handle, &endpoint_dto);
	nano::representative_register::insert_result result{};
	if (result_code == 0)
	{
		result.inserted = true;
	}
	else if (result_code == 1)
	{
		// updated
	}
	else if (result_code == 2)
	{
		result.updated = true;
		result.prev_endpoint = rsnano::dto_to_endpoint (endpoint_dto);
	}
	else
	{
		throw std::runtime_error ("unknown result code");
	}
	return result;
}

bool nano::representative_register::is_pr (std::shared_ptr<nano::transport::channel> const & target_channel) const
{
	return rsnano::rsn_representative_register_is_pr (handle, target_channel->handle);
}

nano::uint128_t nano::representative_register::total_weight () const
{
	nano::amount result;
	rsnano::rsn_representative_register_total_weight (handle, result.bytes.data ());
	return result.number ();
}

std::vector<nano::representative> nano::representative_register::representatives (std::size_t count, nano::uint128_t const minimum_weight, std::optional<decltype (nano::network_constants::protocol_version)> const & minimum_protocol_version)
{
	uint8_t min_version = minimum_protocol_version.value_or (0);
	nano::amount weight{ minimum_weight };

	auto result_handle = rsnano::rsn_representative_register_representatives (handle, count, weight.bytes.data (), min_version);

	auto len = rsnano::rsn_representative_list_len (result_handle);
	std::vector<nano::representative> result;
	result.reserve (len);
	for (auto i = 0; i < len; ++i)
	{
		result.emplace_back (rsnano::rsn_representative_list_get (result_handle, i));
	}
	rsnano::rsn_representative_list_destroy (result_handle);
	return result;
}

/** Total number of representatives */
std::size_t nano::representative_register::representative_count ()
{
	return rsnano::rsn_representative_register_count (handle);
}

void nano::representative_register::cleanup_reps ()
{
	rsnano::rsn_representative_register_cleanup_reps (handle);
}

std::optional<std::chrono::milliseconds> nano::representative_register::last_request_elapsed (std::shared_ptr<nano::transport::channel> const & target_channel) const
{
	auto elapsed_ms = rsnano::rsn_representative_register_last_request_elapsed_ms (handle, target_channel->handle);
	if (elapsed_ms < 0)
	{
		return {};
	}
	else
	{
		return std::chrono::milliseconds (elapsed_ms);
	}
}

void nano::representative_register::on_rep_request (std::shared_ptr<nano::transport::channel> const & target_channel)
{
	rsnano::rsn_representative_register_on_rep_request (handle, target_channel->handle);
}
//
//------------------------------------------------------------------------------
// rep_crawler
//------------------------------------------------------------------------------

nano::rep_crawler::rep_crawler (nano::rep_crawler_config const & config_a, nano::node & node_a) :
	node (node_a)
{
	auto config_dto{ node_a.config->to_dto () };
	auto network_dto{ node_a.network_params.to_dto () };
	handle = rsnano::rsn_rep_crawler_create (
	node_a.representative_register.handle,
	node_a.stats->handle,
	config_a.query_timeout.count (),
	node_a.online_reps.get_handle (),
	&config_dto,
	&network_dto,
	node_a.network->tcp_channels->handle,
	node_a.async_rt.handle,
	node_a.ledger.handle,
	node_a.active.handle);
}

nano::rep_crawler::rep_crawler (rsnano::RepCrawlerHandle * handle, nano::node & node_a) :
	handle{ handle },
	node{ node_a }
{
}

nano::rep_crawler::~rep_crawler ()
{
	rsnano::rsn_rep_crawler_destroy (handle);
}

void nano::rep_crawler::start ()
{
	rsnano::rsn_rep_crawler_start (handle);
}

void nano::rep_crawler::stop ()
{
	rsnano::rsn_rep_crawler_stop (handle);
}

void nano::rep_crawler::query (std::shared_ptr<nano::transport::channel> const & target_channel)
{
	rsnano::rsn_rep_crawler_query (handle, target_channel->handle);
}

bool nano::rep_crawler::is_pr (std::shared_ptr<nano::transport::channel> const & channel) const
{
	return node.representative_register.is_pr (channel);
}

bool nano::rep_crawler::process (std::shared_ptr<nano::vote> const & vote, std::shared_ptr<nano::transport::channel> const & channel)
{
	return rsnano::rsn_rep_crawler_process (handle, vote->get_handle (), channel->handle);
}

nano::uint128_t nano::rep_crawler::total_weight () const
{
	return node.representative_register.total_weight ();
}

std::vector<nano::representative> nano::rep_crawler::representatives (std::size_t count, nano::uint128_t const minimum_weight, std::optional<decltype (nano::network_constants::protocol_version)> const & minimum_protocol_version)
{
	return node.representative_register.representatives (count, minimum_weight, minimum_protocol_version);
}

std::vector<nano::representative> nano::rep_crawler::principal_representatives (std::size_t count, std::optional<decltype (nano::network_constants::protocol_version)> const & minimum_protocol_version)
{
	return representatives (count, node.minimum_principal_weight (), minimum_protocol_version);
}

std::size_t nano::rep_crawler::representative_count ()
{
	return node.representative_register.representative_count ();
}

std::unique_ptr<nano::container_info_component> nano::rep_crawler::collect_container_info (const std::string & name)
{
	return std::make_unique<container_info_composite> (rsnano::rsn_rep_crawler_collect_container_info (handle, name.c_str ()));
}

// Only for tests
void nano::rep_crawler::force_add_rep (const nano::account & account, const std::shared_ptr<nano::transport::channel> & channel)
{
	release_assert (node.network_params.network.is_dev_network ());
	node.representative_register.update_or_insert (account, channel);
}

// Only for tests
void nano::rep_crawler::force_process (const std::shared_ptr<nano::vote> & vote, const std::shared_ptr<nano::transport::channel> & channel)
{
	rsnano::rsn_rep_crawler_force_process (handle, vote->get_handle (), channel->handle);
}

// Only for tests
void nano::rep_crawler::force_query (const nano::block_hash & hash, const std::shared_ptr<nano::transport::channel> & channel)
{
	rsnano::rsn_rep_crawler_force_query (handle, hash.bytes.data (), channel->handle);
}

/*
 * rep_crawler_config
 */

nano::rep_crawler_config::rep_crawler_config (std::chrono::milliseconds query_timeout_a) :
	query_timeout{ query_timeout_a }
{
}

nano::error nano::rep_crawler_config::deserialize (nano::tomlconfig & toml)
{
	auto query_timeout_l = query_timeout.count ();
	toml.get ("query_timeout", query_timeout_l);
	query_timeout = std::chrono::milliseconds{ query_timeout_l };

	return toml.get_error ();
}
