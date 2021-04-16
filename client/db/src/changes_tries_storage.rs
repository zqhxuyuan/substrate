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

//! DB-backed changes tries storage.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use hash_db::Prefix;
use codec::{Decode, Encode};
use parking_lot::RwLock;
use sp_blockchain::{Error as ClientError, Result as ClientResult};
use sp_trie::MemoryDB;
use sc_client_api::backend::PrunableStateChangesTrieStorage;
use sp_blockchain::{well_known_cache_keys, Cache as BlockchainCache, HeaderMetadataCache};
use sp_core::{ChangesTrieConfiguration, ChangesTrieConfigurationRange, convert_hash};
use sp_core::storage::PrefixedStorageKey;
use sp_database::Transaction;
use sp_runtime::traits::{
	Block as BlockT, Header as HeaderT, HashFor, NumberFor, One, Zero, CheckedSub,
};
use sp_runtime::generic::{BlockId, DigestItem, ChangesTrieSignal};
use sp_state_machine::{ChangesTrieBuildCache, ChangesTrieCacheAction};
use crate::{Database, DbHash};
use crate::utils::{self, Meta, meta_keys};
use crate::cache::{
	DbCacheSync, DbCache, DbCacheTransactionOps,
	ComplexBlockId, EntryType as CacheEntryType,
};

/// Extract new changes trie configuration (if available) from the header.
pub fn extract_new_configuration<Header: HeaderT>(header: &Header) -> Option<&Option<ChangesTrieConfiguration>> {
	header.digest()
		.log(DigestItem::as_changes_trie_signal)
		.and_then(ChangesTrieSignal::as_new_configuration)
}

/// Opaque configuration cache transaction. During its lifetime, no-one should modify cache. This is currently
/// guaranteed because import lock is held during block import/finalization.
pub struct DbChangesTrieStorageTransaction<Block: BlockT> {
	/// Cache operations that must be performed after db transaction is committed.
	cache_ops: DbCacheTransactionOps<Block>,
	/// New configuration (if changed at current block).
	new_config: Option<Option<ChangesTrieConfiguration>>,
}

impl<Block: BlockT> DbChangesTrieStorageTransaction<Block> {
	/// Consume self and return transaction with given new configuration.
	pub fn with_new_config(mut self, new_config: Option<Option<ChangesTrieConfiguration>>) -> Self {
		self.new_config = new_config;
		self
	}
}

impl<Block: BlockT> From<DbCacheTransactionOps<Block>> for DbChangesTrieStorageTransaction<Block> {
	fn from(cache_ops: DbCacheTransactionOps<Block>) -> Self {
		DbChangesTrieStorageTransaction {
			cache_ops,
			new_config: None,
		}
	}
}

/// Changes tries storage.
///
/// Stores all tries in separate DB column.
/// Lock order: meta, tries_meta, cache, build_cache.
pub struct DbChangesTrieStorage<Block: BlockT> {
	db: Arc<dyn Database<DbHash>>,
	meta_column: u32,
	changes_tries_column: u32,
	key_lookup_column: u32,
	header_column: u32,
	meta: Arc<RwLock<Meta<NumberFor<Block>, Block::Hash>>>,
	tries_meta: RwLock<ChangesTriesMeta<Block>>,
	min_blocks_to_keep: Option<u32>,
	/// The cache stores all ever existing changes tries configurations.
	cache: DbCacheSync<Block>,
	/// Build cache is a map of block => set of storage keys changed at this block.
	/// They're used to build digest blocks - instead of reading+parsing tries from db
	/// we just use keys sets from the cache.
	build_cache: RwLock<ChangesTrieBuildCache<Block::Hash, NumberFor<Block>>>,
}

