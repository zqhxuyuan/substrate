// This file is part of Substrate.

// Copyright (C) 2018-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

#![warn(unused_extern_crates)]

//! Service implementation. Specialized wrapper over substrate service.

use std::sync::Arc;
use sc_consensus_babe;
use node_primitives::Block;
// use node_runtime::RuntimeApi;
use sc_service::{
	config::Configuration, error::Error as ServiceError, RpcHandlers, TaskManager,
};
use sp_inherents::InherentDataProviders;
use sc_network::{Event, NetworkService};
use sp_runtime::traits::Block as BlockT;
use futures::prelude::*;
use sc_client_api::{ExecutorProvider, RemoteBackend};
use node_executor::Executor;
use sc_telemetry::{Telemetry, TelemetryWorker};
use sc_consensus_babe::SlotProportion;
use log::info;

use sp_consensus_aura::sr25519::AuthorityPair as AuraPair;
use sc_consensus_aura::{ImportQueueParams, StartAuraParams};

type FullClient = sc_service::TFullClient<Block, Executor>;
type FullBackend = sc_service::TFullBackend<Block>;
type FullSelectChain = sc_consensus::LongestChain<FullBackend, Block>;
type FullGrandpaBlockImport =
	grandpa::GrandpaBlockImport<FullBackend, Block, FullClient, FullSelectChain>;
type LightClient = sc_service::TLightClient<Block, Executor>;

pub fn new_partial(
	config: &Configuration,
) -> Result<sc_service::PartialComponents<
	FullClient, FullBackend, FullSelectChain,
	sp_consensus::DefaultImportQueue<Block, FullClient>,
	// sc_transaction_pool::FullPool<Block, FullClient>,
	(
		impl Fn(
			node_rpc::DenyUnsafe,
			sc_rpc::SubscriptionTaskExecutor,
		) -> node_rpc::IoHandler,
		// (
			// sc_consensus_babe::BabeBlockImport<Block, FullClient, FullGrandpaBlockImport>,
			grandpa::LinkHalf<Block, FullClient, FullSelectChain>,
			// sc_consensus_babe::BabeLink<Block>,
		// ),
		grandpa::SharedVoterState,
		Option<Telemetry>,
	)
