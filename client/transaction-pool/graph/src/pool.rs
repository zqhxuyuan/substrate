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

use std::{
	collections::HashMap,
	sync::Arc,
};

use crate::{base_pool as base, watcher::Watcher};

use futures::Future;
use sp_runtime::{
	generic::BlockId,
	traits::{self, SaturatedConversion, Block as BlockT},
	transaction_validity::{
		TransactionValidity, TransactionTag as Tag, TransactionValidityError, TransactionSource,
	},
};
use sp_transaction_pool::error;
use wasm_timer::Instant;
use futures::channel::mpsc::Receiver;

use crate::validated_pool::ValidatedPool;
pub use crate::validated_pool::{IsValidator, ValidatedTransaction};

/// Modification notification event stream type;
pub type EventStream<H> = Receiver<H>;

/// Block hash type for a pool.
pub type BlockHash<A> = <<A as ChainApi>::Block as traits::Block>::Hash;
/// Extrinsic hash type for a pool.
pub type ExtrinsicHash<A> = <<A as ChainApi>::Block as traits::Block>::Hash;
/// Extrinsic type for a pool.
pub type ExtrinsicFor<A> = <<A as ChainApi>::Block as traits::Block>::Extrinsic;
/// Block number type for the ChainApi
pub type NumberFor<A> = traits::NumberFor<<A as ChainApi>::Block>;
/// A type of transaction stored in the pool
pub type TransactionFor<A> = Arc<base::Transaction<ExtrinsicHash<A>, ExtrinsicFor<A>>>;
/// A type of validated transaction stored in the pool.
pub type ValidatedTransactionFor<A> = ValidatedTransaction<
	ExtrinsicHash<A>,
	ExtrinsicFor<A>,
	<A as ChainApi>::Error,
>;

/// Concrete extrinsic validation and query logic.
pub trait ChainApi: Send + Sync {
	/// Block type.
	type Block: BlockT;
	/// Error type.
	type Error: From<error::Error> + error::IntoPoolError;
	/// Validate transaction future.
	type ValidationFuture: Future<Output=Result<TransactionValidity, Self::Error>> + Send + Unpin;
	/// Body future (since block body might be remote)
	type BodyFuture: Future<
		Output = Result<Option<Vec<<Self::Block as traits::Block>::Extrinsic>>, Self::Error>
	> + Unpin + Send + 'static;

	/// Verify extrinsic at given block.
	fn validate_transaction(
		&self,
		at: &BlockId<Self::Block>,
		source: TransactionSource,
		uxt: ExtrinsicFor<Self>,
	) -> Self::ValidationFuture;

	/// Returns a block number given the block id.
	fn block_id_to_number(
		&self,
		at: &BlockId<Self::Block>,
	) -> Result<Option<NumberFor<Self>>, Self::Error>;

	/// Returns a block hash given the block id.
	fn block_id_to_hash(
		&self,
		at: &BlockId<Self::Block>,
	) -> Result<Option<<Self::Block as BlockT>::Hash>, Self::Error>;

	/// Returns hash and encoding length of the extrinsic.
	fn hash_and_length(&self, uxt: &ExtrinsicFor<Self>) -> (ExtrinsicHash<Self>, usize);

	/// Returns a block body given the block id.
	fn block_body(&self, at: &BlockId<Self::Block>) -> Self::BodyFuture;
}

/// Pool configuration options.
#[derive(Debug, Clone)]
pub struct Options {
	/// Ready queue limits.
	pub ready: base::Limit,
	/// Future queue limits.
	pub future: base::Limit,
	/// Reject future transactions.
	pub reject_future_transactions: bool,
}

impl Default for Options {
	fn default() -> Self {
		Options {
			ready: base::Limit {
				count: 8192,
				total_bytes: 20 * 1024 * 1024,
			},
			future: base::Limit {
				count: 512,
				total_bytes: 1 * 1024 * 1024,
			},
			reject_future_transactions: false,
		}
	}
}

/// Should we check that the transaction is banned
/// in the pool, before we verify it?
#[derive(Copy, Clone)]
enum CheckBannedBeforeVerify {
	Yes,
	No,
}

