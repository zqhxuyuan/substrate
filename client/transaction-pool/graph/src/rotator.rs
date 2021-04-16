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

//! Rotate extrinsic inside the pool.
//!
//! Keeps only recent extrinsic and discard the ones kept for a significant amount of time.
//! Discarded extrinsics are banned so that they don't get re-imported again.

use std::{
	collections::HashMap,
	hash,
	iter,
	time::Duration,
};
use parking_lot::RwLock;
use wasm_timer::Instant;

use crate::base_pool::Transaction;

/// Expected size of the banned extrinsics cache.
const EXPECTED_SIZE: usize = 2048;

/// Pool rotator is responsible to only keep fresh extrinsics in the pool.
///
/// Extrinsics that occupy the pool for too long are culled and temporarily banned from entering
/// the pool again.
pub struct PoolRotator<Hash> {
	/// How long the extrinsic is banned for.
	ban_time: Duration,
	/// Currently banned extrinsics.
	banned_until: RwLock<HashMap<Hash, Instant>>,
}

impl<Hash: hash::Hash + Eq> Default for PoolRotator<Hash> {
	fn default() -> Self {
		PoolRotator {
			ban_time: Duration::from_secs(60 * 30),
			banned_until: Default::default(),
		}
	}
}

impl<Hash: hash::Hash + Eq + Clone> PoolRotator<Hash> {
	/// Returns `true` if extrinsic hash is currently banned.
	pub fn is_banned(&self, hash: &Hash) -> bool {
		self.banned_until.read().contains_key(hash)
	}

	/// Bans given set of hashes.
	pub fn ban(&self, now: &Instant, hashes: impl IntoIterator<Item=Hash>) {
		let mut banned = self.banned_until.write();

		for hash in hashes {
			banned.insert(hash, *now + self.ban_time);
		}

		if banned.len() > 2 * EXPECTED_SIZE {
			while banned.len() > EXPECTED_SIZE {
				if let Some(key) = banned.keys().next().cloned() {
					banned.remove(&key);
				}
			}
		}
	}


	/// Bans extrinsic if it's stale.
	///
	/// Returns `true` if extrinsic is stale and got banned.
	pub fn ban_if_stale<Ex>(&self, now: &Instant, current_block: u64, xt: &Transaction<Hash, Ex>) -> bool {
		if xt.valid_till > current_block {
			return false;
		}

		self.ban(now, iter::once(xt.hash.clone()));
		true
	}

	/// Removes timed bans.
	pub fn clear_timeouts(&self, now: &Instant) {
		let mut banned = self.banned_until.write();

		banned.retain(|_, &mut v| v >= *now);
	}
}


