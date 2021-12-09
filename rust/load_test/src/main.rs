use anyhow::anyhow;
use anyhow::Result;
use clap::{App, Arg};
use rand::Rng;
use rsnano::secure::DEV_GENESIS;
use rsnano::secure::DEV_GENESIS_KEY;
use rsnano::secure::DEV_NETWORK_PARAMS;
use serde_json::json;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use rsnano::{
    config::{
        force_nano_dev_network, get_node_toml_config_path, get_rpc_toml_config_path, DaemonConfig,
        NetworkConstants, RpcConfig,
    },
    secure::{unique_path, NetworkParams},
    utils::TomlConfig,
};

const RPC_PORT_START: u16 = 60000;
const PEERING_PORT_START: u16 = 61000;
const IPC_PORT_START: u16 = 62000;

fn write_config_files(data_path: &Path, index: usize) -> Result<()> {
    let network_params = NetworkParams::new(NetworkConstants::active_network())?;
    let mut daemon_config = DaemonConfig::new(&network_params)?;
    daemon_config.node.peering_port = PEERING_PORT_START + index as u16;
    daemon_config
        .node
        .ipc_config
        .transport_tcp
        .transport
        .enabled = true;
    daemon_config.node.ipc_config.transport_tcp.port = IPC_PORT_START + index as u16;

    // Alternate use of memory pool
    daemon_config.node.use_memory_pools = (index % 2) == 0;

    // Write daemon config
    let mut toml = TomlConfig::new();
    daemon_config.serialize_toml(&mut toml)?;
    toml.write(get_node_toml_config_path(data_path))?;

    let mut rpc_config = RpcConfig::new(&network_params.network);
    rpc_config.port = RPC_PORT_START + index as u16;
    rpc_config.enable_control = true;
    rpc_config.rpc_process.ipc_port = IPC_PORT_START + index as u16;

    // Write rpc config
    let mut toml_rpc = TomlConfig::new();
    rpc_config.serialize_toml(&mut toml_rpc)?;
    toml_rpc.write(get_rpc_toml_config_path(data_path))?;
    Ok(())
}

#[derive(Debug)]
struct Account {
    pub private_key: String,
    pub public_key: String,
    pub as_string: String,
}

// class account_info final
// {
// public:
// 	bool operator== (account_info const & other)
// 	{
// 		return frontier == other.frontier && block_count == other.block_count && balance == other.balance && error == other.error;
// 	}

// 	std::string frontier;
// 	std::string block_count;
// 	std::string balance;
// 	bool error{ false };
// };