/// Persistent struct that contains all the changes tries metadata.
#[derive(Decode, Encode, Debug)]
struct ChangesTriesMeta<Block: BlockT> {
	/// Oldest unpruned max-level (or skewed) digest trie blocks range.
	/// The range is inclusive from both sides.
	/// Is None only if:
	/// 1) we haven't yet finalized any blocks (except genesis)
	/// 2) if best_finalized_block - min_blocks_to_keep points to the range where changes tries are disabled
	/// 3) changes tries pruning is disabled
	pub oldest_digest_range: Option<(NumberFor<Block>, NumberFor<Block>)>,
	/// End block (inclusive) of oldest pruned max-level (or skewed) digest trie blocks range.
	/// It is guaranteed that we have no any changes tries before (and including) this block.
	/// It is guaranteed that all existing changes tries after this block are not yet pruned (if created).
	pub oldest_pruned_digest_range_end: NumberFor<Block>,
}

impl<Block: BlockT> DbChangesTrieStorage<Block> {
	/// Create new changes trie storage.
	pub fn new(
		db: Arc<dyn Database<DbHash>>,
		header_metadata_cache: Arc<HeaderMetadataCache<Block>>,
		meta_column: u32,
		changes_tries_column: u32,
		key_lookup_column: u32,
		header_column: u32,
		cache_column: u32,
		meta: Arc<RwLock<Meta<NumberFor<Block>, Block::Hash>>>,
		min_blocks_to_keep: Option<u32>,
	) -> ClientResult<Self> {
		let (finalized_hash, finalized_number, genesis_hash) = {
			let meta = meta.read();
			(meta.finalized_hash, meta.finalized_number, meta.genesis_hash)
		};
		let tries_meta = read_tries_meta(&*db, meta_column)?;
		Ok(Self {
			db: db.clone(),
			meta_column,
			changes_tries_column,
			key_lookup_column,
			header_column,
			meta,
			min_blocks_to_keep,
			cache: DbCacheSync(RwLock::new(DbCache::new(
				db.clone(),
				header_metadata_cache,
				key_lookup_column,
				header_column,
				cache_column,
				genesis_hash,
				ComplexBlockId::new(finalized_hash, finalized_number),
			))),
			build_cache: RwLock::new(ChangesTrieBuildCache::new()),
			tries_meta: RwLock::new(tries_meta),
		})
	}

	/// Commit new changes trie.
	pub fn commit(
		&self,
		tx: &mut Transaction<DbHash>,
		mut changes_trie: MemoryDB<HashFor<Block>>,
		parent_block: ComplexBlockId<Block>,
		block: ComplexBlockId<Block>,
		new_header: &Block::Header,
		finalized: bool,
		new_configuration: Option<Option<ChangesTrieConfiguration>>,
		cache_tx: Option<DbChangesTrieStorageTransaction<Block>>,
	) -> ClientResult<DbChangesTrieStorageTransaction<Block>> {
		// insert changes trie, associated with block, into DB
		for (key, (val, _)) in changes_trie.drain() {
			tx.set(self.changes_tries_column, key.as_ref(), &val);
		}

		// if configuration has not been changed AND block is not finalized => nothing to do here
		let new_configuration = match new_configuration {
			Some(new_configuration) => new_configuration,
			None if !finalized => return Ok(DbCacheTransactionOps::empty().into()),
			None => return self.finalize(
				tx,
				parent_block.hash,
				block.hash,
				block.number,
				Some(new_header),
				cache_tx,
			),
		};

		// update configuration cache
		let mut cache_at = HashMap::new();
		cache_at.insert(well_known_cache_keys::CHANGES_TRIE_CONFIG, new_configuration.encode());
		Ok(DbChangesTrieStorageTransaction::from(match cache_tx {
			Some(cache_tx) => self.cache.0.write()
				.transaction_with_ops(tx, cache_tx.cache_ops)
				.on_block_insert(
					parent_block,
					block,
					cache_at,
					if finalized { CacheEntryType::Final } else { CacheEntryType::NonFinal },
				)?
				.into_ops(),
			None => self.cache.0.write()
				.transaction(tx)
				.on_block_insert(
					parent_block,
					block,
					cache_at,
					if finalized { CacheEntryType::Final } else { CacheEntryType::NonFinal },
				)?
				.into_ops(),
		}).with_new_config(Some(new_configuration)))
	}

