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

//! Pruning window.
//!
//! For each block we maintain a list of nodes pending deletion.
//! There is also a global index of node key to block number.
//! If a node is re-inserted into the window it gets removed from
//! the death list.
//! The changes are journaled in the DB.

use std::collections::{HashMap, HashSet, VecDeque};
use codec::{Encode, Decode};
use crate::{CommitSet, Error, MetaDb, to_meta_key, Hash};
use log::{trace, warn};

const LAST_PRUNED: &[u8] = b"last_pruned";
const PRUNING_JOURNAL: &[u8] = b"pruning_journal";

/// See module documentation.
#[derive(parity_util_mem_derive::MallocSizeOf)]
pub struct RefWindow<BlockHash: Hash, Key: Hash> {
	/// A queue of keys that should be deleted for each block in the pruning window.
	death_rows: VecDeque<DeathRow<BlockHash, Key>>,
	/// An index that maps each key from `death_rows` to block number.
	death_index: HashMap<Key, u64>,
	/// Block number that corresponds to the front of `death_rows`.
	pending_number: u64,
	/// Number of call of `note_canonical` after
	/// last call `apply_pending` or `revert_pending`
	pending_canonicalizations: usize,
	/// Number of calls of `prune_one` after
	/// last call `apply_pending` or `revert_pending`
	pending_prunings: usize,
	/// Keep track of re-inserted keys and do not delete them when pruning.
	/// Setting this to false requires backend that supports reference
	/// counting.
	count_insertions: bool,
}

#[derive(Debug, PartialEq, Eq, parity_util_mem_derive::MallocSizeOf)]
struct DeathRow<BlockHash: Hash, Key: Hash> {
	hash: BlockHash,
	journal_key: Vec<u8>,
	deleted: HashSet<Key>,
}

#[derive(Encode, Decode)]
struct JournalRecord<BlockHash: Hash, Key: Hash> {
	hash: BlockHash,
	inserted: Vec<Key>,
	deleted: Vec<Key>,
}

fn to_journal_key(block: u64) -> Vec<u8> {
	to_meta_key(PRUNING_JOURNAL, &block)
}

impl<BlockHash: Hash, Key: Hash> RefWindow<BlockHash, Key> {
	pub fn new<D: MetaDb>(db: &D, count_insertions: bool) -> Result<RefWindow<BlockHash, Key>, Error<D::Error>> {
		let last_pruned = db.get_meta(&to_meta_key(LAST_PRUNED, &()))
			.map_err(|e| Error::Db(e))?;
		let pending_number: u64 = match last_pruned {
			Some(buffer) => u64::decode(&mut buffer.as_slice())? + 1,
			None => 0,
		};
		let mut block = pending_number;
		let mut pruning = RefWindow {
			death_rows: Default::default(),
			death_index: Default::default(),
			pending_number: pending_number,
			pending_canonicalizations: 0,
			pending_prunings: 0,
			count_insertions,
		};
		// read the journal
		trace!(target: "state-db", "Reading pruning journal. Pending #{}", pending_number);
		loop {
			let journal_key = to_journal_key(block);
			match db.get_meta(&journal_key).map_err(|e| Error::Db(e))? {
				Some(record) => {
					let record: JournalRecord<BlockHash, Key> = Decode::decode(&mut record.as_slice())?;
					trace!(target: "state-db", "Pruning journal entry {} ({} inserted, {} deleted)", block, record.inserted.len(), record.deleted.len());
					pruning.import(&record.hash, journal_key, record.inserted.into_iter(), record.deleted);
				},
				None => break,
			}
			block += 1;
		}
		Ok(pruning)
	}

	fn import<I: IntoIterator<Item=Key>>(&mut self, hash: &BlockHash, journal_key: Vec<u8>, inserted: I, deleted: Vec<Key>) {
		if self.count_insertions {
			// remove all re-inserted keys from death rows
			for k in inserted {
				if let Some(block) = self.death_index.remove(&k) {
					self.death_rows[(block - self.pending_number) as usize].deleted.remove(&k);
				}
			}

			// add new keys
			let imported_block = self.pending_number + self.death_rows.len() as u64;
			for k in deleted.iter() {
				self.death_index.insert(k.clone(), imported_block);
			}
		}
		self.death_rows.push_back(
			DeathRow {
				hash: hash.clone(),
				deleted: deleted.into_iter().collect(),
				journal_key: journal_key,
			}
		);
	}

