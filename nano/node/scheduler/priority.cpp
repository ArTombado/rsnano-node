#include <nano/lib/rsnano.hpp>
#include <nano/node/node.hpp>
#include <nano/node/scheduler/priority.hpp>

nano::scheduler::priority::priority (nano::node & node_a, nano::stats & stats_a) :
	handle{ rsnano::rsn_election_scheduler_create (this) },
	node{ node_a },
	stats{ stats_a }
{
}

nano::scheduler::priority::~priority ()
{
	// Thread must be stopped before destruction
	debug_assert (!thread.joinable ());
	rsnano::rsn_election_scheduler_destroy (handle);
}

void nano::scheduler::priority::start ()
{
	debug_assert (!thread.joinable ());

	thread = std::thread{ [this] () {
		nano::thread_role::set (nano::thread_role::name::election_scheduler);
		run ();
	} };
}

void nano::scheduler::priority::stop ()
{
	{
		nano::lock_guard<nano::mutex> lock{ mutex };
		stopped = true;
	}
	notify ();
	nano::join_or_pass (thread);
}

void nano::scheduler::priority::manual (std::shared_ptr<nano::block> const & block_a, boost::optional<nano::uint128_t> const & previous_balance_a, nano::election_behavior election_behavior_a)
{
	nano::lock_guard<nano::mutex> lock{ mutex };
	manual_queue.push_back (std::make_tuple (block_a, previous_balance_a, election_behavior_a));
	notify ();
}

bool nano::scheduler::priority::activate (nano::account const & account_a, nano::transaction const & transaction)
{
	debug_assert (!account_a.is_zero ());
	auto info = node.ledger.account_info (transaction, account_a);
	if (info)
	{
		nano::confirmation_height_info conf_info;
		node.store.confirmation_height ().get (transaction, account_a, conf_info);
		if (conf_info.height () < info->block_count ())
		{
			debug_assert (conf_info.frontier () != info->head ());
			auto hash = conf_info.height () == 0 ? info->open_block () : node.store.block ().successor (transaction, conf_info.frontier ());
			auto block = node.store.block ().get (transaction, hash);
			debug_assert (block != nullptr);
			if (node.ledger.dependents_confirmed (transaction, *block))
			{
				stats.inc (nano::stat::type::election_scheduler, nano::stat::detail::activated);
				auto balance = node.ledger.balance (transaction, hash);
				auto previous_balance = node.ledger.balance (transaction, conf_info.frontier ());
				nano::lock_guard<nano::mutex> lock{ mutex };
				buckets.push (info->modified (), block, std::max (balance, previous_balance));
				notify ();
				return true; // Activated
			}
		}
	}
	return false; // Not activated
}

void nano::scheduler::priority::flush ()
{
	nano::unique_lock<nano::mutex> lock{ mutex };
	condition.wait (lock, [this] () {
		return stopped || empty_locked () || node.active.vacancy () <= 0;
	});
}

void nano::scheduler::priority::notify ()
{
	condition.notify_all ();
}

std::size_t nano::scheduler::priority::size () const
{
	nano::lock_guard<nano::mutex> lock{ mutex };
	return buckets.size () + manual_queue.size ();
}

bool nano::scheduler::priority::empty_locked () const
{
	return buckets.empty () && manual_queue.empty ();
}

bool nano::scheduler::priority::empty () const
{
	nano::lock_guard<nano::mutex> lock{ mutex };
	return empty_locked ();
}

std::size_t nano::scheduler::priority::priority_queue_size () const
{
	return buckets.size ();
}

bool nano::scheduler::priority::priority_queue_predicate () const
{
	return node.active.vacancy () > 0 && !buckets.empty ();
}

bool nano::scheduler::priority::manual_queue_predicate () const
{
	return !manual_queue.empty ();
}

void nano::scheduler::priority::run ()
{
	nano::unique_lock<nano::mutex> lock{ mutex };
	while (!stopped)
	{
		condition.wait (lock, [this] () {
			return stopped || priority_queue_predicate () || manual_queue_predicate ();
		});
		debug_assert ((std::this_thread::yield (), true)); // Introduce some random delay in debug builds
		if (!stopped)
		{
			stats.inc (nano::stat::type::election_scheduler, nano::stat::detail::loop);

			if (manual_queue_predicate ())
			{
				auto const [block, previous_balance, election_behavior] = manual_queue.front ();
				manual_queue.pop_front ();
				lock.unlock ();
				stats.inc (nano::stat::type::election_scheduler, nano::stat::detail::insert_manual);
				auto result = node.active.insert (block, election_behavior);
				if (result.election != nullptr)
				{
					result.election->transition_active ();
				}
			}
			else if (priority_queue_predicate ())
			{
				auto block = buckets.top ();
				buckets.pop ();
				lock.unlock ();
				stats.inc (nano::stat::type::election_scheduler, nano::stat::detail::insert_priority);
				auto result = node.active.insert (block);
				if (result.inserted)
				{
					stats.inc (nano::stat::type::election_scheduler, nano::stat::detail::insert_priority_success);
				}
				if (result.election != nullptr)
				{
					result.election->transition_active ();
				}
			}
			else
			{
				lock.unlock ();
			}
			notify ();
			lock.lock ();
		}
	}
}

std::unique_ptr<nano::container_info_component> nano::scheduler::priority::collect_container_info (std::string const & name)
{
	nano::unique_lock<nano::mutex> lock{ mutex };

	auto composite = std::make_unique<container_info_composite> (name);
	composite->add_component (std::make_unique<container_info_leaf> (container_info{ "manual_queue", manual_queue.size (), sizeof (decltype (manual_queue)::value_type) }));
	composite->add_component (buckets.collect_container_info ("buckets"));
	return composite;
}
