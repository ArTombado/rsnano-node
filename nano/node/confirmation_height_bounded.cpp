#include "nano/lib/blocks.hpp"
#include "nano/lib/numbers.hpp"
#include "nano/lib/rsnano.hpp"
#include "nano/lib/rsnanoutils.hpp"

#include <nano/lib/logger_mt.hpp>
#include <nano/lib/stats.hpp>
#include <nano/node/confirmation_height_bounded.hpp>
#include <nano/node/logging.hpp>
#include <nano/node/write_database_queue.hpp>
#include <nano/secure/ledger.hpp>

#include <boost/format.hpp>

#include <iterator>
#include <numeric>

nano::hash_circular_buffer::hash_circular_buffer (size_t max_items) :
	handle{ rsnano::rsn_hash_circular_buffer_create (max_items) }
{
}

nano::hash_circular_buffer::~hash_circular_buffer ()
{
	rsnano::rsn_hash_circular_buffer_destroy (handle);
}

bool nano::hash_circular_buffer::empty () const
{
	return rsnano::rsn_hash_circular_buffer_empty (handle);
}

nano::block_hash nano::hash_circular_buffer::back () const
{
	nano::block_hash result;
	rsnano::rsn_hash_circular_buffer_back (handle, result.bytes.data ());
	return result;
}

void nano::hash_circular_buffer::push_back (nano::block_hash const & hash)
{
	rsnano::rsn_hash_circular_buffer_push_back (handle, hash.bytes.data ());
}

void nano::hash_circular_buffer::truncate_after (nano::block_hash const & hash)
{
	rsnano::rsn_hash_circular_buffer_truncate_after (handle, hash.bytes.data ());
}

namespace
{
void notify_observers_callback_wrapper (void * context, rsnano::BlockVecHandle * blocks_handle)
{
	auto callback = static_cast<std::function<void (std::vector<std::shared_ptr<nano::block>> const &)> *> (context);
	rsnano::block_vec block_vec{ blocks_handle };
	auto blocks = block_vec.to_vector ();
	(*callback) (blocks);
}

void notify_observers_delete_context (void * context)
{
	auto callback = static_cast<std::function<void (std::vector<std::shared_ptr<nano::block>> const &)> *> (context);
	delete callback;
}

rsnano::ConfirmationHeightBoundedHandle * create_conf_height_bounded_handle (
nano::write_database_queue & write_database_queue_a,
std::function<void (std::vector<std::shared_ptr<nano::block>> const &)> const & notify_observers_callback_a,
rsnano::AtomicU64Wrapper & batch_write_size_a,
std::shared_ptr<nano::logger_mt> & logger_a,
nano::logging const & logging_a,
nano::ledger & ledger_a)
{
	auto notify_observers_context = new std::function<void (std::vector<std::shared_ptr<nano::block>> const &)> (notify_observers_callback_a);
	auto logging_dto{ logging_a.to_dto () };
	return rsnano::rsn_confirmation_height_bounded_create (
	write_database_queue_a.handle,
	notify_observers_callback_wrapper,
	notify_observers_context,
	notify_observers_delete_context,
	batch_write_size_a.handle,
	nano::to_logger_handle (logger_a),
	&logging_dto,
	ledger_a.handle);
}
}

nano::confirmation_height_bounded::confirmation_height_bounded (nano::ledger & ledger_a, nano::write_database_queue & write_database_queue_a, std::chrono::milliseconds batch_separate_pending_min_time_a, nano::logging const & logging_a, std::shared_ptr<nano::logger_mt> & logger_a, std::atomic<bool> & stopped_a, rsnano::AtomicU64Wrapper & batch_write_size_a, std::function<void (std::vector<std::shared_ptr<nano::block>> const &)> const & notify_observers_callback_a, std::function<void (nano::block_hash const &)> const & notify_block_already_cemented_observers_callback_a, std::function<uint64_t ()> const & awaiting_processing_size_callback_a) :
	handle{ create_conf_height_bounded_handle (write_database_queue_a, notify_observers_callback_a, batch_write_size_a, logger_a, logging_a, ledger_a) },
	accounts_confirmed_info{ handle },
	pending_writes{ handle },
	ledger (ledger_a),
	write_database_queue (write_database_queue_a),
	batch_separate_pending_min_time (batch_separate_pending_min_time_a),
	logging (logging_a),
	logger (logger_a),
	stopped (stopped_a),
	batch_write_size (batch_write_size_a),
	notify_observers_callback (notify_observers_callback_a),
	notify_block_already_cemented_observers_callback (notify_block_already_cemented_observers_callback_a),
	awaiting_processing_size_callback (awaiting_processing_size_callback_a)
{
}

