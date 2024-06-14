#pragma once

#include <nano/node/bootstrap/bootstrap_attempt.hpp>

#include <boost/multi_index_container.hpp>

namespace nano
{
class node;

/**
 * Lazy bootstrap session. Started with a block hash, this will "trace down" the blocks obtained to find a connection to the ledger.
 * This attempts to quickly bootstrap a section of the ledger given a hash that's known to be confirmed.
 */
class bootstrap_attempt_lazy final : public bootstrap_attempt
{
public:
	explicit bootstrap_attempt_lazy (std::shared_ptr<nano::node> const & node_a, uint64_t incremental_id_a, std::string const & id_a = "");
	explicit bootstrap_attempt_lazy (rsnano::BootstrapAttemptHandle * handle);
	void get_information (boost::property_tree::ptree &) override;
};

/**
 * Wallet bootstrap session. This session will trace down accounts within local wallets to try and bootstrap those blocks first.
 */
class bootstrap_attempt_wallet final : public bootstrap_attempt
{
public:
	explicit bootstrap_attempt_wallet (rsnano::BootstrapAttemptHandle * handle);
	explicit bootstrap_attempt_wallet (std::shared_ptr<nano::node> const & node_a, uint64_t incremental_id_a, std::string id_a = "");
	void run () override;
	std::size_t wallet_size ();
	void get_information (boost::property_tree::ptree &) override;
};
}
