use std::{collections::HashMap, sync::Arc};
use rsnano_core::{Account, Amount, BlockHash};
use rsnano_node::node::Node;
use rsnano_rpc_messages::{AccountsReceivableArgs, ReceivableDto, SourceInfo};
use serde_json::to_string_pretty;
use itertools::Itertools;

pub async fn accounts_receivable(node: Arc<Node>, args: AccountsReceivableArgs) -> String {
    let transaction = node.store.tx_begin_read();
    let count = args.count;
    let threshold = args.threshold.unwrap_or(Amount::zero());
    let source = args.source.unwrap_or(false);
    let include_only_confirmed = args.include_only_confirmed.unwrap_or(true);
    let sorting = args.sorting.unwrap_or(false);
    let simple = threshold.is_zero() && !source && !sorting;

    let result = if simple {
        let mut blocks: HashMap<Account, Vec<BlockHash>> = HashMap::new();
        for account in args.accounts {
            let mut receivable_hashes = Vec::new();
            let mut iterator = node.ledger.any().account_receivable_upper_bound(&transaction, account, BlockHash::zero());
            while let Some((key, info)) = iterator.next() {
                if receivable_hashes.len() >= count as usize {
                    break;
                }
                if include_only_confirmed && !node.ledger.confirmed().block_exists_or_pruned(&transaction, &key.send_block_hash) {
                    continue;
                }
                receivable_hashes.push(key.send_block_hash);
            }
            if !receivable_hashes.is_empty() {
                blocks.insert(account, receivable_hashes);
            }
        }
        ReceivableDto::Blocks { blocks }
    } else if source {
        let mut blocks: HashMap<Account, HashMap<BlockHash, SourceInfo>> = HashMap::new();
        for account in args.accounts {
            let mut receivable_info = HashMap::new();
            for current in node.ledger.any().account_receivable_upper_bound(&transaction, account, BlockHash::zero()) {
                if receivable_info.len() >= count as usize {
                    break;
                }
                let (key, info) = current;
                if include_only_confirmed && !node.ledger.confirmed().block_exists_or_pruned(&transaction, &key.send_block_hash) {
                    continue;
                }
                if info.amount < threshold {
                    continue;
                }
                receivable_info.insert(key.send_block_hash, SourceInfo {
                    amount: info.amount,
                    source: info.source,
                });
            }
            if !receivable_info.is_empty() {
                blocks.insert(account, receivable_info);
            }
        }
        if sorting {
            for (_, receivable_info) in blocks.iter_mut() {
                *receivable_info = receivable_info.drain()
                    .sorted_by(|a, b| b.1.amount.cmp(&a.1.amount))
                    .collect();
            }
        }
        ReceivableDto::Source { blocks }
    } else {
        let mut blocks: HashMap<Account, HashMap<BlockHash, Amount>> = HashMap::new();
        for account in args.accounts {
            let mut receivable_amounts = HashMap::new();
            for current in node.ledger.any().account_receivable_upper_bound(&transaction, account, BlockHash::zero()) {
                if receivable_amounts.len() >= count as usize {
                    break;
                }
                let (key, info) = current;
                if include_only_confirmed && !node.ledger.confirmed().block_exists_or_pruned(&transaction, &key.send_block_hash) {
                    continue;
                }
                if info.amount < threshold {
                    continue;
                }
                receivable_amounts.insert(key.send_block_hash, info.amount);
            }
            if !receivable_amounts.is_empty() {
                blocks.insert(account, receivable_amounts);
            }
        }
        if sorting {
            for (_, receivable_amounts) in blocks.iter_mut() {
                *receivable_amounts = receivable_amounts.drain()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .sorted_by(|a, b| b.1.cmp(&a.1))
                    .collect();
            }
        }
        ReceivableDto::Threshold { blocks }
    };

    to_string_pretty(&result).unwrap()
}

#[cfg(test)]
mod tests {
    use crate::service::responses::test_helpers::setup_rpc_client_and_server;
    use rsnano_core::{Account, Amount, BlockEnum, PublicKey, RawKey, StateBlock, WalletId, DEV_GENESIS_KEY};
    use rsnano_ledger::{DEV_GENESIS_ACCOUNT, DEV_GENESIS_HASH, DEV_GENESIS_PUB_KEY};
    use rsnano_node::{node::Node, wallets::WalletsExt};
    use rsnano_rpc_messages::ReceivableDto;
    use std::sync::Arc;
    use std::time::Duration;
    use test_helpers::{assert_timely_msg, System};

    fn send_block(node: Arc<Node>, account: Account) -> BlockEnum {
        let send1 = BlockEnum::State(StateBlock::new(
            *DEV_GENESIS_ACCOUNT,
            *DEV_GENESIS_HASH,
            *DEV_GENESIS_PUB_KEY,
            Amount::MAX - Amount::raw(1),
            account.into(),
            &DEV_GENESIS_KEY,
            node.work_generate_dev((*DEV_GENESIS_HASH).into()),
        ));

        node.process_active(send1.clone());
        assert_timely_msg(
            Duration::from_secs(5),
            || node.active.active(&send1),
            "not active on node 1",
        );

        send1
    }