nano::confirmation_height_bounded::~confirmation_height_bounded ()
{
	rsnano::rsn_confirmation_height_bounded_destroy (handle);
}

// The next block hash to iterate over, the priority is as follows:
// 1 - The next block in the account chain for the last processed receive (if there is any)
// 2 - The next receive block which is closest to genesis
// 3 - The last checkpoint hit.
// 4 - The hash that was passed in originally. Either all checkpoints were exhausted (this can happen when there are many accounts to genesis)
//     or all other blocks have been processed.
nano::confirmation_height_bounded::top_and_next_hash nano::confirmation_height_bounded::get_next_block (
boost::optional<top_and_next_hash> const & next_in_receive_chain_a,
nano::hash_circular_buffer const & checkpoints_a,
boost::circular_buffer_space_optimized<receive_source_pair> const & receive_source_pairs,
boost::optional<receive_chain_details> & receive_details_a,
nano::block const & original_block)
{
	top_and_next_hash next;
	if (next_in_receive_chain_a.is_initialized ())
	{
		next = *next_in_receive_chain_a;
	}
	else if (!receive_source_pairs.empty ())
	{
		auto next_receive_source_pair = receive_source_pairs.back ();
		receive_details_a = next_receive_source_pair.receive_details;
		next = { next_receive_source_pair.source_hash, receive_details_a->next, receive_details_a->height + 1 };
	}
	else if (!checkpoints_a.empty ())
	{
		next = { checkpoints_a.back (), boost::none, 0 };
	}
	else
	{
		next = { original_block.hash (), boost::none, 0 };
	}

	return next;
}

