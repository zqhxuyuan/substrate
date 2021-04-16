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

//! Slots functionality for Substrate.
//!
//! Some consensus algorithms have a concept of *slots*, which are intervals in
//! time during which certain events can and/or must occur.  This crate
//! provides generic functionality for slots.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod slots;
mod aux_schema;

pub use slots::SlotInfo;
use slots::Slots;
pub use aux_schema::{check_equivocation, MAX_SLOT_CAPACITY, PRUNING_BOUND};

use std::{fmt::Debug, ops::Deref, time::Duration};
use codec::{Decode, Encode};
use futures::{future::Either, Future, TryFutureExt};
use futures_timer::Delay;
use log::{debug, error, info, warn};
use sp_api::{ProvideRuntimeApi, ApiRef};
use sp_arithmetic::traits::BaseArithmetic;
use sp_consensus::{BlockImport, Proposer, SyncOracle, SelectChain, CanAuthorWith, SlotData};
use sp_consensus_slots::Slot;
use sp_inherents::{InherentData, InherentDataProviders};
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, Header, HashFor, NumberFor}
};
use sc_telemetry::{telemetry, TelemetryHandle, CONSENSUS_DEBUG, CONSENSUS_WARN, CONSENSUS_INFO};

/// The changes that need to applied to the storage to create the state for a block.
///
/// See [`sp_state_machine::StorageChanges`] for more information.
pub type StorageChanges<Transaction, Block> =
	sp_state_machine::StorageChanges<Transaction, HashFor<Block>, NumberFor<Block>>;

/// The result of [`SlotWorker::on_slot`].
#[derive(Debug, Clone)]
pub struct SlotResult<Block: BlockT, Proof> {
	/// The block that was built.
	pub block: Block,
	/// The storage proof that was recorded while building the block.
	pub storage_proof: Proof,
}

/// A worker that should be invoked at every new slot.
///
/// The implementation should not make any assumptions of the slot being bound to the time or
/// similar. The only valid assumption is that the slot number is always increasing.
#[async_trait::async_trait]
pub trait SlotWorker<B: BlockT, Proof> {
	/// Called when a new slot is triggered.
	///
	/// Returns a future that resolves to a [`SlotResult`] iff a block was successfully built in
	/// the slot. Otherwise `None` is returned.
	async fn on_slot(
		&mut self,
		chain_head: B::Header,
		slot_info: SlotInfo,
	) -> Option<SlotResult<B, Proof>>;
}

/// A skeleton implementation for `SlotWorker` which tries to claim a slot at
/// its beginning and tries to produce a block if successfully claimed, timing
/// out if block production takes too long.
#[async_trait::async_trait]
pub trait SimpleSlotWorker<B: BlockT> {
	/// A handle to a `BlockImport`.
	type BlockImport: BlockImport<B, Transaction = <Self::Proposer as Proposer<B>>::Transaction>
		+ Send + 'static;

	/// A handle to a `SyncOracle`.
	type SyncOracle: SyncOracle;

	/// The type of future resolving to the proposer.
	type CreateProposer: Future<Output = Result<Self::Proposer, sp_consensus::Error>>
		+ Send + Unpin + 'static;

	/// The type of proposer to use to build blocks.
	type Proposer: Proposer<B> + Send;

	/// Data associated with a slot claim.
	type Claim: Send + 'static;

	/// Epoch data necessary for authoring.
	type EpochData: Send + 'static;

	/// The logging target to use when logging messages.
	fn logging_target(&self) -> &'static str;

	/// A handle to a `BlockImport`.
	fn block_import(&mut self) -> &mut Self::BlockImport;

	/// Returns the epoch data necessary for authoring. For time-dependent epochs,
	/// use the provided slot number as a canonical source of time.
	fn epoch_data(
		&self,
		header: &B::Header,
		slot: Slot,
	) -> Result<Self::EpochData, sp_consensus::Error>;

	/// Returns the number of authorities given the epoch data.
	/// None indicate that the authorities information is incomplete.
	fn authorities_len(&self, epoch_data: &Self::EpochData) -> Option<usize>;

