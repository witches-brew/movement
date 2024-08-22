use crate::utils::MovementAddress;
use anyhow::{Error, Result};
use aptos_sdk::{
	move_types::language_storage::TypeTag,
	rest_client::{Client, FaucetClient},
	types::LocalAccount,
};
use aptos_types::account_address::AccountAddress;
use bridge_shared::{
	bridge_contracts::{
		BridgeContractCounterparty, BridgeContractCounterpartyError,
		BridgeContractCounterpartyResult,
	},
	types::{
		Amount, BridgeTransferDetails, BridgeTransferId, HashLock, HashLockPreImage,
		InitiatorAddress, RecipientAddress, TimeLock,
	},
};
use rand::prelude::*;
use serde::Serialize;
use std::{env, fs, io::{Read, Write}, path::PathBuf, process::{Command, Stdio}};
use std::str::FromStr;
use std::{
	sync::{mpsc, Arc, Mutex, RwLock},
	thread,
};
use tokio::{
	io::{AsyncBufReadExt, BufReader},
	process::Command as TokioCommand,
	sync::oneshot,
	task,
};

use url::Url;

pub mod utils;

const DUMMY_ADDRESS: AccountAddress = AccountAddress::new([0; 32]);
const COUNTERPARTY_MODULE_NAME: &str = "atomic_bridge_counterparty";

enum Call {
	Lock,
	Complete,
	Abort,
	GetDetails,
}

pub struct Config {
	pub rpc_url: Option<String>,
	pub ws_url: Option<String>,
	pub chain_id: String,
	pub signer_private_key: Arc<RwLock<LocalAccount>>,
	pub initiator_contract: Option<MovementAddress>,
	pub gas_limit: u64,
}

impl Config {
	pub fn build_for_test() -> Self {
		let seed = [3u8; 32];
		let mut rng = rand::rngs::StdRng::from_seed(seed);

		Config {
			rpc_url: Some("http://localhost:8080".parse().unwrap()),
			ws_url: Some("ws://localhost:8080".parse().unwrap()),
			chain_id: 4.to_string(),
			signer_private_key: Arc::new(RwLock::new(LocalAccount::generate(&mut rng))),
			initiator_contract: None,
			gas_limit: 10_000_000_000,
		}
	}
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct MovementClient {
	///Address of the counterparty moduke
	counterparty_address: AccountAddress,
	///Address of the initiator module
	initiator_address: Vec<u8>,
	///The Apotos Rest Client
	pub rest_client: Client,
	///The Apotos Rest Client
	pub faucet_client: Option<Arc<RwLock<FaucetClient>>>,
	///The signer account
	signer: Arc<LocalAccount>,
}

impl MovementClient {
	pub async fn new(config: Config) -> Result<Self, anyhow::Error> {
		let node_connection_url = format!("http://127.0.0.1:8080");
		let node_connection_url = Url::from_str(node_connection_url.as_str()).unwrap();

		let rest_client = Client::new(node_connection_url.clone());

		let seed = [3u8; 32];
		let mut rng = rand::rngs::StdRng::from_seed(seed);
		let signer = LocalAccount::generate(&mut rng);

		let mut address_bytes = [0u8; AccountAddress::LENGTH];
        	address_bytes[0..2].copy_from_slice(&[0xca, 0xfe]);
		let counterparty_address = AccountAddress::new(address_bytes);

		Ok(MovementClient {
			counterparty_address,
			initiator_address: Vec::new(), //dummy for now
			rest_client,
			faucet_client: None,
			signer: Arc::new(signer),
		})
	}

