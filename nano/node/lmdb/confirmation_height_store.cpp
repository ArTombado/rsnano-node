#include <nano/node/lmdb/confirmation_height_store.hpp>
#include <nano/node/lmdb/lmdb.hpp>
#include <nano/secure/parallel_traversal.hpp>

namespace
{
nano::store_iterator<nano::account, nano::confirmation_height_info> to_iterator (rsnano::LmdbIteratorHandle * it_handle)
{
	if (it_handle == nullptr)
	{
		return nano::store_iterator<nano::account, nano::confirmation_height_info> (nullptr);
	}

	return nano::store_iterator<nano::account, nano::confirmation_height_info> (
	std::make_unique<nano::mdb_iterator<nano::account, nano::confirmation_height_info>> (it_handle));
}
}

nano::lmdb::confirmation_height_store::confirmation_height_store (nano::lmdb::store & store) :
	store{ store },
	handle{ rsnano::rsn_lmdb_confirmation_height_store_create (store.env ().handle) }
{
}

nano::lmdb::confirmation_height_store::~confirmation_height_store ()
{
	rsnano::rsn_lmdb_confirmation_height_store_destroy (handle);
}

void nano::lmdb::confirmation_height_store::put (nano::write_transaction const & transaction, nano::account const & account, nano::confirmation_height_info const & confirmation_height_info)
{
	rsnano::rsn_lmdb_confirmation_height_store_put (handle, transaction.get_rust_handle (), account.bytes.data (), &confirmation_height_info.dto);
}

bool nano::lmdb::confirmation_height_store::get (nano::transaction const & transaction, nano::account const & account, nano::confirmation_height_info & confirmation_height_info)
{
	bool success = rsnano::rsn_lmdb_confirmation_height_store_get (handle, transaction.get_rust_handle (), account.bytes.data (), &confirmation_height_info.dto);
	return !success;
}

bool nano::lmdb::confirmation_height_store::exists (nano::transaction const & transaction, nano::account const & account) const
{
	return rsnano::rsn_lmdb_confirmation_height_store_exists (handle, transaction.get_rust_handle (), account.bytes.data ());
}

void nano::lmdb::confirmation_height_store::del (nano::write_transaction const & transaction, nano::account const & account)
{
	rsnano::rsn_lmdb_confirmation_height_store_del (handle, transaction.get_rust_handle (), account.bytes.data ());
}

uint64_t nano::lmdb::confirmation_height_store::count (nano::transaction const & transaction_a)
{
	return rsnano::rsn_lmdb_confirmation_height_store_count (handle, transaction_a.get_rust_handle ());
}

void nano::lmdb::confirmation_height_store::clear (nano::write_transaction const & transaction_a, nano::account const & account_a)
{
	del (transaction_a, account_a);
}

void nano::lmdb::confirmation_height_store::clear (nano::write_transaction const & transaction_a)
{
	rsnano::rsn_lmdb_confirmation_height_store_clear (handle, transaction_a.get_rust_handle ());
}

nano::store_iterator<nano::account, nano::confirmation_height_info> nano::lmdb::confirmation_height_store::begin (nano::transaction const & transaction, nano::account const & account) const
{
	auto it_handle{ rsnano::rsn_lmdb_confirmation_height_store_begin_at_account (handle, transaction.get_rust_handle (), account.bytes.data ()) };
	return to_iterator (it_handle);
}

nano::store_iterator<nano::account, nano::confirmation_height_info> nano::lmdb::confirmation_height_store::begin (nano::transaction const & transaction) const
{
	auto it_handle{ rsnano::rsn_lmdb_confirmation_height_store_begin (handle, transaction.get_rust_handle ()) };
	return to_iterator (it_handle);
}

nano::store_iterator<nano::account, nano::confirmation_height_info> nano::lmdb::confirmation_height_store::end () const
{
	return nano::store_iterator<nano::account, nano::confirmation_height_info> (nullptr);
}

void nano::lmdb::confirmation_height_store::for_each_par (std::function<void (nano::read_transaction const &, nano::store_iterator<nano::account, nano::confirmation_height_info>, nano::store_iterator<nano::account, nano::confirmation_height_info>)> const & action_a) const
{
	parallel_traversal<nano::uint256_t> (
	[&action_a, this] (nano::uint256_t const & start, nano::uint256_t const & end, bool const is_last) {
		auto transaction (this->store.tx_begin_read ());
		action_a (*transaction, this->begin (*transaction, start), !is_last ? this->begin (*transaction, end) : this->end ());
	});
}

MDB_dbi nano::lmdb::confirmation_height_store::table_handle () const
{
	return rsnano::rsn_lmdb_confirmation_height_store_table_handle (handle);
}

void nano::lmdb::confirmation_height_store::set_table_handle (MDB_dbi handle_a)
{
	rsnano::rsn_lmdb_confirmation_height_store_set_table_handle (handle, handle_a);
}
