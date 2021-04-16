// This file is part of Substrate.

// Copyright (C) 2019-2021 Parity Technologies (UK) Ltd.
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

//! Substrate offchain workers.
//!
//! The offchain workers is a special function of the runtime that
//! gets executed after block is imported. During execution
//! it's able to asynchronously submit extrinsics that will either
//! be propagated to other nodes or added to the next block
//! produced by the node as unsigned transactions.
//!
//! Offchain workers can be used for computation-heavy tasks
//! that are not feasible for execution during regular block processing.
//! It can either be tasks that no consensus is required for,
//! or some form of consensus over the data can be built on-chain
//! for instance via:
//! 1. Challenge period for incorrect computations
//! 2. Majority voting for results
//! 3. etc

#![warn(missing_docs)]

use std::{
	fmt, marker::PhantomData, sync::Arc,
	collections::HashSet,
};

use parking_lot::Mutex;
use threadpool::ThreadPool;
use sp_api::{ApiExt, ProvideRuntimeApi};
use futures::future::Future;
use log::{debug, warn};
use sc_network::{ExHashT, NetworkService, NetworkStateInfo, PeerId};
use sp_core::{offchain, ExecutionContext, traits::SpawnNamed};
use sp_runtime::{generic::BlockId, traits::{self, Header}};
use futures::{prelude::*, future::ready};

mod api;

pub use api::Db as OffchainDb;
pub use sp_offchain::{OffchainWorkerApi, STORAGE_PREFIX};

/// NetworkProvider provides [`OffchainWorkers`] with all necessary hooks into the
/// underlying Substrate networking.
pub trait NetworkProvider: NetworkStateInfo {
	/// Set the authorized peers.
	fn set_authorized_peers(&self, peers: HashSet<PeerId>);

	/// Set the authorized only flag.
	fn set_authorized_only(&self, reserved_only: bool);
}

impl<B, H> NetworkProvider for NetworkService<B, H>
where
	B: traits::Block + 'static,
	H: ExHashT,
{
	fn set_authorized_peers(&self, peers: HashSet<PeerId>) {
		self.set_authorized_peers(peers)
	}

	fn set_authorized_only(&self, reserved_only: bool) {
		self.set_authorized_only(reserved_only)
	}
}

/// An offchain workers manager.
pub struct OffchainWorkers<Client, Block: traits::Block> {
	client: Arc<Client>,
	_block: PhantomData<Block>,
	thread_pool: Mutex<ThreadPool>,
	shared_client: api::SharedClient,
}

impl<Client, Block: traits::Block> OffchainWorkers<Client, Block> {
	/// Creates new `OffchainWorkers`.
	pub fn new(client: Arc<Client>) -> Self {
		let shared_client = api::SharedClient::new();
		Self {
			client,
			_block: PhantomData,
			thread_pool: Mutex::new(ThreadPool::new(num_cpus::get())),
			shared_client,
		}
	}
}

impl<Client, Block: traits::Block> fmt::Debug for OffchainWorkers<
	Client,
	Block,
> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_tuple("OffchainWorkers").finish()
	}
}

impl<Client, Block> OffchainWorkers<
	Client,
	Block,
> where
	Block: traits::Block,
	Client: ProvideRuntimeApi<Block> + Send + Sync + 'static,
	Client::Api: OffchainWorkerApi<Block>,
{
	/// Start the offchain workers after given block.
	#[must_use]
	pub fn on_block_imported(
		&self,
		header: &Block::Header,
		network_provider: Arc<dyn NetworkProvider + Send + Sync>,
		is_validator: bool,
	) -> impl Future<Output = ()> {
		let runtime = self.client.runtime_api();
		let at = BlockId::hash(header.hash());
		let has_api_v1 = runtime.has_api_with::<dyn OffchainWorkerApi<Block>, _>(
			&at, |v| v == 1
		);
		let has_api_v2 = runtime.has_api_with::<dyn OffchainWorkerApi<Block>, _>(
			&at, |v| v == 2
		);
		let version = match (has_api_v1, has_api_v2) {
			(_, Ok(true)) => 2,
			(Ok(true), _) => 1,
			err => {
				let help = "Consider turning off offchain workers if they are not part of your runtime.";
				log::error!("Unsupported Offchain Worker API version: {:?}. {}.", err, help);
				0
			}
		};
		debug!("Checking offchain workers at {:?}: version:{}", at, version);
		if version > 0 {
			let (api, runner) = api::AsyncApi::new(
				network_provider,
				is_validator,
				self.shared_client.clone(),
			);
			debug!("Spawning offchain workers at {:?}", at);
			let header = header.clone();
			let client = self.client.clone();
			self.spawn_worker(move || {
				let runtime = client.runtime_api();
				let api = Box::new(api);
				debug!("Running offchain workers at {:?}", at);
				let context = ExecutionContext::OffchainCall(Some(
					(api, offchain::Capabilities::all())
				));
				let run = if version == 2 {
					runtime.offchain_worker_with_context(&at, context, &header)
				} else {
					#[allow(deprecated)]
					runtime.offchain_worker_before_version_2_with_context(
						&at, context, *header.number()
					)
				};
				if let Err(e) =	run {
					log::error!("Error running offchain workers at {:?}: {:?}", at, e);
				}
			});
			futures::future::Either::Left(runner.process())
		} else {
			futures::future::Either::Right(futures::future::ready(()))
		}
	}

	/// Spawns a new offchain worker.
	///
	/// We spawn offchain workers for each block in a separate thread,
	/// since they can run for a significant amount of time
	/// in a blocking fashion and we don't want to block the runtime.
	///
	/// Note that we should avoid that if we switch to future-based runtime in the future,
	/// alternatively:
	fn spawn_worker(&self, f: impl FnOnce() -> () + Send + 'static) {
		self.thread_pool.lock().execute(f);
	}
}

/// Inform the offchain worker about new imported blocks
pub async fn notification_future<Client, Block, Spawner>(
	is_validator: bool,
	client: Arc<Client>,
	offchain: Arc<OffchainWorkers<Client, Block>>,
	spawner: Spawner,
	network_provider: Arc<dyn NetworkProvider + Send + Sync>,
)
	where
		Block: traits::Block,
		Client: ProvideRuntimeApi<Block> + sc_client_api::BlockchainEvents<Block> + Send + Sync + 'static,
		Client::Api: OffchainWorkerApi<Block>,
		Spawner: SpawnNamed
{
	client.import_notification_stream().for_each(move |n| {
		if n.is_new_best {
			spawner.spawn(
				"offchain-on-block",
				offchain.on_block_imported(
					&n.header,
					network_provider.clone(),
					is_validator,
				).boxed(),
			);
		} else {
			log::debug!(
				target: "sc_offchain",
				"Skipping offchain workers for non-canon block: {:?}",
				n.header,
			)
		}

		ready(())
	}).await;
}