	/// Called when block is finalized.
	pub fn finalize(
		&self,
		tx: &mut Transaction<DbHash>,
		parent_block_hash: Block::Hash,
		block_hash: Block::Hash,
		block_num: NumberFor<Block>,
		new_header: Option<&Block::Header>,
		cache_tx: Option<DbChangesTrieStorageTransaction<Block>>,
	) -> ClientResult<DbChangesTrieStorageTransaction<Block>> {
		// prune obsolete changes tries
		self.prune(tx, block_hash, block_num, new_header.clone(), cache_tx.as_ref())?;

		// if we have inserted the block that we're finalizing in the same transaction
		// => then we have already finalized it from the commit() call
		if cache_tx.is_some() {
			if let Some(new_header) = new_header {
				if new_header.hash() == block_hash {
					return Ok(cache_tx.expect("guarded by cache_tx.is_some(); qed"));
				}
			}
		}

		// and finalize configuration cache entries
		let block = ComplexBlockId::new(block_hash, block_num);
		let parent_block_num = block_num.checked_sub(&One::one()).unwrap_or_else(|| Zero::zero());
		let parent_block = ComplexBlockId::new(parent_block_hash, parent_block_num);
		Ok(match cache_tx {
			Some(cache_tx) => DbChangesTrieStorageTransaction::from(
				self.cache.0.write()
					.transaction_with_ops(tx, cache_tx.cache_ops)
					.on_block_finalize(
						parent_block,
						block,
					)?
					.into_ops()
			).with_new_config(cache_tx.new_config),
			None => DbChangesTrieStorageTransaction::from(
				self.cache.0.write()
					.transaction(tx)
					.on_block_finalize(
						parent_block,
						block,
					)?
					.into_ops()
			),
		})
	}

	/// When block is reverted.
	pub fn revert(
		&self,
		tx: &mut Transaction<DbHash>,
		block: &ComplexBlockId<Block>,
	) -> ClientResult<DbChangesTrieStorageTransaction<Block>> {
		Ok(self.cache.0.write().transaction(tx)
			.on_block_revert(block)?
			.into_ops()
			.into())
	}

	/// When transaction has been committed.
	pub fn post_commit(&self, tx: Option<DbChangesTrieStorageTransaction<Block>>) {
		if let Some(tx) = tx {
			self.cache.0.write().commit(tx.cache_ops)
				.expect("only fails if cache with given name isn't loaded yet;\
						cache is already loaded because there is tx; qed");
		}
	}

	/// Commit changes into changes trie build cache.
	pub fn commit_build_cache(&self, cache_update: ChangesTrieCacheAction<Block::Hash, NumberFor<Block>>) {
		self.build_cache.write().perform(cache_update);
	}