async fn send_receive(
    wallet: &str,
    source: &str,
    destination: &str,
) -> Result<()> {
    let request = json!({
        "action": "send",
        "wallet": wallet,
        "source": source,
        "destination": destination,
        "amount": "1"
    });

    let client = reqwest::Client::new();

    let url = format!("http://[::1]:{}/", RPC_PORT_START);
    let json: serde_json::Value = client
        .post(&url)
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let block = json["block"].to_string();

    let request = json!({
        "action": "receive",
        "wallet": wallet,
        "account": destination,
        "block": block
    });
    client
        .post(url)
        .json(&request)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn rpc_request(request: &serde_json::Value) -> Result<serde_json::Value> {
    let client = reqwest::ClientBuilder::new()
        .timeout(Duration::from_secs(5))
        .build()?;
    let result = client
        .post(format!("http://[::1]:{}/", RPC_PORT_START))
        .json(request)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    Ok(result)
}

async fn keepalive_rpc(port: u16) -> Result<()> {
    let request = json!({
        "action": "keepalive",
        "address": "::1",
        "port": port
    });
    rpc_request(&request).await?;
    Ok(())
}

async fn key_create_rpc() -> Result<Account> {
    let request = json!({
        "action": "key_create"
    });
    let json = rpc_request(&request).await?;

    let account = Account {
        private_key: json["private"].to_string(),
        public_key: json["public"].to_string(),
        as_string: json["account"].to_string(),
    };

    Ok(account)
}

async fn wallet_create_rpc() -> Result<String> {
    let request = json!({
        "action": "wallet_create"
    });
    let json = rpc_request(&request).await?;
    Ok(json["wallet"].to_string())
}

async fn wallet_add_rpc(wallet: &str, prv_key: &str) -> Result<()> {
    let request = json!({
        "action": "wallet_add",
        "wallet": wallet,
        "key": prv_key
    });
    rpc_request(&request).await?;
    Ok(())
}

// void stop_rpc (boost::asio::io_context & ioc, tcp::resolver::results_type const & results)
// {
// 	boost::property_tree::ptree request;
// 	request.put ("action", "stop");
// 	rpc_request (request, ioc, results);
// }

// account_info account_info_rpc (boost::asio::io_context & ioc, tcp::resolver::results_type const & results, std::string const & account)
// {
// 	boost::property_tree::ptree request;
// 	request.put ("action", "account_info");
// 	request.put ("account", account);

// 	account_info account_info;
// 	auto json = rpc_request (request, ioc, results);

// 	auto error = json.get_optional<std::string> ("error");
// 	if (error)
// 	{
// 		account_info.error = true;
// 	}
// 	else
// 	{
// 		account_info.balance = json.get<std::string> ("balance");
// 		account_info.block_count = json.get<std::string> ("block_count");
// 		account_info.frontier = json.get<std::string> ("frontier");
// 	}
// 	return account_info;
// }

#[tokio::main]
async fn main() -> Result<()> {
    force_nano_dev_network();

    let matches = App::new("Nano Load Test")
        .about("This launches a node and fires a lot of send/recieve RPC requests at it (configurable), then other nodes are tested to make sure they observe these blocks as well.")
        .arg(Arg::with_name("node_count").short("n").long("node_count").help("The number of nodes to spin up").default_value("10"))
        .arg(Arg::with_name("node_path").long("node_path").takes_value(true).help( "The path to the nano_node to test"))
        .arg(Arg::with_name("rpc_path").long("rpc_path").takes_value(true).help("The path to the nano_rpc to test"))
        .arg(Arg::with_name("destination_count").long("destination_count").takes_value(true).default_value("2").help("How many destination accounts to choose between"))
        .arg(Arg::with_name("send_count").short("s").long("send_count").takes_value(true).default_value("2000").help("How many send blocks to generate"))
        .arg(Arg::with_name("simultaneous_process_calls").long("simultaneous_process_calls").takes_value(true).default_value("20").help("Number of simultaneous rpc sends to do"))
        .get_matches();

    let node_count = matches
        .value_of("node_count")
        .unwrap()
        .parse::<usize>()
        .unwrap();

    let destination_count = matches
        .value_of("destination_count")
        .unwrap()
        .parse::<usize>()
        .unwrap();

    let send_count = matches
        .value_of("send_count")
        .unwrap()
        .parse::<usize>()
        .unwrap();

    let simultaneous_process_calls = matches
        .value_of("simultaneous_process_calls")
        .unwrap()
        .parse::<usize>()
        .unwrap();

    let running_executable_filepath = std::env::current_exe().unwrap();

    let node_path: PathBuf = match matches.value_of("node_path") {
        Some(p) => p.into(),
        None => {
            let mut node_filepath = running_executable_filepath.clone();
            node_filepath.pop(); //debug
            node_filepath.pop(); //build
            node_filepath.pop(); //cargo
            node_filepath.pop(); //root
            node_filepath.push("nano_node");
            if let Some(ext) = running_executable_filepath.extension() {
                node_filepath.set_extension(ext);
            }
            node_filepath
        }
    };

    if !node_path.exists() {
        panic!("nano_node executable could not be found in {:?}", node_path);
    }

    let rpc_path: PathBuf = match matches.value_of("rpc_path") {
        Some(p) => p.into(),
        None => {
            let mut rpc_filepath = running_executable_filepath.clone();
            rpc_filepath.pop(); //debug
            rpc_filepath.pop(); //build
            rpc_filepath.pop(); //cargo
            rpc_filepath.pop(); //root
            rpc_filepath.push("nano_rpc");
            if let Some(ext) = running_executable_filepath.extension() {
                rpc_filepath.set_extension(ext);
            }
            rpc_filepath
        }
    };

    if !rpc_path.exists() {
        panic!("nano_rpc executable could not be found in {:?}", rpc_path);
    }

    let mut data_paths = Vec::new();
    for i in 0..node_count {
        let data_path = unique_path().ok_or_else(|| anyhow!("no unique path"))?;
        std::fs::create_dir(data_path.as_path())?;
        write_config_files(data_path.as_path(), i)?;
        data_paths.push(data_path);
    }

    let current_network = DEV_NETWORK_PARAMS.network.get_current_network_as_string();
    let mut nodes: Vec<Child> = Vec::new();
    let mut rpc_servers: Vec<Child> = Vec::new();
    for data_path in &data_paths {
        nodes.push(
            Command::new(node_path.as_os_str())
                .arg("--daemon")
                .arg("--data_path")
                .arg(data_path)
                .arg("--network")
                .arg(current_network)
                .spawn()
                .expect("could not spawn node"),
        );
        rpc_servers.push(
            Command::new(rpc_path.as_os_str())
                .arg("--daemon")
                .arg("--data_path")
                .arg(data_path)
                .arg("--network")
                .arg(current_network)
                .spawn()
                .expect("could not spawn rpc server"),
        );
    }

    println!("Waiting for nodes to spin up...");
    sleep(Duration::from_secs(7)).await;
    println!("Connecting nodes...");

    // 	boost::asio::io_context ioc;
    // 	debug_assert (!nano::signal_handler_impl);
    // 	nano::signal_handler_impl = [&ioc] () {
    // 		ioc.stop ();
    // 	};

    // 	std::signal (SIGINT, &nano::signal_handler);
    // 	std::signal (SIGTERM, &nano::signal_handler);

    // 	tcp::resolver resolver{ ioc };
    // 	auto const primary_node_results = resolver.resolve ("::1", std::to_string (rpc_port_start));

    // 	std::thread t ([send_count, &ioc, &primary_node_results, &resolver, &node_count, &destination_count] () {
    let t = tokio::spawn(async move {
        for i in 0..node_count {
            keepalive_rpc(PEERING_PORT_START + i as u16).await?;
        }

        println!("Beginning tests");

        // Create keys
        let mut destination_accounts = Vec::new();
        for _ in 0..destination_count {
            destination_accounts.push(key_create_rpc().await?);
        }
        let destination_accounts = Arc::new(destination_accounts);

        // Create wallet
        let wallet = Arc::new(wallet_create_rpc().await?);

        // Add genesis account to it
        wallet_add_rpc(&wallet, &DEV_GENESIS_KEY.private_key().encode_hex()).await?;

        // Add destination accounts
        for account in destination_accounts.iter() {
            wallet_add_rpc(&wallet, &account.private_key).await?;
        }

        print!("\rPrimary node processing transactions: 00%");
        std::io::stdout().flush()?;

        let send_calls_remaining = Arc::new(AtomicUsize::new(send_count));

        let join_handles = (0..send_count).map(|i| {
            // Send from genesis account to different accounts and receive the funds

            let destination_accounts = destination_accounts.clone();
            let wallet = wallet.clone();
            let send_calls_remaining = send_calls_remaining.clone();

            tokio::spawn(async move {
                let destination_account = if i < destination_accounts.len() {
                    &destination_accounts[i]
                } else {
                    let random_account_index =
                        rand::thread_rng().gen_range(0..destination_accounts.len());
                    &destination_accounts[random_account_index]
                };

                let genesis_account = DEV_GENESIS.as_block().account().encode_account();

                let res = send_receive(
                    &wallet,
                    &genesis_account,
                    &destination_account.as_string,
                )
                .await;
                send_calls_remaining.fetch_sub(1, Ordering::SeqCst);
                res
            })
        }).collect::<Vec<_>>();

        let mut last_percent = 0;
        while send_calls_remaining.load(Ordering::SeqCst) != 0 {
            let percent = (100_f64 * ((send_count as f64 - send_calls_remaining.load(Ordering::SeqCst) as f64) / (send_count as f64))) as i32;
            if last_percent != percent {
                print!("\rPrimary node processing transactions: {:02}%. remaining: {:04}", percent, send_calls_remaining.load(Ordering::SeqCst));
                std::io::stdout().flush()?;
                last_percent = percent;
                sleep(Duration::from_millis(100)).await;
            }
        }

        for h in join_handles{
            h.await??;
        }

        println!("\rPrimary node processed transactions                ");
        println!("Waiting for nodes to catch up...");

        // 		std::map<std::string, account_info> known_account_info;
        // 		for (int i = 0; i < destination_accounts.size (); ++i)
        // 		{
        // 			known_account_info.emplace (destination_accounts[i].as_string, account_info_rpc (ioc, primary_node_results, destination_accounts[i].as_string));
        // 		}

        // 		nano::timer<std::chrono::milliseconds> timer;
        // 		timer.start ();

        // 		for (int i = 1; i < node_count; ++i)
        // 		{
        // 			auto const results = resolver.resolve ("::1", std::to_string (rpc_port_start + i));
        // 			for (auto & account_info : known_account_info)
        // 			{
        // 				while (true)
        // 				{
        // 					auto other_account_info = account_info_rpc (ioc, results, account_info.first);
        // 					if (!other_account_info.error && account_info.second == other_account_info)
        // 					{
        // 						// Found the account in this node
        // 						break;
        // 					}

        // 					if (timer.since_start () > std::chrono::seconds (120))
        // 					{
        // 						throw std::runtime_error ("Timed out");
        // 					}

        // 					std::this_thread::sleep_for (std::chrono::seconds (1));
        // 				}
        // 			}

        // 			stop_rpc (ioc, results);
        // 		}

        // 		// Stop main node
        // 		stop_rpc (ioc, primary_node_results);
        anyhow::Result::<()>::Ok(())
    });
    // 	});
    // 	nano::thread_runner runner (ioc, simultaneous_process_calls);
    t.await??;
    // 	runner.join ();

    // 	for (auto & node : nodes)
    // 	{
    // 		node->wait ();
    // 	}
    // 	for (auto & rpc_server : rpc_servers)
    // 	{
    // 		rpc_server->wait ();
    // 	}

    // 	std::cout << "Done!" << std::endl;
    Ok(())
}