/// Extrinsics pool that performs validation.
pub struct Pool<B: ChainApi> {
	validated_pool: Arc<ValidatedPool<B>>,
}

#[cfg(not(target_os = "unknown"))]
impl<B: ChainApi> parity_util_mem::MallocSizeOf for Pool<B>
where
	ExtrinsicFor<B>: parity_util_mem::MallocSizeOf,
{
	fn size_of(&self, ops: &mut parity_util_mem::MallocSizeOfOps) -> usize {
		self.validated_pool.size_of(ops)
	}
}

impl<B: ChainApi> Pool<B> {
	/// Create a new transaction pool.
	pub fn new(options: Options, is_validator: IsValidator, api: Arc<B>) -> Self {
		Pool {
			validated_pool: Arc::new(ValidatedPool::new(options, is_validator, api)),
		}
	}

	/// Imports a bunch of unverified extrinsics to the pool
	pub async fn submit_at(
		&self,
		at: &BlockId<B::Block>,
		source: TransactionSource,
		xts: impl IntoIterator<Item=ExtrinsicFor<B>>,
	) -> Result<Vec<Result<ExtrinsicHash<B>, B::Error>>, B::Error> {
		let xts = xts.into_iter().map(|xt| (source, xt));
		let validated_transactions = self.verify(at, xts, CheckBannedBeforeVerify::Yes).await?;
		Ok(self.validated_pool.submit(validated_transactions.into_iter().map(|(_, tx)| tx)))
	}

	/// Resubmit the given extrinsics to the pool.
	///
	/// This does not check if a transaction is banned, before we verify it again.
	pub async fn resubmit_at(
		&self,
		at: &BlockId<B::Block>,
		source: TransactionSource,
		xts: impl IntoIterator<Item=ExtrinsicFor<B>>,
	) -> Result<Vec<Result<ExtrinsicHash<B>, B::Error>>, B::Error> {
		let xts = xts.into_iter().map(|xt| (source, xt));
		let validated_transactions = self.verify(at, xts, CheckBannedBeforeVerify::No).await?;
		Ok(self.validated_pool.submit(validated_transactions.into_iter().map(|(_, tx)| tx)))
	}

	/// Imports one unverified extrinsic to the pool
	pub async fn submit_one(
		&self,
		at: &BlockId<B::Block>,
		source: TransactionSource,
		xt: ExtrinsicFor<B>,
	) -> Result<ExtrinsicHash<B>, B::Error> {
		let res = self.submit_at(at, source, std::iter::once(xt)).await?.pop();
		res.expect("One extrinsic passed; one result returned; qed")
	}

	/// Import a single extrinsic and starts to watch their progress in the pool.
	pub async fn submit_and_watch(
		&self,
		at: &BlockId<B::Block>,
		source: TransactionSource,
		xt: ExtrinsicFor<B>,
	) -> Result<Watcher<ExtrinsicHash<B>, ExtrinsicHash<B>>, B::Error> {
		let block_number = self.resolve_block_number(at)?;
		let (_, tx) = self.verify_one(
			at,
			block_number,
			source,
			xt,
			CheckBannedBeforeVerify::Yes,
		).await;
		self.validated_pool.submit_and_watch(tx)
	}

	/// Resubmit some transaction that were validated elsewhere.
	pub fn resubmit(
		&self,
		revalidated_transactions: HashMap<ExtrinsicHash<B>, ValidatedTransactionFor<B>>,
	) {

		let now = Instant::now();
		self.validated_pool.resubmit(revalidated_transactions);
		log::debug!(target: "txpool",
			"Resubmitted. Took {} ms. Status: {:?}",
			now.elapsed().as_millis(),
			self.validated_pool.status()
		);
	}

	/// Prunes known ready transactions.
	///
	/// Used to clear the pool from transactions that were part of recently imported block.
	/// The main difference from the `prune` is that we do not revalidate any transactions
	/// and ignore unknown passed hashes.
	pub fn prune_known(
		&self,
		at: &BlockId<B::Block>,
		hashes: &[ExtrinsicHash<B>],
	) -> Result<(), B::Error> {
		// Get details of all extrinsics that are already in the pool
		let in_pool_tags = self.validated_pool.extrinsics_tags(hashes)
			.into_iter().filter_map(|x| x).flat_map(|x| x);

		// Prune all transactions that provide given tags
		let prune_status = self.validated_pool.prune_tags(in_pool_tags)?;
		let pruned_transactions = hashes.into_iter().cloned()
			.chain(prune_status.pruned.iter().map(|tx| tx.hash.clone()));
		self.validated_pool.fire_pruned(at, pruned_transactions)
	}