	/// Prune obsolete changes tries.
	fn prune(
		&self,
		tx: &mut Transaction<DbHash>,
		block_hash: Block::Hash,
		block_num: NumberFor<Block>,
		new_header: Option<&Block::Header>,
		cache_tx: Option<&DbChangesTrieStorageTransaction<Block>>,
	) -> ClientResult<()> {
		// never prune on archive nodes
		let min_blocks_to_keep = match self.min_blocks_to_keep {
			Some(min_blocks_to_keep) => min_blocks_to_keep,
			None => return Ok(()),
		};

		let mut tries_meta = self.tries_meta.write();
		let mut next_digest_range_start = block_num;
		loop {
			// prune oldest digest if it is known
			// it could be unknown if:
			// 1) either we're finalizing block#1
			// 2) or we are (or were) in period where changes tries are disabled
			if let Some((begin, end)) = tries_meta.oldest_digest_range {
				if block_num <= end || block_num - end <= min_blocks_to_keep.into() {
					break;
				}

				tries_meta.oldest_pruned_digest_range_end = end;
				sp_state_machine::prune_changes_tries(
					&*self,
					begin,
					end,
					&sp_state_machine::ChangesTrieAnchorBlockId {
						hash: convert_hash(&block_hash),
						number: block_num,
					},
					|node| tx.remove(self.changes_tries_column, node.as_ref()),
				);

				next_digest_range_start = end + One::one();
			}

			// proceed to the next configuration range
			let next_digest_range_start_hash = match block_num == next_digest_range_start {
				true => block_hash,
				false => utils::require_header::<Block>(
					&*self.db,
					self.key_lookup_column,
					self.header_column,
					BlockId::Number(next_digest_range_start),
				)?.hash(),
			};

			let config_for_new_block = new_header
				.map(|header| *header.number() == next_digest_range_start)
				.unwrap_or(false);
			let next_config = match cache_tx {
				Some(cache_tx) if config_for_new_block && cache_tx.new_config.is_some() => {
					let config = cache_tx
						.new_config
						.clone()
						.expect("guarded by is_some(); qed");
					ChangesTrieConfigurationRange {
						zero: (block_num, block_hash),
						end: None,
						config,
					}
				},
				_ if config_for_new_block => {
					self.configuration_at(&BlockId::Hash(*new_header.expect(
						"config_for_new_block is only true when new_header is passed; qed"
					).parent_hash()))?
				},
				_ => self.configuration_at(&BlockId::Hash(next_digest_range_start_hash))?,
			};
			if let Some(config) = next_config.config {
				let mut oldest_digest_range = config
					.next_max_level_digest_range(next_config.zero.0, next_digest_range_start)
					.unwrap_or_else(|| (next_digest_range_start, next_digest_range_start));

				if let Some(end) = next_config.end {
					if end.0 < oldest_digest_range.1 {
						oldest_digest_range.1 = end.0;
					}
				}

				tries_meta.oldest_digest_range = Some(oldest_digest_range);
				continue;
			}

			tries_meta.oldest_digest_range = None;
			break;
		}

		write_tries_meta(tx, self.meta_column, &*tries_meta);
		Ok(())
	}
}

impl<Block: BlockT> PrunableStateChangesTrieStorage<Block> for DbChangesTrieStorage<Block> {
	fn storage(&self) -> &dyn sp_state_machine::ChangesTrieStorage<HashFor<Block>, NumberFor<Block>> {
		self
	}

	fn configuration_at(&self, at: &BlockId<Block>) -> ClientResult<
		ChangesTrieConfigurationRange<NumberFor<Block>, Block::Hash>
	> {
		self.cache
			.get_at(&well_known_cache_keys::CHANGES_TRIE_CONFIG, at)?
			.and_then(|(zero, end, encoded)| Decode::decode(&mut &encoded[..]).ok()
				.map(|config| ChangesTrieConfigurationRange { zero, end, config }))
			.ok_or_else(|| ClientError::ErrorReadingChangesTriesConfig)
	}

	fn oldest_pruned_digest_range_end(&self) -> NumberFor<Block> {
		self.tries_meta.read().oldest_pruned_digest_range_end
	}
}

