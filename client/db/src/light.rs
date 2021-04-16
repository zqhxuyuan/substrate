// This file is part of Substrate.

// Copyright (C) 2017-2021 Parity Technologies (UK) Ltd.
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

//! RocksDB-based light client blockchain storage.

use std::{sync::Arc, collections::HashMap};
use std::convert::TryInto;
use parking_lot::RwLock;

use sc_client_api::{
	cht, backend::{AuxStore, NewBlockState, ProvideChtRoots}, UsageInfo,
	blockchain::{
		BlockStatus, Cache as BlockchainCache, Info as BlockchainInfo,
	},
	Storage,
};
use sp_blockchain::{
	CachedHeaderMetadata, HeaderMetadata, HeaderMetadataCache,
	Error as ClientError, Result as ClientResult,
	HeaderBackend as BlockchainHeaderBackend,
	well_known_cache_keys,
};
use sp_database::{Database, Transaction};
use codec::{Decode, Encode};
use sp_runtime::generic::{DigestItem, BlockId};
use sp_runtime::traits::{Block as BlockT, Header as HeaderT, Zero, One, NumberFor, HashFor};
use crate::cache::{DbCacheSync, DbCache, ComplexBlockId, EntryType as CacheEntryType};
use crate::utils::{self, meta_keys, DatabaseType, Meta, read_db, block_id_to_lookup_key, read_meta};
use crate::{DatabaseSettings, FrozenForDuration, DbHash};
use log::{trace, warn, debug};

pub(crate) mod columns {
	pub const META: u32 = crate::utils::COLUMN_META;
	pub const KEY_LOOKUP: u32 = 1;
	pub const HEADER: u32 = 2;
	pub const CACHE: u32 = 3;
	pub const CHT: u32 = 4;
	pub const AUX: u32 = 5;
}

/// Prefix for headers CHT.
const HEADER_CHT_PREFIX: u8 = 0;
/// Prefix for changes tries roots CHT.
const CHANGES_TRIE_CHT_PREFIX: u8 = 1;

/// Light blockchain storage. Stores most recent headers + CHTs for older headers.
/// Locks order: meta, cache.
pub struct LightStorage<Block: BlockT> {
	db: Arc<dyn Database<DbHash>>,
	meta: RwLock<Meta<NumberFor<Block>, Block::Hash>>,
	cache: Arc<DbCacheSync<Block>>,
	header_metadata_cache: Arc<HeaderMetadataCache<Block>>,

	#[cfg(not(target_os = "unknown"))]
	io_stats: FrozenForDuration<kvdb::IoStats>,
}

impl<Block: BlockT> LightStorage<Block> {
	/// Create new storage with given settings.
	pub fn new(config: DatabaseSettings) -> ClientResult<Self> {
		let db = crate::utils::open_database::<Block>(&config, DatabaseType::Light)?;
		Self::from_kvdb(db as Arc<_>)
	}

	/// Create new memory-backed `LightStorage` for tests.
	#[cfg(any(test, feature = "test-helpers"))]
	pub fn new_test() -> Self {
		let db = Arc::new(sp_database::MemDb::default());
		Self::from_kvdb(db as Arc<_>).expect("failed to create test-db")
	}

	fn from_kvdb(db: Arc<dyn Database<DbHash>>) -> ClientResult<Self> {
		let meta = read_meta::<Block>(&*db, columns::HEADER)?;
		let header_metadata_cache = Arc::new(HeaderMetadataCache::default());
		let cache = DbCache::new(
			db.clone(),
			header_metadata_cache.clone(),
			columns::KEY_LOOKUP,
			columns::HEADER,
			columns::CACHE,
			meta.genesis_hash,
			ComplexBlockId::new(meta.finalized_hash, meta.finalized_number),
		);

		Ok(LightStorage {
			db,
			meta: RwLock::new(meta),
			cache: Arc::new(DbCacheSync(RwLock::new(cache))),
			header_metadata_cache,
			#[cfg(not(target_os = "unknown"))]
			io_stats: FrozenForDuration::new(std::time::Duration::from_secs(1)),
		})
	}

	#[cfg(test)]
	pub(crate) fn cache(&self) -> &DbCacheSync<Block> {
		&self.cache
	}