	/// Prunes ready transactions.
	///
	/// Used to clear the pool from transactions that were part of recently imported block.
	/// To perform pruning we need the tags that each extrinsic provides and to avoid calling
	/// into runtime too often we first lookup all extrinsics that are in the pool and get
	/// their provided tags from there. Otherwise we query the runtime at the `parent` block.
	pub async fn prune(
		&self,
		at: &BlockId<B::Block>,
		parent: &BlockId<B::Block>,
		extrinsics: &[ExtrinsicFor<B>],
	) -> Result<(), B::Error> {
		log::debug!(
			target: "txpool",
			"Starting pruning of block {:?} (extrinsics: {})",
			at,
			extrinsics.len()
		);
		// Get details of all extrinsics that are already in the pool
		let in_pool_hashes = extrinsics.iter().map(|extrinsic| self.hash_of(extrinsic)).collect::<Vec<_>>();
		let in_pool_tags = self.validated_pool.extrinsics_tags(&in_pool_hashes);

		// Zip the ones from the pool with the full list (we get pairs `(Extrinsic, Option<Vec<Tag>>)`)
		let all = extrinsics.iter().zip(in_pool_tags.into_iter());

		let mut future_tags = Vec::new();
		for (extrinsic, in_pool_tags) in all {
			match in_pool_tags {
				// reuse the tags for extrinsics that were found in the pool
				Some(tags) => future_tags.extend(tags),
				// if it's not found in the pool query the runtime at parent block
				// to get validity info and tags that the extrinsic provides.
				None => {
					let validity = self.validated_pool.api()
						.validate_transaction(parent, TransactionSource::InBlock, extrinsic.clone())
						.await;

					if let Ok(Ok(validity)) = validity {
						future_tags.extend(validity.provides);
					}
				},
			}
		}

		self.prune_tags(at, future_tags, in_pool_hashes).await
	}

	/// Prunes ready transactions that provide given list of tags.
	///
	/// Given tags are assumed to be always provided now, so all transactions
	/// in the Future Queue that require that particular tag (and have other
	/// requirements satisfied) are promoted to Ready Queue.
	///
	/// Moreover for each provided tag we remove transactions in the pool that:
	/// 1. Provide that tag directly
	/// 2. Are a dependency of pruned transaction.
	///
	/// Returns transactions that have been removed from the pool and must be reverified
	/// before reinserting to the pool.
	///
	/// By removing predecessor transactions as well we might actually end up
	/// pruning too much, so all removed transactions are reverified against
	/// the runtime (`validate_transaction`) to make sure they are invalid.
	///
	/// However we avoid revalidating transactions that are contained within
	/// the second parameter of `known_imported_hashes`. These transactions
	/// (if pruned) are not revalidated and become temporarily banned to
	/// prevent importing them in the (near) future.
	pub async fn prune_tags(
		&self,
		at: &BlockId<B::Block>,
		tags: impl IntoIterator<Item=Tag>,
		known_imported_hashes: impl IntoIterator<Item=ExtrinsicHash<B>> + Clone,
	) -> Result<(), B::Error> {
		log::debug!(target: "txpool", "Pruning at {:?}", at);
		// Prune all transactions that provide given tags
		let prune_status = match self.validated_pool.prune_tags(tags) {
			Ok(prune_status) => prune_status,
			Err(e) => return Err(e),
		};

		// Make sure that we don't revalidate extrinsics that were part of the recently
		// imported block. This is especially important for UTXO-like chains cause the
		// inputs are pruned so such transaction would go to future again.
		self.validated_pool.ban(&Instant::now(), known_imported_hashes.clone().into_iter());

		// Try to re-validate pruned transactions since some of them might be still valid.
		// note that `known_imported_hashes` will be rejected here due to temporary ban.
		let pruned_hashes = prune_status.pruned
			.iter()
			.map(|tx| tx.hash.clone()).collect::<Vec<_>>();
		let pruned_transactions = prune_status.pruned
			.into_iter()
			.map(|tx| (tx.source, tx.data.clone()));

		let reverified_transactions = self.verify(
			at,
			pruned_transactions,
			CheckBannedBeforeVerify::Yes,
		).await?;

		log::trace!(target: "txpool", "Pruning at {:?}. Resubmitting transactions.", at);
		// And finally - submit reverified transactions back to the pool

		self.validated_pool.resubmit_pruned(
			&at,
			known_imported_hashes,
			pruned_hashes,
			reverified_transactions.into_iter().map(|(_, xt)| xt).collect(),
		)
	}

