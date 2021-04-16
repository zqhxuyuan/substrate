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

//! A consensus proposer for "basic" chains which use the primitive inherent-data.

// FIXME #1021 move this into sp-consensus

use std::{pin::Pin, time, sync::Arc};
use sc_client_api::backend;
use codec::Decode;
use sp_consensus::{evaluation, Proposal, ProofRecording, DisableProofRecording, EnableProofRecording};
use sp_core::traits::SpawnNamed;
use sp_inherents::InherentData;
use log::{error, info, debug, trace, warn};
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, Hash as HashT, Header as HeaderT, DigestFor, BlakeTwo256},
};
use sp_transaction_pool::{TransactionPool, InPoolTransaction};
use sc_telemetry::{telemetry, TelemetryHandle, CONSENSUS_INFO};
use sc_block_builder::{BlockBuilderApi, BlockBuilderProvider};
use sp_api::{ProvideRuntimeApi, ApiExt};
use futures::{future, future::{Future, FutureExt}, channel::oneshot, select};
use sp_blockchain::{HeaderBackend, ApplyExtrinsicFailed::Validity, Error::ApplyExtrinsicFailed};
use std::marker::PhantomData;

use prometheus_endpoint::Registry as PrometheusRegistry;
use sc_proposer_metrics::MetricsLink as PrometheusMetrics;

/// Default maximum block size in bytes used by [`Proposer`].
///
/// Can be overwritten by [`ProposerFactory::set_maximum_block_size`].
///
/// Be aware that there is also an upper packet size on what the networking code
/// will accept. If the block doesn't fit in such a package, it can not be
/// transferred to other nodes.
pub const DEFAULT_MAX_BLOCK_SIZE: usize = 4 * 1024 * 1024 + 512;

/// Proposer factory.
pub struct ProposerFactory<A, B, C, PR> {
	spawn_handle: Box<dyn SpawnNamed>,
	/// The client instance.
	client: Arc<C>,
	/// The transaction pool.
	transaction_pool: Arc<A>,
	/// Prometheus Link,
	metrics: PrometheusMetrics,
	max_block_size: usize,
	telemetry: Option<TelemetryHandle>,
	/// phantom member to pin the `Backend`/`ProofRecording` type.
	_phantom: PhantomData<(B, PR)>,
}

impl<A, B, C> ProposerFactory<A, B, C, DisableProofRecording> {
	/// Create a new proposer factory.
	///
	/// Proof recording will be disabled when using proposers built by this instance to build blocks.
	pub fn new(
		spawn_handle: impl SpawnNamed + 'static,
		client: Arc<C>,
		transaction_pool: Arc<A>,
		prometheus: Option<&PrometheusRegistry>,
		telemetry: Option<TelemetryHandle>,
	) -> Self {
		ProposerFactory {
			spawn_handle: Box::new(spawn_handle),
			transaction_pool,
			metrics: PrometheusMetrics::new(prometheus),
			max_block_size: DEFAULT_MAX_BLOCK_SIZE,
			telemetry,
			client,
			_phantom: PhantomData,
		}
	}
}

impl<A, B, C> ProposerFactory<A, B, C, EnableProofRecording> {
	/// Create a new proposer factory with proof recording enabled.
	///
	/// Each proposer created by this instance will record a proof while building a block.
	pub fn with_proof_recording(
		spawn_handle: impl SpawnNamed + 'static,
		client: Arc<C>,
		transaction_pool: Arc<A>,
		prometheus: Option<&PrometheusRegistry>,
		telemetry: Option<TelemetryHandle>,
	) -> Self {
		ProposerFactory {
			spawn_handle: Box::new(spawn_handle),
			client,
			transaction_pool,
			metrics: PrometheusMetrics::new(prometheus),
			max_block_size: DEFAULT_MAX_BLOCK_SIZE,
			telemetry,
			_phantom: PhantomData,
		}
	}
}

impl<A, B, C, PR> ProposerFactory<A, B, C, PR> {
	/// Set the maximum block size in bytes.
	///
	/// The default value for the maximum block size is:
	/// [`DEFAULT_MAX_BLOCK_SIZE`].
	pub fn set_maximum_block_size(&mut self, size: usize) {
		self.max_block_size = size;
	}
}

