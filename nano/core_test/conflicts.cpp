#include <nano/lib/blocks.hpp>
#include <nano/node/active_elections.hpp>
#include <nano/node/election.hpp>
#include <nano/node/scheduler/component.hpp>
#include <nano/node/scheduler/priority.hpp>
#include <nano/secure/ledger.hpp>
#include <nano/test_common/chains.hpp>
#include <nano/test_common/system.hpp>
#include <nano/test_common/testutil.hpp>

#include <gtest/gtest.h>

#include <boost/variant/get.hpp>

using namespace std::chrono_literals;

TEST (conflicts, add_existing)
{
	nano::test::system system{ 1 };
	auto & node1 = *system.nodes[0];
	nano::keypair key1;

	// create a send block to send all of the nano supply to key1
	nano::block_builder builder;
	auto send1 = builder
				 .send ()
				 .previous (nano::dev::genesis->hash ())
				 .destination (key1.pub)
				 .balance (0)
				 .sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				 .work (0)
				 .build ();
	node1.work_generate_blocking (*send1);

	// add the block to ledger as an unconfirmed block
	ASSERT_EQ (nano::block_status::progress, node1.process (send1));

	// wait for send1 to be inserted in the ledger
	ASSERT_TIMELY (5s, node1.block (send1->hash ()));

	// instruct the election scheduler to trigger an election for send1
	nano::test::start_election (system, node1, send1->hash ());

	// wait for election to be started before processing send2
	ASSERT_TIMELY (5s, node1.active.active (*send1));

	nano::keypair key2;
	auto send2 = builder
				 .send ()
				 .previous (nano::dev::genesis->hash ())
				 .destination (key2.pub)
				 .balance (0)
				 .sign (nano::dev::genesis_key.prv, nano::dev::genesis_key.pub)
				 .work (0)
				 .build ();
	node1.work_generate_blocking (*send2);
	send2->sideband_set ({});

	// the block processor will notice that the block is a fork and it will try to publish it
	// which will update the election object
	node1.block_processor.add (send2);

	ASSERT_TRUE (node1.active.active (*send1));
	ASSERT_TIMELY (5s, node1.active.active (*send2));
}

TEST (conflicts, add_two)
{
	nano::test::system system{};
	auto const & node = system.add_node ();
	nano::keypair key1, key2, key3;
	auto gk = nano::dev::genesis_key;

	// create 2 new accounts, that receive 1 raw each, all blocks are force confirmed
	auto [send1, open1] = nano::test::setup_new_account (system, *node, 1, gk, key1, gk.pub, true);
	auto [send2, open2] = nano::test::setup_new_account (system, *node, 1, gk, key2, gk.pub, true);
	ASSERT_EQ (5, node->ledger.cemented_count ());

	// send 1 raw to account key3 from key1
	auto send_a = nano::state_block_builder ()
				  .account (key1.pub)
				  .previous (open1->hash ())
				  .representative (nano::dev::genesis_key.pub)
				  .balance (0)
				  .link (key3.pub)
				  .sign (key1.prv, key1.pub)
				  .work (*system.work.generate (open1->hash ()))
				  .build ();

	// send 1 raw to account key3 from key2
	auto send_b = nano::state_block_builder ()
				  .account (key2.pub)
				  .previous (open2->hash ())
				  .representative (nano::dev::genesis_key.pub)
				  .balance (0)
				  .link (key3.pub)
				  .sign (key2.prv, key2.pub)
				  .work (*system.work.generate (open2->hash ()))
				  .build ();

	// activate elections for the previous two send blocks (to account3) that we did not forcefully confirm
	ASSERT_TRUE (nano::test::process (*node, { send_a, send_b }));
	ASSERT_TRUE (nano::test::start_elections (system, *node, { send_a, send_b }));
	ASSERT_TRUE (node->active.election (send_a->qualified_root ()));
	ASSERT_TRUE (node->active.election (send_b->qualified_root ()));
	ASSERT_TIMELY_EQ (5s, node->active.size (), 2);
}
