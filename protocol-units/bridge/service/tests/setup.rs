use crate::movement::SetupMovementClient;
use alloy::{
	primitives::{Address, U256},
	providers::WalletProvider,
	signers::{
		k256::{elliptic_curve::SecretKey, Secp256k1},
		local::LocalSigner,
	},
};
use alloy_network::{Ethereum, EthereumWallet, NetworkWallet};
use anyhow::Result;
use aptos_sdk::rest_client::{Client, FaucetClient};
use aptos_sdk::types::LocalAccount;
use bridge_service::chains::bridge_contracts::BridgeContract;
use bridge_service::chains::bridge_contracts::BridgeContractResult;
use bridge_service::chains::ethereum::client::{Config as EthConfig, EthClient};
use bridge_service::chains::ethereum::types::AlloyProvider;
use bridge_service::chains::ethereum::types::AtomicBridgeInitiator;
use bridge_service::chains::ethereum::types::{EthAddress, WETH9}; //AlloyProvider, AtomicBridgeInitiator,
use bridge_service::types::{Amount, BridgeAddress, HashLock, TimeLock};
use rand::SeedableRng;
use std::sync::{Arc, RwLock};

pub mod movement;

pub async fn new_setup_with_local_movement() -> (SetupMovementClient, tokio::process::Child) {
	let (movement_client, child) = SetupMovementClient::setup_local_movement_network()
		.await
		.expect("Failed to create MovementClient");
	(Self { eth_client: None, movement_client: movement_client }, child)
}

pub async fn new_only_eth() -> Self {
	let eth_client = EthClient::new(EthConfig::build_for_test())
		.await
		.expect("Failed to create EthClient");
	Self { eth_client: Some(eth_client), movement_client: None }
}

pub fn eth_client(&self) -> Result<&EthClient> {
	self.eth_client.as_ref().ok_or(anyhow::Error::msg("EthClient not initialized"))
}

pub fn eth_client_mut(&mut self) -> Result<&mut EthClient> {
	self.eth_client.as_mut().ok_or(anyhow::Error::msg("EthClient not initialized"))
}

pub fn set_eth_signer(&mut self, signer: SecretKey<Secp256k1>) -> Address {
	let eth_client = self.eth_client_mut().expect("EthClient not initialized");
	let wallet: &mut EthereumWallet = eth_client.rpc_provider_mut().wallet_mut();
	let clone_signer = signer.clone();
	wallet.register_default_signer(LocalSigner::from(signer));
	eth_client.set_signer_address(clone_signer);
	eth_client.get_signer_address()
}

pub fn eth_signer_address(&self) -> Address {
	let eth_client = self.eth_client().expect("EthClient not initialized");
	let wallet: &EthereumWallet = eth_client.rpc_provider().wallet();
	<EthereumWallet as NetworkWallet<Ethereum>>::default_signer_address(wallet)
}

pub fn provider(&self) -> &AlloyProvider {
	self.eth_client().expect("Could not fetch eth client").rpc_provider()
}

/// The port that Anvil will listen on.
pub fn rpc_port(&self) -> u16 {
	self.eth_client().expect("Could not fetch eth client").rpc_port()
}

pub async fn deploy_initiator_contract(&mut self) -> Address {
	let eth_client: &mut EthClient = self.eth_client_mut().expect("EthClient not initialized");
	let contract = AtomicBridgeInitiator::deploy(eth_client.rpc_provider())
		.await
		.expect("Failed to deploy AtomicBridgeInitiator");
	eth_client.set_initiator_contract(contract.with_cloned_provider());
	eth_client.initiator_contract_address().expect("Initiator contract not set")
}

pub async fn deploy_weth_contract(&mut self) -> Address {
	let eth_client = self.eth_client_mut().expect("EthClient not initialized");
	let weth = WETH9::deploy(eth_client.rpc_provider()).await.expect("Failed to deploy WETH9");
	eth_client.set_weth_contract(weth.with_cloned_provider());
	eth_client.weth_contract_address().expect("WETH contract not set")
}

pub async fn deploy_init_contracts(&mut self) {
	let _ = self.deploy_initiator_contract().await;
	let weth_address = self.deploy_weth_contract().await;
	self.eth_client()
		.expect("Failed to get EthClient")
		.initialize_initiator_contract(
			EthAddress(weth_address),
			EthAddress(self.eth_signer_address()),
		)
		.await
		.expect("Failed to initialize contract");
}

pub async fn initiate_bridge_transfer(
	&mut self,
	initiator_address: BridgeAddress<EthAddress>,
	recipient_address: BridgeAddress<Vec<u8>>,
	hash_lock: HashLock,
	time_lock: TimeLock,
	amount: Amount, // the amount
) -> BridgeContractResult<()> {
	let eth_client = self.eth_client_mut().expect("EthClient not initialized");
	eth_client
		.initiate_bridge_transfer(
			initiator_address,
			recipient_address,
			hash_lock,
			time_lock,
			amount,
		)
		.await
}

pub async fn deposit_weth_and_approve(
	&mut self,
	initiator_address: BridgeAddress<EthAddress>,
	amount: Amount,
) -> BridgeContractResult<()> {
	let eth_client = self.eth_client_mut().expect("EthClient not initialized");
	eth_client
		.deposit_weth_and_approve(initiator_address.0 .0, U256::from(amount.weth()))
		.await
		.expect("Failed to deposit WETH");
	Ok(())
}

pub fn gen_aptos_account(&self) -> Vec<u8> {
	let mut rng = ::rand::rngs::StdRng::from_seed([3u8; 32]);
	let movement_recipient = LocalAccount::generate(&mut rng);
	movement_recipient.public_key().to_bytes().to_vec()
}