    #[test]
    fn accounts_receivable_include_only_confirmed_true() {
        let mut system = System::new();
        let node = system.make_node();

        let wallet = WalletId::zero();
        node.wallets.create(wallet);
        let private_key = RawKey::zero();
        let public_key: PublicKey = (&private_key).try_into().unwrap();
        node.wallets.insert_adhoc2(&wallet, &private_key, false).unwrap();

        let send = send_block(node.clone(), public_key.into());
        node.ledger.confirm(&mut node.ledger.rw_txn(), send.hash());

        let (rpc_client, server) = setup_rpc_client_and_server(node.clone(), false);

        let result = node.tokio.block_on(async {
            rpc_client
                .accounts_receivable(vec![public_key.into()], 1, None, None, None, Some(true))
                .await
                .unwrap()
        });

        if let ReceivableDto::Blocks { blocks } = result {
            assert_eq!(blocks.get(&public_key.into()).unwrap(), &vec![send.hash()]);
        } else {
            panic!("Expected ReceivableDto::Blocks variant");
        }

        server.abort();
    }

    #[test]
    fn accounts_receivable_include_only_confirmed_false() {
        let mut system = System::new();
        let node = system.make_node();

        let wallet = WalletId::zero();
        node.wallets.create(wallet);
        let private_key = RawKey::zero();
        let public_key: PublicKey = (&private_key).try_into().unwrap();
        node.wallets.insert_adhoc2(&wallet, &private_key, false).unwrap();

        let send = send_block(node.clone(), public_key.into());

        let (rpc_client, server) = setup_rpc_client_and_server(node.clone(), false);

        let result = node.tokio.block_on(async {
            rpc_client
                .accounts_receivable(vec![public_key.into()], 1, None, None, None, Some(false))
                .await
                .unwrap()
        });

        if let ReceivableDto::Blocks { blocks } = result {
            assert_eq!(blocks.get(&public_key.into()).unwrap(), &vec![send.hash()]);
        } else {
            panic!("Expected ReceivableDto::Blocks variant");
        }

        server.abort();
    }

    #[test]
    fn accounts_receivable_options_none() {
        let mut system = System::new();
        let node = system.make_node();

        let wallet = WalletId::zero();
        node.wallets.create(wallet);
        let private_key = RawKey::zero();
        let public_key: PublicKey = (&private_key).try_into().unwrap();
        node.wallets.insert_adhoc2(&wallet, &private_key, false).unwrap();

        let send = send_block(node.clone(), public_key.into());

        let (rpc_client, server) = setup_rpc_client_and_server(node.clone(), false);

        let result = node.tokio.block_on(async {
            rpc_client
                .accounts_receivable(vec![DEV_GENESIS_KEY.public_key().as_account()], 1, None, None, None, None)
                .await
                .unwrap()
        });

        //assert!(result.value.is_empty());

        server.abort();
    }

    #[test]
    fn accounts_receivable_threshold_some() {
        let mut system = System::new();
        let node = system.make_node();

        let wallet = WalletId::zero();
        node.wallets.create(wallet);
        let private_key = RawKey::zero();
        let public_key: PublicKey = (&private_key).try_into().unwrap();
        node.wallets.insert_adhoc2(&wallet, &private_key, false).unwrap();

        let send = send_block(node.clone(), public_key.into());

        let (rpc_client, server) = setup_rpc_client_and_server(node.clone(), false);

        let result = node.tokio.block_on(async {
            rpc_client
                .accounts_receivable(vec![public_key.into()], 1, Some(Amount::raw(2)), None, None, Some(false))
                .await
                .unwrap()
        });

        if let ReceivableDto::Threshold { blocks } = result {
            assert_eq!(blocks.len(), 0);
        } else {
            panic!("Expected ReceivableDto::Threshold variant");
        }

        server.abort();
    }

    #[test]
    fn accounts_receivable_sorting() {
        let mut system = System::new();
        let node = system.make_node();

        let wallet = WalletId::zero();
        node.wallets.create(wallet);
        let private_key = RawKey::zero();
        let public_key: PublicKey = (&private_key).try_into().unwrap();
        node.wallets.insert_adhoc2(&wallet, &private_key, false).unwrap();

        let send = send_block(node.clone(), public_key.into());

        let (rpc_client, server) = setup_rpc_client_and_server(node.clone(), false);

        let result = node.tokio.block_on(async {
            rpc_client
                .accounts_receivable(vec![public_key.into()], 1, None, None, Some(true), Some(false))
                .await
                .unwrap()
        });

        if let ReceivableDto::Threshold { blocks } = result {
            assert_eq!(blocks.len(), 1);
            let (recv_account, recv_blocks) = blocks.iter().next().unwrap();
            assert_eq!(recv_account, &public_key.into());
            assert_eq!(recv_blocks.len(), 1);
            let (block_hash, amount) = recv_blocks.iter().next().unwrap();
            assert_eq!(block_hash, &send.hash());
            assert_eq!(amount, &Amount::raw(1));
        } else {
            panic!("Expected ReceivableDto::Threshold variant");
        }

        server.abort();
    }
}