void nano::confirmation_height_bounded::process (std::shared_ptr<nano::block> original_block)
{
	if (pending_empty ())
	{
		clear_process_vars ();
		timer.restart ();
	}

	boost::optional<top_and_next_hash> next_in_receive_chain;
	nano::hash_circular_buffer checkpoints{ max_items };
	boost::circular_buffer_space_optimized<receive_source_pair> receive_source_pairs{ max_items };
	nano::block_hash current;
	bool first_iter = true;
	auto transaction (ledger.store.tx_begin_read ());
	do
	{
		boost::optional<receive_chain_details> receive_details;
		auto hash_to_process = get_next_block (next_in_receive_chain, checkpoints, receive_source_pairs, receive_details, *original_block);
		current = hash_to_process.top;

		auto top_level_hash = current;
		std::shared_ptr<nano::block> block;
		if (first_iter)
		{
			debug_assert (current == original_block->hash ());
			block = original_block;
		}
		else
		{
			block = ledger.store.block ().get (*transaction, current);
		}

		if (!block)
		{
			if (ledger.pruning_enabled () && ledger.store.pruned ().exists (*transaction, current))
			{
				if (!receive_source_pairs.empty ())
				{
					receive_source_pairs.pop_back ();
				}
				continue;
			}
			else
			{
				auto error_str = (boost::format ("Ledger mismatch trying to set confirmation height for block %1% (bounded processor)") % current.to_string ()).str ();
				logger->always_log (error_str);
				std::cerr << error_str << std::endl;
				release_assert (block);
			}
		}
		nano::account account (block->account ());
		if (account.is_zero ())
		{
			account = block->sideband ().account ();
		}

		// Checks if we have encountered this account before but not commited changes yet, if so then update the cached confirmation height
		nano::confirmation_height_info confirmation_height_info;
		auto found_info = accounts_confirmed_info.find (account);
		if (found_info)
		{
			confirmation_height_info = nano::confirmation_height_info (found_info->confirmed_height, found_info->iterated_frontier);
		}
		else
		{
			ledger.store.confirmation_height ().get (*transaction, account, confirmation_height_info);
			// This block was added to the confirmation height processor but is already confirmed
			if (first_iter && confirmation_height_info.height () >= block->sideband ().height () && current == original_block->hash ())
			{
				notify_block_already_cemented_observers_callback (original_block->hash ());
			}
		}

		auto block_height = block->sideband ().height ();
		bool already_cemented = confirmation_height_info.height () >= block_height;

		// If we are not already at the bottom of the account chain (1 above cemented frontier) then find it
		if (!already_cemented && block_height - confirmation_height_info.height () > 1)
		{
			if (block_height - confirmation_height_info.height () == 2)
			{
				// If there is 1 uncemented block in-between this block and the cemented frontier,
				// we can just use the previous block to get the least unconfirmed hash.
				current = block->previous ();
				--block_height;
			}
			else if (!next_in_receive_chain.is_initialized ())
			{
				current = get_least_unconfirmed_hash_from_top_level (*transaction, current, account, confirmation_height_info, block_height);
			}
			else
			{
				// Use the cached successor of the last receive which saves having to do more IO in get_least_unconfirmed_hash_from_top_level
				// as we already know what the next block we should process should be.
				current = *hash_to_process.next;
				block_height = hash_to_process.next_height;
			}
		}

		auto top_most_non_receive_block_hash = current;

		bool hit_receive = false;
		if (!already_cemented)
		{
			hit_receive = iterate (*transaction, block_height, current, checkpoints, top_most_non_receive_block_hash, top_level_hash, receive_source_pairs, account);
		}

		// Exit early when the processor has been stopped, otherwise this function may take a
		// while (and hence keep the process running) if updating a long chain.
		if (stopped)
		{
			break;
		}

		// next_in_receive_chain can be modified when writing, so need to cache it here before resetting
		auto is_set = next_in_receive_chain.is_initialized ();
		next_in_receive_chain = boost::none;

		// Need to also handle the case where we are hitting receives where the sends below should be confirmed
		if (!hit_receive || (receive_source_pairs.size () == 1 && top_most_non_receive_block_hash != current))
		{
			preparation_data preparation_data{ *transaction, top_most_non_receive_block_hash, already_cemented, checkpoints, confirmation_height_info, account, block_height, current, receive_details, next_in_receive_chain };
			prepare_iterated_blocks_for_cementing (preparation_data);

			// If used the top level, don't pop off the receive source pair because it wasn't used
			if (!is_set && !receive_source_pairs.empty ())
			{
				receive_source_pairs.pop_back ();
			}

			auto total_pending_write_block_count = pending_writes.total_pending_write_block_count ();

			auto max_batch_write_size_reached = (total_pending_write_block_count >= batch_write_size.load ());
			// When there are a lot of pending confirmation height blocks, it is more efficient to
			// bulk some of them up to enable better write performance which becomes the bottleneck.
			auto min_time_exceeded = (timer.since_start () >= batch_separate_pending_min_time);
			auto finished_iterating = current == original_block->hash ();
			auto non_awaiting_processing = awaiting_processing_size_callback () == 0;
			auto should_output = finished_iterating && (non_awaiting_processing || min_time_exceeded);
			auto force_write = pending_writes.size () >= pending_writes_max_size || accounts_confirmed_info.size () >= pending_writes_max_size;

			if ((max_batch_write_size_reached || should_output || force_write) && !pending_writes.empty ())
			{
				// If nothing is currently using the database write lock then write the cemented pending blocks otherwise continue iterating
				if (write_database_queue.process (nano::writer::confirmation_height))
				{
					auto scoped_write_guard = write_database_queue.pop ();
					cement_blocks (scoped_write_guard);
				}
				else if (force_write)
				{
					auto scoped_write_guard = write_database_queue.wait (nano::writer::confirmation_height);
					cement_blocks (scoped_write_guard);
				}
			}
		}

		first_iter = false;
		transaction->refresh ();
	} while ((!receive_source_pairs.empty () || current != original_block->hash ()) && !stopped);

	debug_assert (checkpoints.empty ());
}

nano::block_hash nano::confirmation_height_bounded::get_least_unconfirmed_hash_from_top_level (nano::transaction const & transaction_a, nano::block_hash const & hash_a, nano::account const & account_a, nano::confirmation_height_info const & confirmation_height_info_a, uint64_t & block_height_a)
{
	nano::block_hash least_unconfirmed_hash = hash_a;
	if (confirmation_height_info_a.height () != 0)
	{
		if (block_height_a > confirmation_height_info_a.height ())
		{
			auto block (ledger.store.block ().get (transaction_a, confirmation_height_info_a.frontier ()));
			release_assert (block != nullptr);
			least_unconfirmed_hash = block->sideband ().successor ();
			block_height_a = block->sideband ().height () + 1;
		}
	}
	else
	{
		// No blocks have been confirmed, so the first block will be the open block
		auto info = ledger.account_info (transaction_a, account_a);
		release_assert (info);
		least_unconfirmed_hash = info->open_block ();
		block_height_a = 1;
	}
	return least_unconfirmed_hash;
}