	fn update_meta(
		&self,
		hash: Block::Hash,
		number: NumberFor<Block>,
		is_best: bool,
		is_finalized: bool,
	) {
		let mut meta = self.meta.write();

		if number.is_zero() {
			meta.genesis_hash = hash;
			meta.finalized_hash = hash;
		}

		if is_best {
			meta.best_number = number;
			meta.best_hash = hash;
		}

		if is_finalized {
			meta.finalized_number = number;
			meta.finalized_hash = hash;
		}
	}
}

impl<Block> BlockchainHeaderBackend<Block> for LightStorage<Block>
	where
		Block: BlockT,
{
	fn header(&self, id: BlockId<Block>) -> ClientResult<Option<Block::Header>> {
		utils::read_header(&*self.db, columns::KEY_LOOKUP, columns::HEADER, id)
	}

	fn info(&self) -> BlockchainInfo<Block> {
		let meta = self.meta.read();
		BlockchainInfo {
			best_hash: meta.best_hash,
			best_number: meta.best_number,
			genesis_hash: meta.genesis_hash,
			finalized_hash: meta.finalized_hash,
			finalized_number: meta.finalized_number,
			number_leaves: 1,
		}
	}

	fn status(&self, id: BlockId<Block>) -> ClientResult<BlockStatus> {
		let exists = match id {
			BlockId::Hash(_) => read_db(
				&*self.db,
				columns::KEY_LOOKUP,
				columns::HEADER,
				id
			)?.is_some(),
			BlockId::Number(n) => n <= self.meta.read().best_number,
		};
		match exists {
			true => Ok(BlockStatus::InChain),
			false => Ok(BlockStatus::Unknown),
		}
	}

	fn number(&self, hash: Block::Hash) -> ClientResult<Option<NumberFor<Block>>> {
		if let Some(lookup_key) = block_id_to_lookup_key::<Block>(&*self.db, columns::KEY_LOOKUP, BlockId::Hash(hash))? {
			let number = utils::lookup_key_to_number(&lookup_key)?;
			Ok(Some(number))
		} else {
			Ok(None)
		}
	}

	fn hash(&self, number: NumberFor<Block>) -> ClientResult<Option<Block::Hash>> {
		Ok(self.header(BlockId::Number(number))?.map(|header| header.hash().clone()))
	}
}

impl<Block: BlockT> HeaderMetadata<Block> for LightStorage<Block> {
	type Error = ClientError;

	fn header_metadata(&self, hash: Block::Hash) -> Result<CachedHeaderMetadata<Block>, Self::Error> {
		self.header_metadata_cache.header_metadata(hash).map_or_else(|| {
			self.header(BlockId::hash(hash))?.map(|header| {
				let header_metadata = CachedHeaderMetadata::from(&header);
				self.header_metadata_cache.insert_header_metadata(
					header_metadata.hash,
					header_metadata.clone(),
				);
				header_metadata
			}).ok_or_else(|| ClientError::UnknownBlock(format!("header not found in db: {}", hash)))
		}, Ok)
	}

	fn insert_header_metadata(&self, hash: Block::Hash, metadata: CachedHeaderMetadata<Block>) {
		self.header_metadata_cache.insert_header_metadata(hash, metadata)
	}

	fn remove_header_metadata(&self, hash: Block::Hash) {
		self.header_metadata_cache.remove_header_metadata(hash);
	}
}

impl<Block: BlockT> LightStorage<Block> {
	// Get block changes trie root, if available.
	fn changes_trie_root(&self, block: BlockId<Block>) -> ClientResult<Option<Block::Hash>> {
		self.header(block)
			.map(|header| header.and_then(|header|
				header.digest().log(DigestItem::as_changes_trie_root)
					.cloned()))
	}