	pub fn window_size(&self) -> u64 {
		(self.death_rows.len() - self.pending_prunings) as u64
	}

	pub fn next_hash(&self) -> Option<BlockHash> {
		self.death_rows.get(self.pending_prunings).map(|r| r.hash.clone())
	}

	pub fn mem_used(&self) -> usize {
		0
	}

	pub fn pending(&self) -> u64 {
		self.pending_number + self.pending_prunings as u64
	}

	pub fn have_block(&self, hash: &BlockHash) -> bool {
		self.death_rows.iter().skip(self.pending_prunings).any(|r| r.hash == *hash)
	}

	/// Prune next block. Expects at least one block in the window. Adds changes to `commit`.
	pub fn prune_one(&mut self, commit: &mut CommitSet<Key>) {
		if let Some(pruned) = self.death_rows.get(self.pending_prunings) {
			trace!(target: "state-db", "Pruning {:?} ({} deleted)", pruned.hash, pruned.deleted.len());
			let index = self.pending_number + self.pending_prunings as u64;
			commit.data.deleted.extend(pruned.deleted.iter().cloned());
			commit.meta.inserted.push((to_meta_key(LAST_PRUNED, &()), index.encode()));
			commit.meta.deleted.push(pruned.journal_key.clone());
			self.pending_prunings += 1;
		} else {
			warn!(target: "state-db", "Trying to prune when there's nothing to prune");
		}
	}

	/// Add a change set to the window. Creates a journal record and pushes it to `commit`
	pub fn note_canonical(&mut self, hash: &BlockHash, commit: &mut CommitSet<Key>) {
		trace!(target: "state-db", "Adding to pruning window: {:?} ({} inserted, {} deleted)", hash, commit.data.inserted.len(), commit.data.deleted.len());
		let inserted = if self.count_insertions {
			commit.data.inserted.iter().map(|(k, _)| k.clone()).collect()
		} else {
			Default::default()
		};
		let deleted = ::std::mem::take(&mut commit.data.deleted);
		let journal_record = JournalRecord {
			hash: hash.clone(),
			inserted,
			deleted,
		};
		let block = self.pending_number + self.death_rows.len() as u64;
		let journal_key = to_journal_key(block);
		commit.meta.inserted.push((journal_key.clone(), journal_record.encode()));
		self.import(&journal_record.hash, journal_key, journal_record.inserted.into_iter(), journal_record.deleted);
		self.pending_canonicalizations += 1;
	}

	/// Apply all pending changes
	pub fn apply_pending(&mut self) {
		self.pending_canonicalizations = 0;
		for _ in 0 .. self.pending_prunings {
			let pruned = self.death_rows.pop_front().expect("pending_prunings is always < death_rows.len()");
			trace!(target: "state-db", "Applying pruning {:?} ({} deleted)", pruned.hash, pruned.deleted.len());
			if self.count_insertions {
				for k in pruned.deleted.iter() {
					self.death_index.remove(&k);
				}
			}
			self.pending_number += 1;
		}
		self.pending_prunings = 0;
	}

	/// Revert all pending changes
	pub fn revert_pending(&mut self) {
		// Revert pending deletions.
		// Note that pending insertions might cause some existing deletions to be removed from `death_index`
		// We don't bother to track and revert that for now. This means that a few nodes might end up no being
		// deleted in case transaction fails and `revert_pending` is called.
		self.death_rows.truncate(self.death_rows.len() - self.pending_canonicalizations);
		if self.count_insertions {
			let new_max_block = self.death_rows.len() as u64 + self.pending_number;
			self.death_index.retain(|_, block| *block < new_max_block);
		}
		self.pending_canonicalizations = 0;
		self.pending_prunings = 0;
	}
}