bool nano::confirmation_height_bounded::iterate (
nano::read_transaction & transaction_a,
uint64_t bottom_height_a,
nano::block_hash const & bottom_hash_a,
nano::hash_circular_buffer & checkpoints_a,
nano::block_hash & top_most_non_receive_block_hash_a,
nano::block_hash const & top_level_hash_a,
boost::circular_buffer_space_optimized<receive_source_pair> & receive_source_pairs_a,
nano::account const & account_a)
{
	bool reached_target = false;
	bool hit_receive = false;
	auto hash = bottom_hash_a;
	uint64_t num_blocks = 0;
	while (!hash.is_zero () && !reached_target && !stopped)
	{
		// Keep iterating upwards until we either reach the desired block or the second receive.
		// Once a receive is cemented, we can cement all blocks above it until the next receive, so store those details for later.
		++num_blocks;
		auto block = ledger.store.block ().get (transaction_a, hash);
		auto source (block->source ());
		if (source.is_zero ())
		{
			source = block->link ().as_block_hash ();
		}

		if (!source.is_zero () && !ledger.is_epoch_link (source) && ledger.store.block ().exists (transaction_a, source))
		{
			hit_receive = true;
			reached_target = true;
			auto const & sideband (block->sideband ());
			auto next = !sideband.successor ().is_zero () && sideband.successor () != top_level_hash_a ? boost::optional<nano::block_hash> (sideband.successor ()) : boost::none;
			receive_source_pairs_a.push_back ({ receive_chain_details{ account_a, sideband.height (), hash, top_level_hash_a, next, bottom_height_a, bottom_hash_a }, source });
			// Store a checkpoint every max_items so that we can always traverse a long number of accounts to genesis
			if (receive_source_pairs_a.size () % max_items == 0)
			{
				checkpoints_a.push_back (top_level_hash_a);
			}
		}
		else
		{
			// Found a send/change/epoch block which isn't the desired top level
			top_most_non_receive_block_hash_a = hash;
			if (hash == top_level_hash_a)
			{
				reached_target = true;
			}
			else
			{
				hash = block->sideband ().successor ();
			}
		}

		// We could be traversing a very large account so we don't want to open read transactions for too long.
		if ((num_blocks > 0) && num_blocks % batch_read_size == 0)
		{
			transaction_a.refresh ();
		}
	}

	return hit_receive;
}

// Once the path to genesis has been iterated to, we can begin to cement the lowest blocks in the accounts. This sets up
// the non-receive blocks which have been iterated for an account, and the associated receive block.
void nano::confirmation_height_bounded::prepare_iterated_blocks_for_cementing (preparation_data & preparation_data_a)
{
	if (!preparation_data_a.already_cemented)
	{
		// Add the non-receive blocks iterated for this account
		auto block_height = (ledger.store.block ().account_height (preparation_data_a.transaction, preparation_data_a.top_most_non_receive_block_hash));
		if (block_height > preparation_data_a.confirmation_height_info.height ())
		{
			confirmed_info confirmed_info_l{ block_height, preparation_data_a.top_most_non_receive_block_hash };
			auto found_info{ accounts_confirmed_info.find (preparation_data_a.account) };
			if (found_info)
			{
				accounts_confirmed_info.insert (preparation_data_a.account, confirmed_info_l);
			}
			else
			{
				accounts_confirmed_info.insert (preparation_data_a.account, confirmed_info_l);
				rsnano::rsn_confirmation_height_bounded_accounts_confirmed_info_size_inc (handle);
			}

			preparation_data_a.checkpoints.truncate_after (preparation_data_a.top_most_non_receive_block_hash);

			nano::confirmation_height_bounded::write_details details{
				preparation_data_a.account,
				preparation_data_a.bottom_height,
				preparation_data_a.bottom_most,
				block_height,
				preparation_data_a.top_most_non_receive_block_hash
			};
			pending_writes.push_back (details);
			rsnano::rsn_confirmation_height_bounded_pending_writes_size_inc (handle);
		}
	}

	// Add the receive block and all non-receive blocks above that one
	auto & receive_details = preparation_data_a.receive_details;
	if (receive_details)
	{
		auto found_info{ accounts_confirmed_info.find (receive_details->account) };
		if (found_info)
		{
			nano::confirmation_height_bounded::confirmed_info receive_confirmed_info{ receive_details->height, receive_details->hash };
			accounts_confirmed_info.insert (receive_details->account, receive_confirmed_info);
		}
		else
		{
			nano::confirmation_height_bounded::confirmed_info receive_confirmed_info{ receive_details->height, receive_details->hash };
			accounts_confirmed_info.insert (receive_details->account, receive_confirmed_info);
			rsnano::rsn_confirmation_height_bounded_accounts_confirmed_info_size_inc (handle);
		}

		if (receive_details->next.is_initialized ())
		{
			preparation_data_a.next_in_receive_chain = top_and_next_hash{ receive_details->top_level, receive_details->next, receive_details->height + 1 };
		}
		else
		{
			preparation_data_a.checkpoints.truncate_after (receive_details->hash);
		}

		nano::confirmation_height_bounded::write_details details{
			receive_details->account,
			receive_details->bottom_height,
			receive_details->bottom_most,
			receive_details->height,
			receive_details->hash
		};
		pending_writes.push_back (details);
		rsnano::rsn_confirmation_height_bounded_pending_writes_size_inc (handle);
	}
}