	/// Handle setting head within a transaction. `route_to` should be the last
	/// block that existed in the database. `best_to` should be the best block
	/// to be set.
	///
	/// In the case where the new best block is a block to be imported, `route_to`
	/// should be the parent of `best_to`. In the case where we set an existing block
	/// to be best, `route_to` should equal to `best_to`.
	fn set_head_with_transaction(
		&self,
		transaction: &mut Transaction<DbHash>,
		route_to: Block::Hash,
		best_to: (NumberFor<Block>, Block::Hash),
	) -> ClientResult<()> {
		let lookup_key = utils::number_and_hash_to_lookup_key(best_to.0, &best_to.1)?;

		// handle reorg.
		let meta = self.meta.read();
		if meta.best_hash != Default::default() {
			let tree_route = sp_blockchain::tree_route(self, meta.best_hash, route_to)?;

			// update block number to hash lookup entries.
			for retracted in tree_route.retracted() {
				if retracted.hash == meta.finalized_hash {
					// TODO: can we recover here?
					warn!("Safety failure: reverting finalized block {:?}",
						  (&retracted.number, &retracted.hash));
				}

				utils::remove_number_to_key_mapping(
					transaction,
					columns::KEY_LOOKUP,
					retracted.number
				)?;
			}

			for enacted in tree_route.enacted() {
				utils::insert_number_to_key_mapping(
					transaction,
					columns::KEY_LOOKUP,
					enacted.number,
					enacted.hash
				)?;
			}
		}

		transaction.set_from_vec(columns::META, meta_keys::BEST_BLOCK, lookup_key);
		utils::insert_number_to_key_mapping(
			transaction,
			columns::KEY_LOOKUP,
			best_to.0,
			best_to.1,
		)?;

		Ok(())
	}

	// Note that a block is finalized. Only call with child of last finalized block.
	fn note_finalized(
		&self,
		transaction: &mut Transaction<DbHash>,
		header: &Block::Header,
		hash: Block::Hash,
	) -> ClientResult<()> {
		let meta = self.meta.read();
		if &meta.finalized_hash != header.parent_hash() {
			return Err(::sp_blockchain::Error::NonSequentialFinalization(
				format!("Last finalized {:?} not parent of {:?}",
					meta.finalized_hash, hash),
			).into())
		}

		let lookup_key = utils::number_and_hash_to_lookup_key(header.number().clone(), hash)?;
		transaction.set_from_vec(columns::META, meta_keys::FINALIZED_BLOCK, lookup_key);

		// build new CHT(s) if required
		if let Some(new_cht_number) = cht::is_build_required(cht::size(), *header.number()) {
			let new_cht_start: NumberFor<Block> = cht::start_number(cht::size(), new_cht_number);

			let mut current_num = new_cht_start;
			let cht_range = ::std::iter::from_fn(|| {
				let old_current_num = current_num;
				current_num = current_num + One::one();
				Some(old_current_num)
			});

			let new_header_cht_root = cht::compute_root::<Block::Header, HashFor<Block>, _>(
				cht::size(), new_cht_number, cht_range.map(|num| self.hash(num))
			)?;
			transaction.set(
				columns::CHT,
				&cht_key(HEADER_CHT_PREFIX, new_cht_start)?,
				new_header_cht_root.as_ref()
			);

			// if the header includes changes trie root, let's build a changes tries roots CHT
			if header.digest().log(DigestItem::as_changes_trie_root).is_some() {
				let mut current_num = new_cht_start;
				let cht_range = std::iter::from_fn(|| {
					let old_current_num = current_num;
					current_num = current_num + One::one();
					Some(old_current_num)
				});
				let new_changes_trie_cht_root = cht::compute_root::<Block::Header, HashFor<Block>, _>(
					cht::size(), new_cht_number, cht_range
						.map(|num| self.changes_trie_root(BlockId::Number(num)))
				)?;
				transaction.set(
					columns::CHT,
					&cht_key(CHANGES_TRIE_CHT_PREFIX, new_cht_start)?,
					new_changes_trie_cht_root.as_ref()
				);
			}

			// prune headers that are replaced with CHT
			let mut prune_block = new_cht_start;
			let new_cht_end = cht::end_number(cht::size(), new_cht_number);
			trace!(target: "db", "Replacing blocks [{}..{}] with CHT#{}",
				new_cht_start, new_cht_end, new_cht_number);

			while prune_block <= new_cht_end {
				if let Some(hash) = self.hash(prune_block)? {
					let lookup_key = block_id_to_lookup_key::<Block>(&*self.db, columns::KEY_LOOKUP, BlockId::Number(prune_block))?
						.expect("retrieved hash for `prune_block` right above. therefore retrieving lookup key must succeed. q.e.d.");
					utils::remove_key_mappings(
						transaction,
						columns::KEY_LOOKUP,
						prune_block,
						hash
					)?;
					transaction.remove(columns::HEADER, &lookup_key);
				}
				prune_block += One::one();
			}
		}

		Ok(())
	}