impl<Block: BlockT> sp_state_machine::ChangesTrieRootsStorage<HashFor<Block>, NumberFor<Block>>
	for DbChangesTrieStorage<Block>
{
	fn build_anchor(
		&self,
		hash: Block::Hash,
	) -> Result<sp_state_machine::ChangesTrieAnchorBlockId<Block::Hash, NumberFor<Block>>, String> {
		utils::read_header::<Block>(&*self.db, self.key_lookup_column, self.header_column, BlockId::Hash(hash))
			.map_err(|e| e.to_string())
			.and_then(|maybe_header| maybe_header.map(|header|
				sp_state_machine::ChangesTrieAnchorBlockId {
					hash,
					number: *header.number(),
				}
			).ok_or_else(|| format!("Unknown header: {}", hash)))
	}

	fn root(
		&self,
		anchor: &sp_state_machine::ChangesTrieAnchorBlockId<Block::Hash, NumberFor<Block>>,
		block: NumberFor<Block>,
	) -> Result<Option<Block::Hash>, String> {
		// check API requirement: we can't get NEXT block(s) based on anchor
		if block > anchor.number {
			return Err(format!("Can't get changes trie root at {} using anchor at {}", block, anchor.number));
		}

		// we need to get hash of the block to resolve changes trie root
		let block_id = if block <= self.meta.read().finalized_number {
			// if block is finalized, we could just read canonical hash
			BlockId::Number(block)
		} else {
			// the block is not finalized
			let mut current_num = anchor.number;
			let mut current_hash: Block::Hash = convert_hash(&anchor.hash);
			let maybe_anchor_header: Block::Header = utils::require_header::<Block>(
				&*self.db, self.key_lookup_column, self.header_column, BlockId::Number(current_num)
			).map_err(|e| e.to_string())?;
			if maybe_anchor_header.hash() == current_hash {
				// if anchor is canonicalized, then the block is also canonicalized
				BlockId::Number(block)
			} else {
				// else (block is not finalized + anchor is not canonicalized):
				// => we should find the required block hash by traversing
				// back from the anchor to the block with given number
				while current_num != block {
					let current_header: Block::Header = utils::require_header::<Block>(
						&*self.db, self.key_lookup_column, self.header_column, BlockId::Hash(current_hash)
					).map_err(|e| e.to_string())?;

					current_hash = *current_header.parent_hash();
					current_num = current_num - One::one();
				}

				BlockId::Hash(current_hash)
			}
		};

		Ok(
			utils::require_header::<Block>(
				&*self.db,
				self.key_lookup_column,
				self.header_column,
				block_id,
			)
			.map_err(|e| e.to_string())?
			.digest()
			.log(DigestItem::as_changes_trie_root)
			.cloned()
		)
	}
}

impl<Block> sp_state_machine::ChangesTrieStorage<HashFor<Block>, NumberFor<Block>>
	for DbChangesTrieStorage<Block>
where
	Block: BlockT,
{
	fn as_roots_storage(&self) -> &dyn sp_state_machine::ChangesTrieRootsStorage<HashFor<Block>, NumberFor<Block>> {
		self
	}

	fn with_cached_changed_keys(
		&self,
		root: &Block::Hash,
		functor: &mut dyn FnMut(&HashMap<Option<PrefixedStorageKey>, HashSet<Vec<u8>>>),
	) -> bool {
		self.build_cache.read().with_changed_keys(root, functor)
	}

	fn get(&self, key: &Block::Hash, _prefix: Prefix) -> Result<Option<Vec<u8>>, String> {
		Ok(self.db.get(self.changes_tries_column, key.as_ref()))
	}
}

/// Read changes tries metadata from database.
fn read_tries_meta<Block: BlockT>(
	db: &dyn Database<DbHash>,
	meta_column: u32,
) -> ClientResult<ChangesTriesMeta<Block>> {
	match db.get(meta_column, meta_keys::CHANGES_TRIES_META) {
		Some(h) => match Decode::decode(&mut &h[..]) {
			Ok(h) => Ok(h),
			Err(err) => Err(ClientError::Backend(format!("Error decoding changes tries metadata: {}", err))),
		},
		None => Ok(ChangesTriesMeta {
			oldest_digest_range: None,
			oldest_pruned_digest_range_end: Zero::zero(),
		}),
	}
}

/// Write changes tries metadata from database.
fn write_tries_meta<Block: BlockT>(
	tx: &mut Transaction<DbHash>,
	meta_column: u32,
	meta: &ChangesTriesMeta<Block>,
) {
	tx.set_from_vec(meta_column, meta_keys::CHANGES_TRIES_META, meta.encode());
}