void nano::confirmation_height_bounded::cement_blocks (nano::write_guard & scoped_write_guard_a)
{
	// Will contain all blocks that have been cemented (bounded by batch_write_size)
	// and will get run through the cemented observer callback
	rsnano::block_vec cemented_blocks;
	auto const maximum_batch_write_time = 250; // milliseconds
	auto const maximum_batch_write_time_increase_cutoff = maximum_batch_write_time - (maximum_batch_write_time / 5);
	auto const amount_to_change = batch_write_size.load () / 10; // 10%
	auto const minimum_batch_write_size = 16384u;
	rsnano::RsNanoTimer cemented_batch_timer;
	auto error = false;
	//------------------------------
	// todo: move code into this function:
	auto write_guard_handle = rsnano::rsn_confirmation_height_bounded_cement_blocks (
	handle,
	cemented_batch_timer.handle,
	cemented_blocks.handle,
	scoped_write_guard_a.handle,
	amount_to_change,
	&error);

	if (write_guard_handle != nullptr)
	{
		scoped_write_guard_a = nano::write_guard{ write_guard_handle };
	}
	//------------------------------
	auto time_spent_cementing = cemented_batch_timer.elapsed_ms ();

	// Scope guard could have been released earlier (0 cemented_blocks would indicate that)
	if (scoped_write_guard_a.is_owned () && !cemented_blocks.empty ())
	{
		scoped_write_guard_a.release ();
		auto block_vector{ cemented_blocks.to_vector () };
		notify_observers_callback (block_vector);
	}

	// Bail if there was an error. This indicates that there was a fatal issue with the ledger
	// (the blocks probably got rolled back when they shouldn't have).
	release_assert (!error);
	if (time_spent_cementing > maximum_batch_write_time)
	{
		// Reduce (unless we have hit a floor)
		batch_write_size.store (std::max<uint64_t> (minimum_batch_write_size, batch_write_size.load () - amount_to_change));
	}

	debug_assert (pending_writes.empty ());
	debug_assert (rsnano::rsn_confirmation_height_bounded_pending_writes_size (handle) == 0);
	timer.restart ();
}

bool nano::confirmation_height_bounded::pending_empty () const
{
	return pending_writes.empty ();
}

void nano::confirmation_height_bounded::clear_process_vars ()
{
	accounts_confirmed_info.clear ();
	rsnano::rsn_confirmation_height_bounded_accounts_confirmed_info_size_store (handle, 0);
}

nano::confirmation_height_bounded::receive_chain_details::receive_chain_details (nano::account const & account_a, uint64_t height_a, nano::block_hash const & hash_a, nano::block_hash const & top_level_a, boost::optional<nano::block_hash> next_a, uint64_t bottom_height_a, nano::block_hash const & bottom_most_a) :
	account (account_a),
	height (height_a),
	hash (hash_a),
	top_level (top_level_a),
	next (next_a),
	bottom_height (bottom_height_a),
	bottom_most (bottom_most_a)
{
}