	/// Read CHT root of given type for the block.
	fn read_cht_root(
		&self,
		cht_type: u8,
		cht_size: NumberFor<Block>,
		block: NumberFor<Block>
	) -> ClientResult<Option<Block::Hash>> {
		let no_cht_for_block = || ClientError::Backend(format!("Missing CHT for block {}", block));

		let meta = self.meta.read();
		let max_cht_number = cht::max_cht_number(cht_size, meta.finalized_number);
		let cht_number = cht::block_to_cht_number(cht_size, block).ok_or_else(no_cht_for_block)?;
		match max_cht_number {
			Some(max_cht_number) if cht_number <= max_cht_number => (),
			_ => return Ok(None),
		}

		let cht_start = cht::start_number(cht_size, cht_number);
		self.db.get(columns::CHT, &cht_key(cht_type, cht_start)?)
			.ok_or_else(no_cht_for_block)
			.and_then(|hash| Block::Hash::decode(&mut &*hash).map_err(|_| no_cht_for_block()))
			.map(Some)
	}
}

impl<Block> AuxStore for LightStorage<Block>
	where Block: BlockT,
{
	fn insert_aux<
		'a,
		'b: 'a,
		'c: 'a,
		I: IntoIterator<Item=&'a(&'c [u8], &'c [u8])>,
		D: IntoIterator<Item=&'a &'b [u8]>,
	>(&self, insert: I, delete: D) -> ClientResult<()> {
		let mut transaction = Transaction::new();
		for (k, v) in insert {
			transaction.set(columns::AUX, k, v);
		}
		for k in delete {
			transaction.remove(columns::AUX, k);
		}
		self.db.commit(transaction)?;

		Ok(())
	}

	fn get_aux(&self, key: &[u8]) -> ClientResult<Option<Vec<u8>>> {
		Ok(self.db.get(columns::AUX, key))
	}
}