	/// Returns transaction hash
	pub fn hash_of(&self, xt: &ExtrinsicFor<B>) -> ExtrinsicHash<B> {
		self.validated_pool.api().hash_and_length(xt).0
	}

	/// Resolves block number by id.
	fn resolve_block_number(&self, at: &BlockId<B::Block>) -> Result<NumberFor<B>, B::Error> {
		self.validated_pool.api().block_id_to_number(at)
			.and_then(|number| number.ok_or_else(||
				error::Error::InvalidBlockId(format!("{:?}", at)).into()))
	}

	/// Returns future that validates a bunch of transactions at given block.
	async fn verify(
		&self,
		at: &BlockId<B::Block>,
		xts: impl IntoIterator<Item=(TransactionSource, ExtrinsicFor<B>)>,
		check: CheckBannedBeforeVerify,
	) -> Result<HashMap<ExtrinsicHash<B>, ValidatedTransactionFor<B>>, B::Error> {
		// we need a block number to compute tx validity
		let block_number = self.resolve_block_number(at)?;

		let res = futures::future::join_all(
			xts.into_iter()
				.map(|(source, xt)| self.verify_one(at, block_number, source, xt, check))
		).await.into_iter().collect::<HashMap<_, _>>();

		Ok(res)
	}

	/// Returns future that validates single transaction at given block.
	async fn verify_one(
		&self,
		block_id: &BlockId<B::Block>,
		block_number: NumberFor<B>,
		source: TransactionSource,
		xt: ExtrinsicFor<B>,
		check: CheckBannedBeforeVerify,
	) -> (ExtrinsicHash<B>, ValidatedTransactionFor<B>) {
		let (hash, bytes) = self.validated_pool.api().hash_and_length(&xt);

		let ignore_banned = matches!(check, CheckBannedBeforeVerify::No);
		if let Err(err) = self.validated_pool.check_is_known(&hash, ignore_banned) {
			return (hash.clone(), ValidatedTransaction::Invalid(hash, err.into()))
		}

		let validation_result = self.validated_pool.api().validate_transaction(
			block_id,
			source,
			xt.clone(),
		).await;

		let status = match validation_result {
			Ok(status) => status,
			Err(e) => return (hash.clone(), ValidatedTransaction::Invalid(hash, e)),
		};

		let validity = match status {
			Ok(validity) => {
				if validity.provides.is_empty() {
					ValidatedTransaction::Invalid(hash.clone(), error::Error::NoTagsProvided.into())
				} else {
					ValidatedTransaction::valid_at(
						block_number.saturated_into::<u64>(),
						hash.clone(),
						source,
						xt,
						bytes,
						validity,
					)
				}
			},
			Err(TransactionValidityError::Invalid(e)) =>
				ValidatedTransaction::Invalid(hash.clone(), error::Error::InvalidTransaction(e).into()),
			Err(TransactionValidityError::Unknown(e)) =>
				ValidatedTransaction::Unknown(hash.clone(), error::Error::UnknownTransaction(e).into()),
		};

		(hash, validity)
	}

	/// get a reference to the underlying validated pool.
	pub fn validated_pool(&self) ->  &ValidatedPool<B> {
		&self.validated_pool
	}
}

impl<B: ChainApi> Clone for Pool<B> {
	fn clone(&self) -> Self {
		Self {
			validated_pool: self.validated_pool.clone(),
		}
	}
}