	pub async fn new_for_test(
		config: Config,
	) -> Result<(Self, tokio::process::Child), anyhow::Error> {
		let (setup_complete_tx, mut setup_complete_rx) = oneshot::channel();
		let mut child = TokioCommand::new("movement")
			.args(&["node", "run-local-testnet"])
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()?;

		let stdout = child.stdout.take().expect("Failed to capture stdout");
		let stderr = child.stderr.take().expect("Failed to capture stderr");

		let node_handle = task::spawn(async move {
			let mut stdout_reader = BufReader::new(stdout).lines();
			let mut stderr_reader = BufReader::new(stderr).lines();

			loop {
				tokio::select! {
					line = stdout_reader.next_line() => {
						match line {
							Ok(Some(line)) => {
								println!("STDOUT: {}", line);
								if line.contains("Setup is complete") {
									println!("Testnet is up and running!");
									let _ = setup_complete_tx.send(());
																	return Ok(());
								}
							},
							Ok(None) => {
								return Err(anyhow::anyhow!("Unexpected end of stdout stream"));
							},
							Err(e) => {
								return Err(anyhow::anyhow!("Error reading stdout: {}", e));
							}
						}
					},
					line = stderr_reader.next_line() => {
						match line {
							Ok(Some(line)) => {
								println!("STDERR: {}", line);
								if line.contains("Setup is complete") {
									println!("Testnet is up and running!");
									let _ = setup_complete_tx.send(());
																	return Ok(());
								}
							},
							Ok(None) => {
								return Err(anyhow::anyhow!("Unexpected end of stderr stream"));
							}
							Err(e) => {
								return Err(anyhow::anyhow!("Error reading stderr: {}", e));
							}
						}
					}
				}
			}
		});

		setup_complete_rx.await.expect("Failed to receive setup completion signal");
		println!("Setup complete message received.");

		let node_connection_url = format!("http://127.0.0.1:8080");
		let node_connection_url = Url::from_str(node_connection_url.as_str()).unwrap();
		let rest_client = Client::new(node_connection_url.clone());

		let faucet_url = format!("http://127.0.0.1:8081");
		let faucet_url = Url::from_str(faucet_url.as_str()).unwrap();
		let faucet_client = Arc::new(RwLock::new(FaucetClient::new(
			faucet_url.clone(),
			node_connection_url.clone(),
		)));

		let mut rng = ::rand::rngs::StdRng::from_seed([3u8; 32]);
		Ok((
			MovementClient {
				counterparty_address: DUMMY_ADDRESS,
				initiator_address: Vec::new(), // dummy for now
				rest_client,
				faucet_client: Some(faucet_client),
				signer: Arc::new(LocalAccount::generate(&mut rng)),
			},
			child,
		))
	}
	