impl<Block> Storage<Block> for LightStorage<Block>
	where Block: BlockT,
{
	fn import_header(
		&self,
		header: Block::Header,
		mut cache_at: HashMap<well_known_cache_keys::Id, Vec<u8>>,
		leaf_state: NewBlockState,
		aux_ops: Vec<(Vec<u8>, Option<Vec<u8>>)>,
	) -> ClientResult<()> {
		let mut transaction = Transaction::new();

		let hash = header.hash();
		let number = *header.number();
		let parent_hash = *header.parent_hash();

		for (key, maybe_val) in aux_ops {
			match maybe_val {
				Some(val) => transaction.set_from_vec(columns::AUX, &key, val),
				None => transaction.remove(columns::AUX, &key),
			}
		}

		// blocks are keyed by number + hash.
		let lookup_key = utils::number_and_hash_to_lookup_key(number, &hash)?;

		if leaf_state.is_best() {
			self.set_head_with_transaction(&mut transaction, parent_hash, (number, hash))?;
		}

		utils::insert_hash_to_key_mapping(
			&mut transaction,
			columns::KEY_LOOKUP,
			number,
			hash,
		)?;
		transaction.set_from_vec(columns::HEADER, &lookup_key, header.encode());

		let header_metadata = CachedHeaderMetadata::from(&header);
		self.header_metadata_cache.insert_header_metadata(
			header.hash().clone(),
			header_metadata,
		);

		let is_genesis = number.is_zero();
		if is_genesis {
			self.cache.0.write().set_genesis_hash(hash);
			transaction.set(columns::META, meta_keys::GENESIS_HASH, hash.as_ref());
		}

		let finalized = match leaf_state {
			_ if is_genesis => true,
			NewBlockState::Final => true,
			_ => false,
		};

		if finalized {
			self.note_finalized(
				&mut transaction,
				&header,
				hash,
			)?;
		}

		// update changes trie configuration cache
		if !cache_at.contains_key(&well_known_cache_keys::CHANGES_TRIE_CONFIG) {
			if let Some(new_configuration) = crate::changes_tries_storage::extract_new_configuration(&header) {
				cache_at.insert(well_known_cache_keys::CHANGES_TRIE_CONFIG, new_configuration.encode());
			}
		}

		{
			let mut cache = self.cache.0.write();
			let cache_ops = cache.transaction(&mut transaction)
				.on_block_insert(
					ComplexBlockId::new(*header.parent_hash(), if number.is_zero() { Zero::zero() } else { number - One::one() }),
					ComplexBlockId::new(hash, number),
					cache_at,
					if finalized { CacheEntryType::Final } else { CacheEntryType::NonFinal },
				)?
				.into_ops();

			debug!("Light DB Commit {:?} ({})", hash, number);

			self.db.commit(transaction)?;
			cache.commit(cache_ops)
				.expect("only fails if cache with given name isn't loaded yet;\
						cache is already loaded because there are cache_ops; qed");
		}

		self.update_meta(hash, number, leaf_state.is_best(), finalized);

		Ok(())
	}

	fn set_head(&self, id: BlockId<Block>) -> ClientResult<()> {
		if let Some(header) = self.header(id)? {
			let hash = header.hash();
			let number = header.number();

			let mut transaction = Transaction::new();
			self.set_head_with_transaction(&mut transaction, hash.clone(), (number.clone(), hash.clone()))?;
			self.db.commit(transaction)?;
			self.update_meta(hash, header.number().clone(), true, false);

			Ok(())
		} else {
			Err(ClientError::UnknownBlock(format!("Cannot set head {:?}", id)))
		}
	}

	fn finalize_header(&self, id: BlockId<Block>) -> ClientResult<()> {
		if let Some(header) = self.header(id)? {
			let mut transaction = Transaction::new();
			let hash = header.hash();
			let number = *header.number();
			self.note_finalized(&mut transaction, &header, hash.clone())?;
			{
				let mut cache = self.cache.0.write();
				let cache_ops = cache.transaction(&mut transaction)
					.on_block_finalize(
						ComplexBlockId::new(*header.parent_hash(), if number.is_zero() { Zero::zero() } else { number - One::one() }),
						ComplexBlockId::new(hash, number)
					)?
					.into_ops();

				self.db.commit(transaction)?;
				cache.commit(cache_ops)
					.expect("only fails if cache with given name isn't loaded yet;\
							cache is already loaded because there are cache_ops; qed");
			}
			self.update_meta(hash, header.number().clone(), false, true);

			Ok(())
		} else {
			Err(ClientError::UnknownBlock(format!("Cannot finalize block {:?}", id)))
		}
	}

	fn last_finalized(&self) -> ClientResult<Block::Hash> {
		Ok(self.meta.read().finalized_hash.clone())
	}

	fn cache(&self) -> Option<Arc<dyn BlockchainCache<Block>>> {
		Some(self.cache.clone())
	}

	#[cfg(not(target_os = "unknown"))]
	fn usage_info(&self) -> Option<UsageInfo> {
		use sc_client_api::{MemoryInfo, IoInfo, MemorySize};

		// TODO: reimplement IO stats
		let database_cache = MemorySize::from_bytes(0);
		let io_stats = self.io_stats.take_or_else(|| kvdb::IoStats::empty());

		Some(UsageInfo {
			memory: MemoryInfo {
				database_cache,
				state_cache: Default::default(),
				state_db: Default::default(),
			},
			io: IoInfo {
				transactions: io_stats.transactions,
				bytes_read: io_stats.bytes_read,
				bytes_written: io_stats.bytes_written,
				writes: io_stats.writes,
				reads: io_stats.reads,
				average_transaction_size: io_stats.avg_transaction_size() as u64,
				// Light client does not track those
				state_reads: 0,
				state_writes: 0,
				state_reads_cache: 0,
				state_writes_cache: 0,
				state_writes_nodes: 0,
			}
		})
	}

	#[cfg(target_os = "unknown")]
	fn usage_info(&self) -> Option<UsageInfo> {
		None
	}
}

impl<Block> ProvideChtRoots<Block> for LightStorage<Block>
	where Block: BlockT,
{
	fn header_cht_root(
		&self,
		cht_size: NumberFor<Block>,
		block: NumberFor<Block>,
	) -> ClientResult<Option<Block::Hash>> {
		self.read_cht_root(HEADER_CHT_PREFIX, cht_size, block)
	}

	fn changes_trie_cht_root(
		&self,
		cht_size: NumberFor<Block>,
		block: NumberFor<Block>,
	) -> ClientResult<Option<Block::Hash>> {
		self.read_cht_root(CHANGES_TRIE_CHT_PREFIX, cht_size, block)
	}
}

/// Build the key for inserting header-CHT at given block.
fn cht_key<N: TryInto<u32>>(cht_type: u8, block: N) -> ClientResult<[u8; 5]> {
	let mut key = [cht_type; 5];
	key[1..].copy_from_slice(&utils::number_index_key(block)?);
	Ok(key)
}