>, ServiceError> {
	let telemetry = config.telemetry_endpoints.clone()
		.filter(|x| !x.is_empty())
		.map(|endpoints| -> Result<_, sc_telemetry::Error> {
			let worker = TelemetryWorker::new(16)?;
			let telemetry = worker.handle().new_telemetry(endpoints);
			Ok((worker, telemetry))
		})
		.transpose()?;

	let short_name = config.chain_spec.id().clone().to_ascii_lowercase();
	info!("chain spec name:{}", short_name.as_str());

	// Many ways to implements RuntimeApi
	// 1. passing different RuntimeApi
	// 2. passing mock RuntimeApi
	// 3. passing none RuntimeApi

	let (client, backend, keystore_container, task_manager) =
		sc_service::new_full_parts::<Block, Executor>(
			&config,
			telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
		)?;
	let client = Arc::new(client);

	let telemetry = telemetry
		.map(|(worker, telemetry)| {
			task_manager.spawn_handle().spawn("telemetry", worker.run());
			telemetry
		});

	let select_chain = sc_consensus::LongestChain::new(backend.clone());

	// let transaction_pool = sc_transaction_pool::BasicPool::new_full(
	// 	config.transaction_pool.clone(),
	// 	config.role.is_authority().into(),
	// 	config.prometheus_registry(),
	// 	task_manager.spawn_handle(),
	// 	client.clone(),
	// );

	let (grandpa_block_import, grandpa_link) = grandpa::block_import(
		client.clone(),
		&(client.clone() as Arc<_>),
		select_chain.clone(),
		telemetry.as_ref().map(|x| x.handle()),
	)?;
	let justification_import = grandpa_block_import.clone();
	let inherent_data_providers = sp_inherents::InherentDataProviders::new();

	let (block_import, babe_link) = sc_consensus_babe::block_import(
		sc_consensus_babe::Config::get_or_compute(&*client)?,
		grandpa_block_import.clone(),
		client.clone(),
	)?;

	let import_queue = sc_consensus_babe::import_queue(
		babe_link.clone(),
		block_import.clone(),
		Some(Box::new(justification_import)),
		client.clone(),
		select_chain.clone(),
		inherent_data_providers.clone(),
		&task_manager.spawn_essential_handle(),
		config.prometheus_registry(),
		sp_consensus::CanAuthorWithNativeVersion::new(client.executor().clone()),
		telemetry.as_ref().map(|x| x.handle()),
	)?;

	// let import_queue = sc_archive::import_queue(
	// 	grandpa_block_import, client.clone(), &task_manager.spawn_essential_handle()
	// )?;

	// let aura_block_import = sc_consensus_aura::AuraBlockImport::<_, _, _, AuraPair>::new(
	// 	grandpa_block_import.clone(), client.clone(),
	// );
	//
	// let import_queue = sc_consensus_aura::import_queue::<AuraPair, _, _, _, _, _>(
	// 	ImportQueueParams {
	// 		block_import: aura_block_import.clone(),
	// 		justification_import: Some(Box::new(grandpa_block_import.clone())),
	// 		client: client.clone(),
	// 		inherent_data_providers: inherent_data_providers.clone(),
	// 		spawner: &task_manager.spawn_essential_handle(),
	// 		can_author_with: sp_consensus::CanAuthorWithNativeVersion::new(client.executor().clone()),
	// 		slot_duration: sc_consensus_aura::slot_duration(&*client)?,
	// 		registry: config.prometheus_registry(),
	// 		check_for_equivocation: Default::default(),
	// 		telemetry: telemetry.as_ref().map(|x| x.handle()),
	// 	},
	// )?;

	let import_setup = grandpa_link;

	let (rpc_extensions_builder, rpc_setup) = {
		let grandpa_link = &import_setup;

		let justification_stream = grandpa_link.justification_stream();
		let shared_authority_set = grandpa_link.shared_authority_set().clone();
		let shared_voter_state = grandpa::SharedVoterState::empty();
		let rpc_setup = shared_voter_state.clone();

		let finality_proof_provider = grandpa::FinalityProofProvider::new_for_service(
			backend.clone(),
			Some(shared_authority_set.clone()),
		);

		// let babe_config = babe_link.config().clone();
		// let shared_epoch_changes = babe_link.epoch_changes().clone();

		let client = client.clone();
		// let pool = transaction_pool.clone();
		let select_chain = select_chain.clone();
		let keystore = keystore_container.sync_keystore();
		let chain_spec = config.chain_spec.cloned_box();

		let rpc_extensions_builder = move |deny_unsafe, subscription_executor| {
			let deps = node_rpc::FullDeps {
				client: client.clone(),
				// pool: pool.clone(),
				select_chain: select_chain.clone(),
				chain_spec: chain_spec.cloned_box(),
				deny_unsafe,
				// babe: node_rpc::BabeDeps {
				// 	babe_config: babe_config.clone(),
				// 	shared_epoch_changes: shared_epoch_changes.clone(),
				// 	keystore: keystore.clone(),
				// },
				grandpa: node_rpc::GrandpaDeps {
					shared_voter_state: shared_voter_state.clone(),
					shared_authority_set: shared_authority_set.clone(),
					justification_stream: justification_stream.clone(),
					subscription_executor,
					finality_provider: finality_proof_provider.clone(),
				},
			};

			node_rpc::create_full(deps)
		};

		(rpc_extensions_builder, rpc_setup)
	};

	Ok(sc_service::PartialComponents {
		client,
		backend,
		task_manager,
		keystore_container,
		select_chain,
		import_queue,
		// transaction_pool,
		inherent_data_providers,
		other: (rpc_extensions_builder, import_setup, rpc_setup, telemetry),
	})
}

pub struct NewFullBase {
	pub task_manager: TaskManager,
	pub inherent_data_providers: InherentDataProviders,
	pub client: Arc<FullClient>,
	pub network: Arc<NetworkService<Block, <Block as BlockT>::Hash>>,
	pub network_status_sinks: sc_service::NetworkStatusSinks<Block>,
	// pub transaction_pool: Arc<sc_transaction_pool::FullPool<Block, FullClient>>,
}