impl<B, Block, C, A, PR> ProposerFactory<A, B, C, PR>
	where
		A: TransactionPool<Block = Block> + 'static,
		B: backend::Backend<Block> + Send + Sync + 'static,
		Block: BlockT,
		C: BlockBuilderProvider<B, Block, C> + HeaderBackend<Block> + ProvideRuntimeApi<Block>
			+ Send + Sync + 'static,
		C::Api: ApiExt<Block, StateBackend = backend::StateBackendFor<B, Block>>
			+ BlockBuilderApi<Block>,
{
	fn init_with_now(
		&mut self,
		parent_header: &<Block as BlockT>::Header,
		now: Box<dyn Fn() -> time::Instant + Send + Sync>,
	) -> Proposer<B, Block, C, A, PR> {
		let parent_hash = parent_header.hash();

		let id = BlockId::hash(parent_hash);

		info!("🙌 Starting consensus session on top of parent {:?}", parent_hash);

		let proposer = Proposer::<_, _, _, _, PR> {
			spawn_handle: self.spawn_handle.clone(),
			client: self.client.clone(),
			parent_hash,
			parent_id: id,
			parent_number: *parent_header.number(),
			transaction_pool: self.transaction_pool.clone(),
			now,
			metrics: self.metrics.clone(),
			max_block_size: self.max_block_size,
			telemetry: self.telemetry.clone(),
			_phantom: PhantomData,
		};

		proposer
	}
}

impl<A, B, Block, C, PR> sp_consensus::Environment<Block> for
	ProposerFactory<A, B, C, PR>
		where
			A: TransactionPool<Block = Block> + 'static,
			B: backend::Backend<Block> + Send + Sync + 'static,
			Block: BlockT,
			C: BlockBuilderProvider<B, Block, C> + HeaderBackend<Block> + ProvideRuntimeApi<Block>
				+ Send + Sync + 'static,
			C::Api: ApiExt<Block, StateBackend = backend::StateBackendFor<B, Block>>
				+ BlockBuilderApi<Block>,
			PR: ProofRecording,
{
	type CreateProposer = future::Ready<Result<Self::Proposer, Self::Error>>;
	type Proposer = Proposer<B, Block, C, A, PR>;
	type Error = sp_blockchain::Error;

	fn init(
		&mut self,
		parent_header: &<Block as BlockT>::Header,
	) -> Self::CreateProposer {
		future::ready(Ok(self.init_with_now(parent_header, Box::new(time::Instant::now))))
	}
}

/// The proposer logic.
pub struct Proposer<B, Block: BlockT, C, A: TransactionPool, PR> {
	spawn_handle: Box<dyn SpawnNamed>,
	client: Arc<C>,
	parent_hash: <Block as BlockT>::Hash,
	parent_id: BlockId<Block>,
	parent_number: <<Block as BlockT>::Header as HeaderT>::Number,
	transaction_pool: Arc<A>,
	now: Box<dyn Fn() -> time::Instant + Send + Sync>,
	metrics: PrometheusMetrics,
	max_block_size: usize,
	telemetry: Option<TelemetryHandle>,
	_phantom: PhantomData<(B, PR)>,
}

impl<A, B, Block, C, PR> sp_consensus::Proposer<Block> for
	Proposer<B, Block, C, A, PR>
		where
			A: TransactionPool<Block = Block> + 'static,
			B: backend::Backend<Block> + Send + Sync + 'static,
			Block: BlockT,
			C: BlockBuilderProvider<B, Block, C> + HeaderBackend<Block> + ProvideRuntimeApi<Block>
				+ Send + Sync + 'static,
			C::Api: ApiExt<Block, StateBackend = backend::StateBackendFor<B, Block>>
				+ BlockBuilderApi<Block>,
			PR: ProofRecording,
{
	type Transaction = backend::TransactionFor<B, Block>;
	type Proposal = Pin<Box<dyn Future<
		Output = Result<Proposal<Block, Self::Transaction, PR::Proof>, Self::Error>
	> + Send>>;
	type Error = sp_blockchain::Error;
	type ProofRecording = PR;
	type Proof = PR::Proof;

	fn propose(
		self,
		inherent_data: InherentData,
		inherent_digests: DigestFor<Block>,
		max_duration: time::Duration,
	) -> Self::Proposal {
		let (tx, rx) = oneshot::channel();
		let spawn_handle = self.spawn_handle.clone();

		spawn_handle.spawn_blocking("basic-authorship-proposer", Box::pin(async move {
			// leave some time for evaluation and block finalization (33%)
			let deadline = (self.now)() + max_duration - max_duration / 3;
			let res = self.propose_with(
				inherent_data,
				inherent_digests,
				deadline,
			).await;
			if tx.send(res).is_err() {
				trace!("Could not send block production result to proposer!");
			}
		}));

		async move {
			rx.await?
		}.boxed()
	}
}