	/// Tries to claim the given slot, returning an object with claim data if successful.
	fn claim_slot(
		&self,
		header: &B::Header,
		slot: Slot,
		epoch_data: &Self::EpochData,
	) -> Option<Self::Claim>;

	/// Notifies the given slot. Similar to `claim_slot`, but will be called no matter whether we
	/// need to author blocks or not.
	fn notify_slot(
		&self,
		_header: &B::Header,
		_slot: Slot,
		_epoch_data: &Self::EpochData,
	) {}

	/// Return the pre digest data to include in a block authored with the given claim.
	fn pre_digest_data(
		&self,
		slot: Slot,
		claim: &Self::Claim,
	) -> Vec<sp_runtime::DigestItem<B::Hash>>;

	/// Returns a function which produces a `BlockImportParams`.
	fn block_import_params(&self) -> Box<
		dyn Fn(
			B::Header,
			&B::Hash,
			Vec<B::Extrinsic>,
			StorageChanges<<Self::BlockImport as BlockImport<B>>::Transaction, B>,
			Self::Claim,
			Self::EpochData,
		) -> Result<
				sp_consensus::BlockImportParams<B, <Self::BlockImport as BlockImport<B>>::Transaction>,
				sp_consensus::Error
			> + Send + 'static
	>;

	/// Whether to force authoring if offline.
	fn force_authoring(&self) -> bool;

	/// Returns whether the block production should back off.
	///
	/// By default this function always returns `false`.
	///
	/// An example strategy that back offs if the finalized head is lagging too much behind the tip
	/// is implemented by [`BackoffAuthoringOnFinalizedHeadLagging`].
	fn should_backoff(&self, _slot: Slot, _chain_head: &B::Header) -> bool {
		false
	}

	/// Returns a handle to a `SyncOracle`.
	fn sync_oracle(&mut self) -> &mut Self::SyncOracle;

	/// Returns a `Proposer` to author on top of the given block.
	fn proposer(&mut self, block: &B::Header) -> Self::CreateProposer;

	/// Returns a [`TelemetryHandle`] if any.
	fn telemetry(&self) -> Option<TelemetryHandle>;

	/// Remaining duration for proposing.
	fn proposing_remaining_duration(
		&self,
		head: &B::Header,
		slot_info: &SlotInfo,
	) -> Duration;