/// Creates a full service from the configuration.
pub fn new_full_base(
	mut config: Configuration,
	with_startup_data: impl FnOnce(
		&sc_consensus_babe::BabeBlockImport<Block, FullClient, FullGrandpaBlockImport>,
		&sc_consensus_babe::BabeLink<Block>,
	)
) -> Result<NewFullBase, ServiceError> {
	let sc_service::PartialComponents {
		client,
		backend,
		mut task_manager,
		import_queue,
		keystore_container,
		select_chain,
		// transaction_pool,
		inherent_data_providers,
		other: (rpc_extensions_builder, import_setup, rpc_setup, mut telemetry),
	} = new_partial(&config)?;

	let shared_voter_state = rpc_setup;

	config.network.extra_sets.push(grandpa::grandpa_peers_set_config());

	#[cfg(feature = "cli")]
	config.network.request_response_protocols.push(
		sc_finality_grandpa_warp_sync::request_response_config_for_chain(
			&config,
			task_manager.spawn_handle(),
			backend.clone(),
			import_setup.shared_authority_set().clone(),
		)
	);

	let (network, network_status_sinks, system_rpc_tx, network_starter) =
		sc_service::build_network(sc_service::BuildNetworkParams {
			config: &config,
			client: client.clone(),
			// transaction_pool: transaction_pool.clone(),
			spawn_handle: task_manager.spawn_handle(),
			import_queue,
			on_demand: None,
			block_announce_validator_builder: None,
		})?;

	let role = config.role.clone();
	// let force_authoring = config.force_authoring;
	// let backoff_authoring_blocks = Some(sc_consensus_slots::BackoffAuthoringOnFinalizedHeadLagging::default());
	let name = config.network.node_name.clone();
	let enable_grandpa = !config.disable_grandpa;
	let prometheus_registry = config.prometheus_registry().cloned();

	let _rpc_handlers = sc_service::spawn_tasks(
		sc_service::SpawnTasksParams {
			config,
			backend: backend.clone(),
			client: client.clone(),
			keystore: keystore_container.sync_keystore(),
			network: network.clone(),
			rpc_extensions_builder: Box::new(rpc_extensions_builder),
			// transaction_pool: transaction_pool.clone(),
			task_manager: &mut task_manager,
			on_demand: None,
			remote_blockchain: None,
			network_status_sinks: network_status_sinks.clone(),
			system_rpc_tx,
			telemetry: telemetry.as_mut(),
		},
	)?;

	let grandpa_link = import_setup;

	// It's ok to delete this line, cause no one used it except test case
	// (with_startup_data)(&block_import, &babe_link);

	// default role is Full
	// if let sc_service::config::Role::Authority { .. } = &role {
	// 	let proposer = sc_basic_authorship::ProposerFactory::new(
	// 		task_manager.spawn_handle(),
	// 		client.clone(),
	// 		transaction_pool.clone(),
	// 		prometheus_registry.as_ref(),
	// 		telemetry.as_ref().map(|x| x.handle()),
	// 	);
	//
	// 	let can_author_with =
	// 		sp_consensus::CanAuthorWithNativeVersion::new(client.executor().clone());
	//
	// 	let babe_config = sc_consensus_babe::BabeParams {
	// 		keystore: keystore_container.sync_keystore(),
	// 		client: client.clone(),
	// 		select_chain,
	// 		env: proposer,
	// 		block_import,
	// 		sync_oracle: network.clone(),
	// 		inherent_data_providers: inherent_data_providers.clone(),
	// 		force_authoring,
	// 		backoff_authoring_blocks,
	// 		babe_link,
	// 		can_author_with,
	// 		block_proposal_slot_portion: SlotProportion::new(0.5),
	// 		telemetry: telemetry.as_ref().map(|x| x.handle()),
	// 	};
	//
	// 	let babe = sc_consensus_babe::start_babe(babe_config)?;
	// 	task_manager.spawn_essential_handle().spawn_blocking("babe-proposer", babe);
	// }

	// if the node isn't actively participating in consensus then it doesn't
	// need a keystore, regardless of which protocol we use below.
	let keystore = if role.is_authority() {
		Some(keystore_container.sync_keystore())
	} else {
		None
	};

	let config = grandpa::Config {
		// FIXME #1578 make this available through chainspec
		gossip_duration: std::time::Duration::from_millis(333),
		justification_period: 512,
		name: Some(name),
		observer_enabled: false,
		keystore,
		is_authority: role.is_authority(),
		telemetry: telemetry.as_ref().map(|x| x.handle()),
	};

	if enable_grandpa {
		// start the full GRANDPA voter
		// NOTE: non-authorities could run the GRANDPA observer protocol, but at
		// this point the full voter should provide better guarantees of block
		// and vote data availability than the observer. The observer has not
		// been tested extensively yet and having most nodes in a network run it
		// could lead to finality stalls.
		let grandpa_config = grandpa::GrandpaParams {
			config,
			link: grandpa_link,
			network: network.clone(),
			telemetry: telemetry.as_ref().map(|x| x.handle()),
			voting_rule: grandpa::VotingRulesBuilder::default().build(),
			prometheus_registry,
			shared_voter_state,
		};

		// the GRANDPA voter task is considered infallible, i.e.
		// if it fails we take down the service with it.
		task_manager.spawn_essential_handle().spawn_blocking(
			"grandpa-voter",
			grandpa::run_grandpa_voter(grandpa_config)?
		);
	}

	network_starter.start_network();
	Ok(NewFullBase {
		task_manager,
		inherent_data_providers,
		client,
		network,
		network_status_sinks,
		// transaction_pool,
	})
}

/// Builds a new service for a full client.
pub fn new_full(
	config: Configuration,
) -> Result<TaskManager, ServiceError> {
	new_full_base(config, |_, _| ()).map(|NewFullBase { task_manager, .. }| {
		task_manager
	})
}