impl<A, B, Block, C, PR> Proposer<B, Block, C, A, PR>
	where
		A: TransactionPool<Block = Block>,
		B: backend::Backend<Block> + Send + Sync + 'static,
		Block: BlockT,
		C: BlockBuilderProvider<B, Block, C> + HeaderBackend<Block> + ProvideRuntimeApi<Block>
			+ Send + Sync + 'static,
		C::Api: ApiExt<Block, StateBackend = backend::StateBackendFor<B, Block>>
			+ BlockBuilderApi<Block>,
		PR: ProofRecording,
{
	async fn propose_with(
		self,
		inherent_data: InherentData,
		inherent_digests: DigestFor<Block>,
		deadline: time::Instant,
	) -> Result<Proposal<Block, backend::TransactionFor<B, Block>, PR::Proof>, sp_blockchain::Error> {
		/// If the block is full we will attempt to push at most
		/// this number of transactions before quitting for real.
		/// It allows us to increase block utilization.
		const MAX_SKIPPED_TRANSACTIONS: usize = 8;

		let mut block_builder = self.client.new_block_at(
			&self.parent_id,
			inherent_digests,
			PR::ENABLED,
		)?;

		for inherent in block_builder.create_inherents(inherent_data)? {
			match block_builder.push(inherent) {
				Err(ApplyExtrinsicFailed(Validity(e))) if e.exhausted_resources() =>
					warn!("⚠️  Dropping non-mandatory inherent from overweight block."),
				Err(ApplyExtrinsicFailed(Validity(e))) if e.was_mandatory() => {
					error!("❌️ Mandatory inherent extrinsic returned error. Block cannot be produced.");
					Err(ApplyExtrinsicFailed(Validity(e)))?
				}
				Err(e) => {
					warn!("❗️ Inherent extrinsic returned unexpected error: {}. Dropping.", e);
				}
				Ok(_) => {}
			}
		}

		// proceed with transactions
		let block_timer = time::Instant::now();
		let mut skipped = 0;
		let mut unqueue_invalid = Vec::new();

		let mut t1 = self.transaction_pool.ready_at(self.parent_number).fuse();
		let mut t2 = futures_timer::Delay::new(deadline.saturating_duration_since((self.now)()) / 8).fuse();

		let pending_iterator = select! {
			res = t1 => res,
			_ = t2 => {
				log::warn!(
					"Timeout fired waiting for transaction pool at block #{}. \
					Proceeding with production.",
					self.parent_number,
				);
				self.transaction_pool.ready()
			},
		};

		debug!("Attempting to push transactions from the pool.");
		debug!("Pool status: {:?}", self.transaction_pool.status());
		for pending_tx in pending_iterator {
			if (self.now)() > deadline {
				debug!(
					"Consensus deadline reached when pushing block transactions, \
					proceeding with proposing."
				);
				break;
			}

			let pending_tx_data = pending_tx.data().clone();
			let pending_tx_hash = pending_tx.hash().clone();
			trace!("[{:?}] Pushing to the block.", pending_tx_hash);
			match sc_block_builder::BlockBuilder::push(&mut block_builder, pending_tx_data) {
				Ok(()) => {
					debug!("[{:?}] Pushed to the block.", pending_tx_hash);
				}
				Err(ApplyExtrinsicFailed(Validity(e)))
						if e.exhausted_resources() => {
					if skipped < MAX_SKIPPED_TRANSACTIONS {
						skipped += 1;
						debug!(
							"Block seems full, but will try {} more transactions before quitting.",
							MAX_SKIPPED_TRANSACTIONS - skipped,
						);
					} else {
						debug!("Block is full, proceed with proposing.");
						break;
					}
				}
				Err(e) if skipped > 0 => {
					trace!(
						"[{:?}] Ignoring invalid transaction when skipping: {}",
						pending_tx_hash,
						e
					);
				}
				Err(e) => {
					debug!("[{:?}] Invalid transaction: {}", pending_tx_hash, e);
					unqueue_invalid.push(pending_tx_hash);
				}
			}
		}

		self.transaction_pool.remove_invalid(&unqueue_invalid);

		let (block, storage_changes, proof) = block_builder.build()?.into_inner();

		self.metrics.report(
			|metrics| {
				metrics.number_of_transactions.set(block.extrinsics().len() as u64);
				metrics.block_constructed.observe(block_timer.elapsed().as_secs_f64());
			}
		);

		info!("🎁 Prepared block for proposing at {} [hash: {:?}; parent_hash: {}; extrinsics ({}): [{}]]",
			block.header().number(),
			<Block as BlockT>::Hash::from(block.header().hash()),
			block.header().parent_hash(),
			block.extrinsics().len(),
			block.extrinsics()
				.iter()
				.map(|xt| format!("{}", BlakeTwo256::hash_of(xt)))
				.collect::<Vec<_>>()
				.join(", ")
		);
		telemetry!(
			self.telemetry;
			CONSENSUS_INFO;
			"prepared_block_for_proposing";
			"number" => ?block.header().number(),
			"hash" => ?<Block as BlockT>::Hash::from(block.header().hash()),
		);

		if Decode::decode(&mut block.encode().as_slice()).as_ref() != Ok(&block) {
			error!("Failed to verify block encoding/decoding");
		}

		if let Err(err) = evaluation::evaluate_initial(
			&block,
			&self.parent_hash,
			self.parent_number,
			self.max_block_size,
		) {
			error!("Failed to evaluate authored block: {:?}", err);
		}

		let proof = PR::into_proof(proof)
			.map_err(|e| sp_blockchain::Error::Application(Box::new(e)))?;
		Ok(Proposal { block, proof, storage_changes })
	}
}