	/// Implements [`SlotWorker::on_slot`].
	async fn on_slot(
		&mut self,
		chain_head: B::Header,
		slot_info: SlotInfo,
	) -> Option<SlotResult<B, <Self::Proposer as Proposer<B>>::Proof>> {
		let (timestamp, slot) = (slot_info.timestamp, slot_info.slot);
		let telemetry = self.telemetry();
		let logging_target = self.logging_target();

		let proposing_remaining_duration = self.proposing_remaining_duration(&chain_head, &slot_info);

		let proposing_remaining = if proposing_remaining_duration == Duration::default() {
			debug!(
				target: logging_target,
				"Skipping proposal slot {} since there's no time left to propose",
				slot,
			);

			return None
		} else {
			Delay::new(proposing_remaining_duration)
		};

		let epoch_data = match self.epoch_data(&chain_head, slot) {
			Ok(epoch_data) => epoch_data,
			Err(err) => {
				warn!(
					target: logging_target,
					"Unable to fetch epoch data at block {:?}: {:?}",
					chain_head.hash(),
					err,
				);

				telemetry!(
					telemetry;
					CONSENSUS_WARN;
					"slots.unable_fetching_authorities";
					"slot" => ?chain_head.hash(),
					"err" => ?err,
				);

				return None;
			}
		};

		self.notify_slot(&chain_head, slot, &epoch_data);

		let authorities_len = self.authorities_len(&epoch_data);

		if !self.force_authoring() &&
			self.sync_oracle().is_offline() &&
			authorities_len.map(|a| a > 1).unwrap_or(false)
		{
			debug!(target: logging_target, "Skipping proposal slot. Waiting for the network.");
			telemetry!(
				telemetry;
				CONSENSUS_DEBUG;
				"slots.skipping_proposal_slot";
				"authorities_len" => authorities_len,
			);

			return None;
		}

		let claim = match self.claim_slot(&chain_head, slot, &epoch_data) {
			None => return None,
			Some(claim) => claim,
		};

		if self.should_backoff(slot, &chain_head) {
			return None;
		}

		debug!(
			target: self.logging_target(),
			"Starting authorship at slot {}; timestamp = {}",
			slot,
			timestamp,
		);

		telemetry!(
			telemetry;
			CONSENSUS_DEBUG;
			"slots.starting_authorship";
			"slot_num" => *slot,
			"timestamp" => *timestamp,
		);

		let proposer = match self.proposer(&chain_head).await {
			Ok(p) => p,
			Err(err) => {
				warn!(
					target: logging_target,
					"Unable to author block in slot {:?}: {:?}",
					slot,
					err,
				);

				telemetry!(
					telemetry;
					CONSENSUS_WARN;
					"slots.unable_authoring_block";
					"slot" => *slot,
					"err" => ?err
				);

				return None
			}
		};

		let logs = self.pre_digest_data(slot, &claim);

		// deadline our production to 98% of the total time left for proposing. As we deadline
		// the proposing below to the same total time left, the 2% margin should be enough for
		// the result to be returned.
		let proposing = proposer.propose(
			slot_info.inherent_data,
			sp_runtime::generic::Digest {
				logs,
			},
			proposing_remaining_duration.mul_f32(0.98),
		).map_err(|e| sp_consensus::Error::ClientImport(format!("{:?}", e)));

		let proposal = match futures::future::select(proposing, proposing_remaining).await {
			Either::Left((Ok(p), _)) => p,
			Either::Left((Err(err), _)) => {
				warn!(
					target: logging_target,
					"Proposing failed: {:?}",
					err,
				);

				return None
			},
			Either::Right(_) => {
				info!(
					target: logging_target,
					"⌛️ Discarding proposal for slot {}; block production took too long",
					slot,
				);
				// If the node was compiled with debug, tell the user to use release optimizations.
				#[cfg(build_type="debug")]
				info!(
					target: logging_target,
					"👉 Recompile your node in `--release` mode to mitigate this problem.",
				);
				telemetry!(
					telemetry;
					CONSENSUS_INFO;
					"slots.discarding_proposal_took_too_long";
					"slot" => *slot,
				);

				return None
			},
		};

		let block_import_params_maker = self.block_import_params();
		let block_import = self.block_import();

		let (block, storage_proof) = (proposal.block, proposal.proof);
		let (header, body) = block.deconstruct();
		let header_num = *header.number();
		let header_hash = header.hash();
		let parent_hash = *header.parent_hash();

		let block_import_params = match block_import_params_maker(
			header,
			&header_hash,
			body.clone(),
			proposal.storage_changes,
			claim,
			epoch_data,
		) {
			Ok(bi) => bi,
			Err(err) => {
				warn!(
					target: logging_target,
					"Failed to create block import params: {:?}",
					err,
				);

				return None
			}
		};

		info!(
			target: logging_target,
			"🔖 Pre-sealed block for proposal at {}. Hash now {:?}, previously {:?}.",
			header_num,
			block_import_params.post_hash(),
			header_hash,
		);

		telemetry!(
			telemetry;
			CONSENSUS_INFO;
			"slots.pre_sealed_block";
			"header_num" => ?header_num,
			"hash_now" => ?block_import_params.post_hash(),
			"hash_previously" => ?header_hash,
		);

		let header = block_import_params.post_header();
		if let Err(err) = block_import
			.import_block(block_import_params, Default::default())
			.await
		{
			warn!(
				target: logging_target,
				"Error with block built on {:?}: {:?}",
				parent_hash,
				err,
			);

			telemetry!(
				telemetry;
				CONSENSUS_WARN;
				"slots.err_with_block_built_on";
				"hash" => ?parent_hash,
				"err" => ?err,
			);
		}

		Some(SlotResult { block: B::new(header, body), storage_proof })
	}
}

