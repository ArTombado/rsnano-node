use rsnano_core::{Account, KeyPair};
use rsnano_rpc_messages::KeyPairDto;
use toml::to_string_pretty;

pub async fn key_create() -> String {
    let keypair = KeyPair::new();
    let private = keypair.private_key();
    let public = keypair.public_key();
    let account = Account::from(public);

    to_string_pretty(&KeyPairDto::new(private, public, account)).unwrap()
}

#[cfg(test)]
mod tests {
    use crate::service::responses::test_helpers::setup_rpc_client_and_server;
    use test_helpers::System;

    #[test]
    fn key_create() {
        let mut system = System::new();
        let node = system.make_node();

        let (rpc_client, server) = setup_rpc_client_and_server(node.clone());

        node.tokio
            .block_on(async { rpc_client.key_create().await.unwrap() });

        server.abort();
    }
}
