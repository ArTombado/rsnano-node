#include <nano/lib/blocks.hpp>
#include <nano/lib/logging.hpp>
#include <nano/node/active_transactions.hpp>
#include <nano/node/confirming_set.hpp>
#include <nano/node/election.hpp>
#include <nano/node/make_store.hpp>
#include <nano/secure/ledger.hpp>
#include <nano/test_common/system.hpp>
#include <nano/test_common/testutil.hpp>

#include <gtest/gtest.h>

using namespace std::chrono_literals;

TEST (confirmation_callback, observer_callbacks)
{
	nano::test::system system;
	nano::node_flags node_flags;
	nano::node_config node_config = system.default_config ();
	node_config.frontiers_confirmation = nano::frontiers_confirmation_mode::disabled;
	auto node = system.add_node (node_config, node_flags);

	auto wallet_id = node->wallets.first_wallet_id ();
	(void)node->wallets.insert_adhoc (wallet_id, nano::dev::genesis_key.prv);
	nano::block_hash latest (node->latest (nano::dev::genesis_key.pub));

	nano::keypair key1;
	nano::block_builder builder;
	auto send = builder
				.send ()
				.previous (latest)
				.destination (key1.pub)
				.balance (nano::dev::constants.genesis_amount - nano::Gxrb_ratio)
				.sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				.work (*system.work.generate (latest))
				.build ();
	auto send1 = builder
				 .send ()
				 .previous (send->hash ())
				 .destination (key1.pub)
				 .balance (nano::dev::constants.genesis_amount - nano::Gxrb_ratio * 2)
				 .sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				 .work (*system.work.generate (send->hash ()))
				 .build ();

	{
		auto transaction = node->store.tx_begin_write ();
		ASSERT_EQ (nano::block_status::progress, node->ledger.process (*transaction, send));
		ASSERT_EQ (nano::block_status::progress, node->ledger.process (*transaction, send1));
	}

	node->confirming_set.add (send1->hash ());

	// Callback is performed for all blocks that are confirmed
	ASSERT_TIMELY_EQ (5s, 2, node->stats->count (nano::stat::type::confirmation_observer, nano::stat::detail::all, nano::stat::dir::out));

	ASSERT_EQ (2, node->stats->count (nano::stat::type::confirmation_height, nano::stat::detail::blocks_confirmed, nano::stat::dir::in));
	ASSERT_EQ (3, node->ledger.cemented_count ());
	ASSERT_EQ (0, node->active.election_winner_details_size ());
}

// The callback and confirmation history should only be updated after confirmation height is set (and not just after voting)
TEST (confirmation_callback, confirmed_history)
{
	nano::test::system system;
	nano::node_flags node_flags;
	node_flags.set_force_use_write_queue (true);
	node_flags.disable_ascending_bootstrap ();
	nano::node_config node_config = system.default_config ();
	node_config.frontiers_confirmation = nano::frontiers_confirmation_mode::disabled;
	auto node = system.add_node (node_config, node_flags);

	nano::block_hash latest (node->latest (nano::dev::genesis_key.pub));

	nano::keypair key1;
	nano::block_builder builder;
	auto send = builder
				.send ()
				.previous (latest)
				.destination (key1.pub)
				.balance (nano::dev::constants.genesis_amount - nano::Gxrb_ratio)
				.sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				.work (*system.work.generate (latest))
				.build ();
	{
		auto transaction = node->store.tx_begin_write ();
		ASSERT_EQ (nano::block_status::progress, node->ledger.process (*transaction, send));
	}

	auto send1 = builder
				 .send ()
				 .previous (send->hash ())
				 .destination (key1.pub)
				 .balance (nano::dev::constants.genesis_amount - nano::Gxrb_ratio * 2)
				 .sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				 .work (*system.work.generate (send->hash ()))
				 .build ();

	node->process_active (send1);
	std::shared_ptr<nano::election> election;
	ASSERT_TIMELY (5s, election = nano::test::start_election (system, *node, send1->hash ()));
	{
		// The write guard prevents the confirmation height processor doing any writes
		auto write_guard = node->ledger.wait (nano::store::writer::testing);

		// Confirm send1
		node->active.force_confirm (*election);
		ASSERT_TIMELY_EQ (10s, node->active.size (), 0);
		ASSERT_EQ (0, node->active.recently_cemented.list ().size ());
		ASSERT_TRUE (node->active.empty ());

		auto transaction = node->store.tx_begin_read ();
		ASSERT_FALSE (node->ledger.block_confirmed (*transaction, send->hash ()));

		ASSERT_TIMELY (10s, node->ledger.queue_contains (nano::store::writer::confirmation_height));

		// Confirm that no inactive callbacks have been called when the confirmation height processor has already iterated over it, waiting to write
		ASSERT_EQ (0, node->stats->count (nano::stat::type::confirmation_observer, nano::stat::detail::inactive_conf_height, nano::stat::dir::out));
	}

	ASSERT_TIMELY (10s, !node->ledger.queue_contains (nano::store::writer::confirmation_height));

	auto transaction = node->store.tx_begin_read ();
	ASSERT_TRUE (node->ledger.block_confirmed (*transaction, send->hash ()));

	ASSERT_TIMELY_EQ (10s, node->active.size (), 0);
	ASSERT_TIMELY_EQ (10s, node->stats->count (nano::stat::type::confirmation_observer, nano::stat::detail::active_quorum, nano::stat::dir::out), 1);

	// Each block that's confirmed is in the recently_cemented history
	ASSERT_EQ (2, node->active.recently_cemented.list ().size ());
	ASSERT_TRUE (node->active.empty ());

	// Confirm the callback is not called under this circumstance
	ASSERT_EQ (1, node->stats->count (nano::stat::type::confirmation_observer, nano::stat::detail::active_quorum, nano::stat::dir::out));
	ASSERT_EQ (1, node->stats->count (nano::stat::type::confirmation_observer, nano::stat::detail::inactive_conf_height, nano::stat::dir::out));
	ASSERT_EQ (2, node->stats->count (nano::stat::type::confirmation_height, nano::stat::detail::blocks_confirmed, nano::stat::dir::in));
	ASSERT_EQ (3, node->ledger.cemented_count ());
	ASSERT_EQ (0, node->active.election_winner_details_size ());
}