#[async_trait::async_trait]
impl<B: BlockT, T: SimpleSlotWorker<B> + Send> SlotWorker<B, <T::Proposer as Proposer<B>>::Proof> for T {
	async fn on_slot(
		&mut self,
		chain_head: B::Header,
		slot_info: SlotInfo,
	) -> Option<SlotResult<B, <T::Proposer as Proposer<B>>::Proof>> {
		SimpleSlotWorker::on_slot(self, chain_head, slot_info).await
	}
}

/// Slot compatible inherent data.
pub trait SlotCompatible {
	/// Extract timestamp and slot from inherent data.
	fn extract_timestamp_and_slot(
		&self,
		inherent: &InherentData,
	) -> Result<(sp_timestamp::Timestamp, Slot, std::time::Duration), sp_consensus::Error>;
}

/// Start a new slot worker.
///
/// Every time a new slot is triggered, `worker.on_slot` is called and the future it returns is
/// polled until completion, unless we are major syncing.
pub fn start_slot_worker<B, C, W, T, SO, SC, CAW, Proof>(
	slot_duration: SlotDuration<T>,
	client: C,
	mut worker: W,
	mut sync_oracle: SO,
	inherent_data_providers: InherentDataProviders,
	timestamp_extractor: SC,
	can_author_with: CAW,
) -> impl Future<Output = ()>
where
	B: BlockT,
	C: SelectChain<B>,
	W: SlotWorker<B, Proof>,
	SO: SyncOracle + Send,
	SC: SlotCompatible + Unpin,
	T: SlotData + Clone,
	CAW: CanAuthorWith<B> + Send,
{
	let SlotDuration(slot_duration) = slot_duration;

	// rather than use a timer interval, we schedule our waits ourselves
	let mut slots = Slots::<SC>::new(
		slot_duration.slot_duration(),
		inherent_data_providers,
		timestamp_extractor,
	);

	async move {
		loop {
			let slot_info = match slots.next_slot().await {
				Ok(slot) => slot,
				Err(err) => {
					debug!(target: "slots", "Faulty timer: {:?}", err);
					return
				},
			};

			// only propose when we are not syncing.
			if sync_oracle.is_major_syncing() {
				debug!(target: "slots", "Skipping proposal slot due to sync.");
				continue;
			}

			let slot = slot_info.slot;
			let chain_head = match client.best_chain() {
				Ok(x) => x,
				Err(e) => {
					warn!(
						target: "slots",
						"Unable to author block in slot {}. No best block header: {:?}",
						slot,
						e,
					);
					continue;
				}
			};

			if let Err(err) = can_author_with.can_author_with(&BlockId::Hash(chain_head.hash())) {
				warn!(
					target: "slots",
					"Unable to author block in slot {},. `can_author_with` returned: {} \
					Probably a node update is required!",
					slot,
					err,
				);
			} else {
				worker.on_slot(chain_head, slot_info).await;
			}
		}
	}
}

/// A header which has been checked
pub enum CheckedHeader<H, S> {
	/// A header which has slot in the future. this is the full header (not stripped)
	/// and the slot in which it should be processed.
	Deferred(H, Slot),
	/// A header which is fully checked, including signature. This is the pre-header
	/// accompanied by the seal components.
	///
	/// Includes the digest item that encoded the seal.
	Checked(H, S),
}

#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error<T> where T: Debug {
	#[error("Slot duration is invalid: {0:?}")]
	SlotDurationInvalid(SlotDuration<T>),
}

/// A slot duration. Create with `get_or_compute`.
// The internal member should stay private here to maintain invariants of
// `get_or_compute`.
#[derive(Clone, Copy, Debug, Encode, Decode, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct SlotDuration<T>(T);

impl<T> Deref for SlotDuration<T> {
	type Target = T;
	fn deref(&self) -> &T {
		&self.0
	}
}

impl<T: SlotData> SlotData for SlotDuration<T> {
	fn slot_duration(&self) -> std::time::Duration {
		self.0.slot_duration()
	}

	const SLOT_KEY: &'static [u8] = T::SLOT_KEY;
}