	pub fn publish_for_test(&self) -> Result<()> {
		//println!("Current directory: {:?}", env::current_dir());
		let mut process = Command::new("movement")
                .args(&["init"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("Failed to execute command");

		let stdin: &mut std::process::ChildStdin = process.stdin.as_mut().expect("Failed to open stdin");

		// Press enter for the first prompt
		stdin.write_all(b"yes\n").expect("Failed to write to stdin");

		// Write "local" to the second prompt
		stdin.write_all(b"local\n").expect("Failed to write to stdin");

		// Press enter for the third prompt
		stdin.write_all(b"\n").expect("Failed to write to stdin");

		// Close stdin to indicate that no more input will be provided
		drop(stdin);

		let addr_output = process
			.wait_with_output()
			.expect("Failed to read command output");

		if !addr_output.stdout.is_empty() {
			println!("stdout: {}", String::from_utf8_lossy(&addr_output.stdout));
		}
	
		if !addr_output.stderr.is_empty() {
			eprintln!("stderr: {}", String::from_utf8_lossy(&addr_output.stderr));
		}
		let addr_output_str = String::from_utf8_lossy(&addr_output.stderr);
		let address = addr_output_str
			.split_whitespace()
			.find(|word| word.starts_with("0x")
		) 
		    	.expect("Failed to extract the Movement account address");
	    
		println!("Extracted address: {}", address);

		let resource_output = Command::new("movement")
			.args(&[
				"account",
				"derive-resource-account-address",
				"--address",
				address,
				"--seed",
				"12345",
			])
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.output()
			.expect("Failed to execute command");

		// Print the output of the resource address command for debugging
		if !resource_output.stdout.is_empty() {
			println!("stdout: {}", String::from_utf8_lossy(&resource_output.stdout));
		}
		if !resource_output.stderr.is_empty() {
			eprintln!("stderr: {}", String::from_utf8_lossy(&resource_output.stderr));
		}

		// Extract the resource address from the JSON output
		let resource_output_str = String::from_utf8_lossy(&resource_output.stdout);
		let resource_address = resource_output_str
			.lines()
			.find(|line| line.contains("\"Result\""))
			.and_then(|line| line.split('"').nth(3))
			.expect("Failed to extract the resource account address");

		// Ensure the address has a "0x" prefix
		let formatted_resource_address = if resource_address.starts_with("0x") {
			resource_address.to_string()
		} else {
			format!("0x{}", resource_address)
		};

		println!("Derived resource address: {}", formatted_resource_address);

		let current_dir = env::current_dir().expect("Failed to get current directory");
		println!("Current directory: {:?}", current_dir);

		let move_toml_path = PathBuf::from("../move-modules/Move.toml");


		// Read the existing content of Move.toml
		let move_toml_content = fs::read_to_string(&move_toml_path)
			.expect("Failed to read Move.toml file");
	
		// Update the content of Move.toml with the new addresses
		let updated_content = move_toml_content
			.replace(r#"resource_addr = ""#, &format!(r#"resource_addr = "{}""#, formatted_resource_address))
			.replace(r#"atomic_bridge = ""#, &format!(r#"atomic_bridge = "{}""#, formatted_resource_address))
			.replace(r#"moveth = ""#, &format!(r#"moveth = "{}""#, formatted_resource_address))
			.replace(r#"master_minter = ""#, &format!(r#"master_minter = "{}""#, formatted_resource_address))
			.replace(r#"minter = ""#, &format!(r#"minter = "{}""#, formatted_resource_address))
			.replace(r#"admin = ""#, &format!(r#"admin = "{}""#, formatted_resource_address))
			.replace(r#"origin_addr = ""#, &format!(r#"origin_addr = "{}""#, address))
			.replace(r#"source_account = ""#, &format!(r#"source_account = "{}""#, address));
	
		// Write the updated content back to Move.toml
		fs::write(move_toml_path, updated_content)
			.expect("Failed to update Move.toml file");
	
		println!("Move.toml updated successfully.");

		let output2 = Command::new("movement")
			.args(&[
				"move", 
				"create-resource-account-and-publish-package",
				"--assume-yes",
				"--address-name",
				"moveth", 
				"--seed",
				"12345",
				"--package-dir", 
				"../move-modules"
			])
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.output()
			.expect("Failed to execute command");

	if !output2.stderr.is_empty() {
        	eprintln!("stderr: {}", String::from_utf8_lossy(&output2.stderr));
    	}

		Ok(())
	}
	
	pub fn rest_client(&self) -> &Client {
		&self.rest_client
	}

	pub fn faucet_client(&self) -> Result<&Arc<RwLock<FaucetClient>>> {
		if let Some(faucet_client) = &self.faucet_client {
			Ok(faucet_client)
		} else {
			Err(anyhow::anyhow!("Faucet client not initialized"))
		}
	}
}

#[async_trait::async_trait]
impl BridgeContractCounterparty for MovementClient {
	type Address = MovementAddress;
	type Hash = [u8; 32];

	async fn lock_bridge_transfer_assets(
		&mut self,
		bridge_transfer_id: BridgeTransferId<Self::Hash>,
		hash_lock: HashLock<Self::Hash>,
		time_lock: TimeLock,
		initiator: InitiatorAddress<Vec<u8>>,
		recipient: RecipientAddress<Self::Address>,
		amount: Amount,
	) -> BridgeContractCounterpartyResult<()> {
		//@TODO properly return an error instead of unwrapping
		let args = vec![
			to_bcs_bytes(&initiator.0).unwrap(),
			to_bcs_bytes(&bridge_transfer_id.0).unwrap(),
			to_bcs_bytes(&hash_lock.0).unwrap(),
			to_bcs_bytes(&time_lock.0).unwrap(),
			to_bcs_bytes(&recipient.0).unwrap(),
			to_bcs_bytes(&amount.0).unwrap(),
		];
		let payload = utils::make_aptos_payload(
			self.counterparty_address,
			COUNTERPARTY_MODULE_NAME,
			"lock_bridge_transfer_assets",
			self.counterparty_type_args(Call::Lock),
			args,
		);
		let _ = utils::send_aptos_transaction(&self.rest_client, self.signer.as_ref(), payload)
			.await
			.map_err(|_| BridgeContractCounterpartyError::LockTransferAssetsError);
		Ok(())
	}

	async fn complete_bridge_transfer(
		&mut self,
		bridge_transfer_id: BridgeTransferId<Self::Hash>,
		preimage: HashLockPreImage,
	) -> BridgeContractCounterpartyResult<()> {
		let args = vec![
			to_bcs_bytes(&self.signer.address()).unwrap(),
			to_bcs_bytes(&bridge_transfer_id.0).unwrap(),
			to_bcs_bytes(&preimage.0).unwrap(),
		];
		let payload = utils::make_aptos_payload(
			self.counterparty_address,
			COUNTERPARTY_MODULE_NAME,
			"complete_bridge_transfer",
			self.counterparty_type_args(Call::Complete),
			args,
		);

		let _ = utils::send_aptos_transaction(&self.rest_client, self.signer.as_ref(), payload)
			.await
			.map_err(|_| BridgeContractCounterpartyError::CompleteTransferError);
		Ok(())
	}

	async fn abort_bridge_transfer(
		&mut self,
		bridge_transfer_id: BridgeTransferId<Self::Hash>,
	) -> BridgeContractCounterpartyResult<()> {
		let args = vec![
			to_bcs_bytes(&self.signer.address()).unwrap(),
			to_bcs_bytes(&bridge_transfer_id.0).unwrap(),
		];
		let payload = utils::make_aptos_payload(
			self.counterparty_address,
			COUNTERPARTY_MODULE_NAME,
			"abort_bridge_transfer",
			self.counterparty_type_args(Call::Abort),
			args,
		);
		let _ = utils::send_aptos_transaction(&self.rest_client, self.signer.as_ref(), payload)
			.await
			.map_err(|_| BridgeContractCounterpartyError::AbortTransferError);
		Ok(())
	}

	async fn get_bridge_transfer_details(
		&mut self,
		_bridge_transfer_id: BridgeTransferId<Self::Hash>,
	) -> BridgeContractCounterpartyResult<Option<BridgeTransferDetails<Self::Address, Self::Hash>>>
	{
		// let _ = utils::send_view_request(
		// 	self.rest_client,
		// 	self.counterparty_address,
		// 	"atomic_bridge_counterparty".to_string(),
		// );
		todo!();
	}
}

impl MovementClient {
	fn counterparty_type_args(&self, call: Call) -> Vec<TypeTag> {
		match call {
			Call::Lock => vec![TypeTag::Address, TypeTag::U64, TypeTag::U64, TypeTag::U8],
			Call::Complete => vec![TypeTag::Address, TypeTag::U64, TypeTag::U8],
			Call::Abort => vec![TypeTag::Address, TypeTag::U64],
			Call::GetDetails => vec![TypeTag::Address, TypeTag::U64],
		}
	}
}

fn to_bcs_bytes<T>(value: &T) -> Result<Vec<u8>, anyhow::Error>
where
	T: Serialize,
{
	Ok(bcs::to_bytes(value)?)
}