TEST (confirmation_callback, dependent_election)
{
	nano::test::system system;
	nano::node_flags node_flags;
	node_flags.set_force_use_write_queue (true);
	nano::node_config node_config = system.default_config ();
	node_config.frontiers_confirmation = nano::frontiers_confirmation_mode::disabled;
	auto node = system.add_node (node_config, node_flags);

	nano::block_hash latest (node->latest (nano::dev::genesis_key.pub));

	nano::keypair key1;
	nano::block_builder builder;
	auto send = builder
				.send ()
				.previous (latest)
				.destination (key1.pub)
				.balance (nano::dev::constants.genesis_amount - nano::Gxrb_ratio)
				.sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				.work (*system.work.generate (latest))
				.build ();
	auto send1 = builder
				 .send ()
				 .previous (send->hash ())
				 .destination (key1.pub)
				 .balance (nano::dev::constants.genesis_amount - nano::Gxrb_ratio * 2)
				 .sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				 .work (*system.work.generate (send->hash ()))
				 .build ();
	auto send2 = builder
				 .send ()
				 .previous (send1->hash ())
				 .destination (key1.pub)
				 .balance (nano::dev::constants.genesis_amount - nano::Gxrb_ratio * 3)
				 .sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				 .work (*system.work.generate (send1->hash ()))
				 .build ();
	{
		auto transaction = node->store.tx_begin_write ();
		ASSERT_EQ (nano::block_status::progress, node->ledger.process (*transaction, send));
		ASSERT_EQ (nano::block_status::progress, node->ledger.process (*transaction, send1));
		ASSERT_EQ (nano::block_status::progress, node->ledger.process (*transaction, send2));
	}

	// This election should be confirmed as active_conf_height
	ASSERT_TRUE (nano::test::start_election (system, *node, send1->hash ()));
	// Start an election and confirm it
	auto election = nano::test::start_election (system, *node, send2->hash ());
	ASSERT_NE (nullptr, election);
	node->active.force_confirm (*election);

	// Wait for blocks to be confirmed in ledger, callbacks will happen after
	ASSERT_TIMELY_EQ (5s, 3, node->stats->count (nano::stat::type::confirmation_height, nano::stat::detail::blocks_confirmed, nano::stat::dir::in));
	// Once the item added to the confirming set no longer exists, callbacks have completed
	ASSERT_TIMELY (5s, !node->confirming_set.exists (send2->hash ()));

	ASSERT_EQ (1, node->stats->count (nano::stat::type::confirmation_observer, nano::stat::detail::active_quorum, nano::stat::dir::out)); // send2
	ASSERT_EQ (1, node->stats->count (nano::stat::type::confirmation_observer, nano::stat::detail::active_conf_height, nano::stat::dir::out)); // send1
	ASSERT_EQ (1, node->stats->count (nano::stat::type::confirmation_observer, nano::stat::detail::inactive_conf_height, nano::stat::dir::out)); // send
	ASSERT_EQ (4, node->ledger.cemented_count ());

	ASSERT_EQ (0, node->active.election_winner_details_size ());
}

TEST (confirmation_callback, election_winner_details_clearing_node_process_confirmed)
{
	// Make sure election_winner_details is also cleared if the block never enters the confirmation height processor from node::process_confirmed
	nano::test::system system (1);
	auto node = system.nodes.front ();

	nano::block_builder builder;
	auto send = builder
				.send ()
				.previous (nano::dev::genesis->hash ())
				.destination (nano::dev::genesis_key.pub)
				.balance (nano::dev::constants.genesis_amount - nano::Gxrb_ratio)
				.sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				.work (*system.work.generate (nano::dev::genesis->hash ()))
				.build ();
	// Add to election_winner_details. Use an unrealistic iteration so that it should fall into the else case and do a cleanup
	node->active.add_election_winner_details (send->hash (),
	std::make_shared<nano::election> (
	*node, send,
	[] (std::shared_ptr<nano::block> const &) {},
	[] (nano::account const &) {}, nano::election_behavior::normal));
	nano::election_status election;
	election.set_winner (send);
	node->process_confirmed (election, 1000000);
	ASSERT_EQ (0, node->active.election_winner_details_size ());
}