impl<T: Clone + Send + Sync + 'static> SlotDuration<T> {
	/// Either fetch the slot duration from disk or compute it from the
	/// genesis state.
	///
	/// `slot_key` is marked as `'static`, as it should really be a
	/// compile-time constant.
	pub fn get_or_compute<B: BlockT, C, CB>(client: &C, cb: CB) -> sp_blockchain::Result<Self> where
		C: sc_client_api::backend::AuxStore,
		C: ProvideRuntimeApi<B>,
		CB: FnOnce(ApiRef<C::Api>, &BlockId<B>) -> sp_blockchain::Result<T>,
		T: SlotData + Encode + Decode + Debug,
	{
		let slot_duration = match client.get_aux(T::SLOT_KEY)? {
			Some(v) => <T as codec::Decode>::decode(&mut &v[..])
				.map(SlotDuration)
				.map_err(|_| {
					sp_blockchain::Error::Backend({
						error!(target: "slots", "slot duration kept in invalid format");
						"slot duration kept in invalid format".to_string()
					})
				}),
			None => {
				use sp_runtime::traits::Zero;
				let genesis_slot_duration =
					cb(client.runtime_api(), &BlockId::number(Zero::zero()))?;

				info!(
					"⏱  Loaded block-time = {:?} milliseconds from genesis on first-launch",
					genesis_slot_duration.slot_duration()
				);

				genesis_slot_duration
					.using_encoded(|s| client.insert_aux(&[(T::SLOT_KEY, &s[..])], &[]))?;

				Ok(SlotDuration(genesis_slot_duration))
			}
		}?;

		if slot_duration.slot_duration() == Default::default() {
			return Err(sp_blockchain::Error::Application(Box::new(Error::SlotDurationInvalid(slot_duration))))
		}

		Ok(slot_duration)
	}

	/// Returns slot data value.
	pub fn get(&self) -> T {
		self.0.clone()
	}
}

/// A unit type wrapper to express the proportion of a slot.
pub struct SlotProportion(f32);

impl SlotProportion {
	/// Create a new proportion.
	///
	/// The given value `inner` should be in the range `[0,1]`. If the value is not in the required
	/// range, it is clamped into the range.
	pub fn new(inner: f32) -> Self {
		Self(inner.clamp(0.0, 1.0))
	}

	/// Returns the inner that is guaranted to be in the range `[0,1]`.
	pub fn get(&self) -> f32 {
		self.0
	}
}

/// Calculate a slot duration lenience based on the number of missed slots from current
/// to parent. If the number of skipped slots is greated than 0 this method will apply
/// an exponential backoff of at most `2^7 * slot_duration`, if no slots were skipped
/// this method will return `None.`
pub fn slot_lenience_exponential(parent_slot: Slot, slot_info: &SlotInfo) -> Option<Duration> {
	// never give more than 2^this times the lenience.
	const BACKOFF_CAP: u64 = 7;

	// how many slots it takes before we double the lenience.
	const BACKOFF_STEP: u64 = 2;

	// we allow a lenience of the number of slots since the head of the
	// chain was produced, minus 1 (since there is always a difference of at least 1)
	//
	// exponential back-off.
	// in normal cases we only attempt to issue blocks up to the end of the slot.
	// when the chain has been stalled for a few slots, we give more lenience.
	let skipped_slots = *slot_info.slot.saturating_sub(parent_slot + 1);

	if skipped_slots == 0 {
		None
	} else {
		let slot_lenience = skipped_slots / BACKOFF_STEP;
		let slot_lenience = std::cmp::min(slot_lenience, BACKOFF_CAP);
		let slot_lenience = 1 << slot_lenience;
		Some(slot_lenience * slot_info.duration)
	}
}