nano::confirmation_height_bounded::write_details::write_details (nano::account const & account_a, uint64_t bottom_height_a, nano::block_hash const & bottom_hash_a, uint64_t top_height_a, nano::block_hash const & top_hash_a) :
	account (account_a),
	bottom_height (bottom_height_a),
	bottom_hash (bottom_hash_a),
	top_height (top_height_a),
	top_hash (top_hash_a)
{
}

nano::confirmation_height_bounded::write_details::write_details (rsnano::WriteDetailsDto const & dto) :
	bottom_height (dto.bottom_height),
	top_height (dto.top_height)
{
	std::copy (std::begin (dto.account), std::end (dto.account), std::begin (account.bytes));
	std::copy (std::begin (dto.bottom_hash), std::end (dto.bottom_hash), std::begin (bottom_hash.bytes));
	std::copy (std::begin (dto.top_hash), std::end (dto.top_hash), std::begin (top_hash.bytes));
}

rsnano::WriteDetailsDto nano::confirmation_height_bounded::write_details::to_dto () const
{
	rsnano::WriteDetailsDto dto;
	std::copy (std::begin (account.bytes), std::end (account.bytes), std::begin (dto.account));
	std::copy (std::begin (bottom_hash.bytes), std::end (bottom_hash.bytes), std::begin (dto.bottom_hash));
	std::copy (std::begin (top_hash.bytes), std::end (top_hash.bytes), std::begin (dto.top_hash));
	dto.bottom_height = bottom_height;
	dto.top_height = top_height;
	return dto;
}

nano::confirmation_height_bounded::receive_source_pair::receive_source_pair (confirmation_height_bounded::receive_chain_details const & receive_details_a, const block_hash & source_a) :
	receive_details (receive_details_a),
	source_hash (source_a)
{
}

nano::confirmation_height_bounded::confirmed_info::confirmed_info (uint64_t confirmed_height_a, nano::block_hash const & iterated_frontier_a) :
	confirmed_height (confirmed_height_a),
	iterated_frontier (iterated_frontier_a)
{
}

std::unique_ptr<nano::container_info_component> nano::collect_container_info (confirmation_height_bounded & confirmation_height_bounded, std::string const & name_a)
{
	auto composite = std::make_unique<container_info_composite> (name_a);
	composite->add_component (std::make_unique<container_info_leaf> (container_info{ "pending_writes", rsnano::rsn_confirmation_height_bounded_pending_writes_size (confirmation_height_bounded.handle), sizeof (nano::confirmation_height_bounded::write_details) }));
	composite->add_component (std::make_unique<container_info_leaf> (container_info{ "accounts_confirmed_info", rsnano::rsn_confirmation_height_bounded_accounts_confirmed_info_size (confirmation_height_bounded.handle), sizeof (nano::account) + sizeof (nano::confirmation_height_bounded::confirmed_info) }));
	return composite;
}

nano::confirmation_height_bounded::pending_writes_queue::pending_writes_queue (rsnano::ConfirmationHeightBoundedHandle * handle_a) :
	handle{ handle_a }
{
}

size_t nano::confirmation_height_bounded::pending_writes_queue::size () const
{
	return rsnano::rsn_pending_writes_queue_size (handle);
}

bool nano::confirmation_height_bounded::pending_writes_queue::empty () const
{
	return size () == 0;
}

void nano::confirmation_height_bounded::pending_writes_queue::push_back (nano::confirmation_height_bounded::write_details const & details)
{
	auto dto{ details.to_dto () };
	rsnano::rsn_pending_writes_queue_push_back (handle, &dto);
}

nano::confirmation_height_bounded::write_details nano::confirmation_height_bounded::pending_writes_queue::front () const
{
	rsnano::WriteDetailsDto details_dto;
	rsnano::rsn_pending_writes_queue_front (handle, &details_dto);
	return nano::confirmation_height_bounded::write_details{ details_dto };
}

void nano::confirmation_height_bounded::pending_writes_queue::pop_front ()
{
	rsnano::rsn_pending_writes_queue_pop_front (handle);
}

uint64_t nano::confirmation_height_bounded::pending_writes_queue::total_pending_write_block_count () const
{
	return rsnano::rsn_pending_writes_queue_total_pending_write_block_count (handle);
}