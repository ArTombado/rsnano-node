use rsnano_core::Account;
use rsnano_node::node::Node;
use rsnano_rpc_messages::{AccountRpcMessage, ErrorDto};
use serde_json::to_string_pretty;
use std::sync::Arc;

pub async fn account_representative(node: Arc<Node>, account: Account) -> String {
    let tx = node.ledger.read_txn();
    match node.ledger.store.account.get(&tx, &account) {
        Some(account_info) => {
            let account_representative = AccountRpcMessage::new(
                "representative".to_string(),
                account_info.representative.as_account(),
            );
            to_string_pretty(&account_representative).unwrap()
        }
        None => to_string_pretty(&ErrorDto::new("Account not found".to_string())).unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use crate::service::responses::test_helpers::setup_rpc_client_and_server;
    use rsnano_ledger::DEV_GENESIS_ACCOUNT;
    use test_helpers::System;

    #[test]
    fn account_block_count() {
        let mut system = System::new();
        let node = system.make_node();

        let (rpc_client, server) = setup_rpc_client_and_server(node.clone(), true);

        let result = node.tokio.block_on(async {
            rpc_client
                .account_representative(DEV_GENESIS_ACCOUNT.to_owned())
                .await
                .unwrap()
        });

        assert_eq!(result.value, DEV_GENESIS_ACCOUNT.to_owned());

        server.abort();
    }
}