/// Calculate a slot duration lenience based on the number of missed slots from current
/// to parent. If the number of skipped slots is greated than 0 this method will apply
/// a linear backoff of at most `20 * slot_duration`, if no slots were skipped
/// this method will return `None.`
pub fn slot_lenience_linear(parent_slot: Slot, slot_info: &SlotInfo) -> Option<Duration> {
	// never give more than 20 times more lenience.
	const BACKOFF_CAP: u64 = 20;

	// we allow a lenience of the number of slots since the head of the
	// chain was produced, minus 1 (since there is always a difference of at least 1)
	//
	// linear back-off.
	// in normal cases we only attempt to issue blocks up to the end of the slot.
	// when the chain has been stalled for a few slots, we give more lenience.
	let skipped_slots = *slot_info.slot.saturating_sub(parent_slot + 1);

	if skipped_slots == 0 {
		None
	} else {
		let slot_lenience = std::cmp::min(skipped_slots, BACKOFF_CAP);
		// We cap `slot_lenience` to `20`, so it should always fit into an `u32`.
		Some(slot_info.duration * (slot_lenience as u32))
	}
}

/// Trait for providing the strategy for when to backoff block authoring.
pub trait BackoffAuthoringBlocksStrategy<N> {
	/// Returns true if we should backoff authoring new blocks.
	fn should_backoff(
		&self,
		chain_head_number: N,
		chain_head_slot: Slot,
		finalized_number: N,
		slow_now: Slot,
		logging_target: &str,
	) -> bool;
}

/// A simple default strategy for how to decide backing off authoring blocks if the number of
/// unfinalized blocks grows too large.
#[derive(Clone)]
pub struct BackoffAuthoringOnFinalizedHeadLagging<N> {
	/// The max interval to backoff when authoring blocks, regardless of delay in finality.
	pub max_interval: N,
	/// The number of unfinalized blocks allowed before starting to consider to backoff authoring
	/// blocks. Note that depending on the value for `authoring_bias`, there might still be an
	/// additional wait until block authorship starts getting declined.
	pub unfinalized_slack: N,
	/// Scales the backoff rate. A higher value effectively means we backoff slower, taking longer
	/// time to reach the maximum backoff as the unfinalized head of chain grows.
	pub authoring_bias: N,
}

/// These parameters is supposed to be some form of sensible defaults.
impl<N: BaseArithmetic> Default for BackoffAuthoringOnFinalizedHeadLagging<N> {
	fn default() -> Self {
		Self {
			// Never wait more than 100 slots before authoring blocks, regardless of delay in
			// finality.
			max_interval: 100.into(),
			// Start to consider backing off block authorship once we have 50 or more unfinalized
			// blocks at the head of the chain.
			unfinalized_slack: 50.into(),
			// A reasonable default for the authoring bias, or reciprocal interval scaling, is 2.
			// Effectively meaning that consider the unfinalized head suffix length to grow half as
			// fast as in actuality.
			authoring_bias: 2.into(),
		}
	}
}

impl<N> BackoffAuthoringBlocksStrategy<N> for BackoffAuthoringOnFinalizedHeadLagging<N>
where
	N: BaseArithmetic + Copy
{
	fn should_backoff(
		&self,
		chain_head_number: N,
		chain_head_slot: Slot,
		finalized_number: N,
		slot_now: Slot,
		logging_target: &str,
	) -> bool {
		// This should not happen, but we want to keep the previous behaviour if it does.
		if slot_now <= chain_head_slot {
			return false;
		}

		let unfinalized_block_length = chain_head_number - finalized_number;
		let interval = unfinalized_block_length.saturating_sub(self.unfinalized_slack)
			/ self.authoring_bias;
		let interval = interval.min(self.max_interval);

		// We're doing arithmetic between block and slot numbers.
		let interval: u64 = interval.unique_saturated_into();

		// If interval is nonzero we backoff if the current slot isn't far enough ahead of the chain
		// head.
		if *slot_now <= *chain_head_slot + interval {
			info!(
				target: logging_target,
				"Backing off claiming new slot for block authorship: finality is lagging.",
			);
			true
		} else {
			false
		}
	}
}

impl<N> BackoffAuthoringBlocksStrategy<N> for () {
	fn should_backoff(
		&self,
		_chain_head_number: N,
		_chain_head_slot: Slot,
		_finalized_number: N,
		_slot_now: Slot,
		_logging_target: &str,
	) -> bool {
		false
	}
}


