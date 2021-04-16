// This file is part of Substrate.

// Copyright (C) 2017-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! System manager: Handles all of the top-level stuff; executing block/transaction, setting code
//! and depositing logs.

use sp_std::prelude::*;
use sp_io::{
	storage::root as storage_root, storage::changes_root as storage_changes_root,
	hashing::blake2_256, trie,
};
use frame_support::storage;
use frame_support::{decl_storage, decl_module};
use sp_runtime::{
	traits::Header as _, generic, ApplyExtrinsicResult,
	transaction_validity::{
		TransactionValidity, ValidTransaction, InvalidTransaction, TransactionValidityError,
	},
};
use codec::{KeyedVec, Encode, Decode};
use frame_system::Config;
use crate::{
	AccountId, BlockNumber, Extrinsic, Transfer, H256 as Hash, Block, Header, Digest, AuthorityId
};
use sp_core::{storage::well_known_keys, ChangesTrieConfiguration};

const NONCE_OF: &[u8] = b"nonce:";
const BALANCE_OF: &[u8] = b"balance:";

decl_module! {
	pub struct Module<T: Config> for enum Call where origin: T::Origin {}
}

decl_storage! {
	trait Store for Module<T: Config> as TestRuntime {
		ExtrinsicData: map hasher(blake2_128_concat) u32 => Vec<u8>;
		// The current block number being processed. Set by `execute_block`.
		Number get(fn number): Option<BlockNumber>;
		ParentHash get(fn parent_hash): Hash;
		NewAuthorities get(fn new_authorities): Option<Vec<AuthorityId>>;
		NewChangesTrieConfig get(fn new_changes_trie_config): Option<Option<ChangesTrieConfiguration>>;
		StorageDigest get(fn storage_digest): Option<Digest>;
		Authorities get(fn authorities) config(): Vec<AuthorityId>;
	}
}

pub fn balance_of_key(who: AccountId) -> Vec<u8> {
	who.to_keyed_vec(BALANCE_OF)
}

pub fn balance_of(who: AccountId) -> u64 {
	storage::hashed::get_or(&blake2_256, &balance_of_key(who), 0)
}

pub fn nonce_of(who: AccountId) -> u64 {
	storage::hashed::get_or(&blake2_256, &who.to_keyed_vec(NONCE_OF), 0)
}

pub fn initialize_block(header: &Header) {
	// populate environment.
	<Number>::put(&header.number);
	<ParentHash>::put(&header.parent_hash);
	<StorageDigest>::put(header.digest());
	storage::unhashed::put(well_known_keys::EXTRINSIC_INDEX, &0u32);

	// try to read something that depends on current header digest
	// so that it'll be included in execution proof
	if let Some(generic::DigestItem::Other(v)) = header.digest().logs().iter().next() {
		let _: Option<u32> = storage::unhashed::get(&v);
	}
}

pub fn authorities() -> Vec<AuthorityId> {
	Authorities::get()
}

pub fn get_block_number() -> Option<BlockNumber> {
	Number::get()
}

pub fn take_block_number() -> Option<BlockNumber> {
	Number::take()
}

#[derive(Copy, Clone)]
enum Mode {
	Verify,
	Overwrite,
}

/// Actually execute all transitioning for `block`.
pub fn polish_block(block: &mut Block) {
	execute_block_with_state_root_handler(block, Mode::Overwrite);
}

pub fn execute_block(mut block: Block) {
	execute_block_with_state_root_handler(&mut block, Mode::Verify);
}

fn execute_block_with_state_root_handler(block: &mut Block, mode: Mode) {
	let header = &mut block.header;

	initialize_block(header);

	// execute transactions
	block.extrinsics.iter().for_each(|e| {
		let _ = execute_transaction(e.clone()).unwrap_or_else(|_| panic!("Invalid transaction"));
	});

	let new_header = finalize_block();

	if let Mode::Overwrite = mode {
		header.state_root = new_header.state_root;
	} else {
		info_expect_equal_hash(&new_header.state_root, &header.state_root);
		assert!(
			new_header.state_root == header.state_root,
			"Storage root must match that calculated.",
		);
	}

	if let Mode::Overwrite = mode {
		header.extrinsics_root = new_header.extrinsics_root;
	} else {
		info_expect_equal_hash(&new_header.extrinsics_root, &header.extrinsics_root);
		assert!(
			new_header.extrinsics_root == header.extrinsics_root,
			"Transaction trie root must be valid.",
		);
	}
}

/// The block executor.
pub struct BlockExecutor;

impl frame_support::traits::ExecuteBlock<Block> for BlockExecutor {
	fn execute_block(block: Block) {
		execute_block(block);
	}
}

/// Execute a transaction outside of the block execution function.
/// This doesn't attempt to validate anything regarding the block.
pub fn validate_transaction(utx: Extrinsic) -> TransactionValidity {
	if check_signature(&utx).is_err() {
		return InvalidTransaction::BadProof.into();
	}

	let tx = utx.transfer();
	let nonce_key = tx.from.to_keyed_vec(NONCE_OF);
	let expected_nonce: u64 = storage::hashed::get_or(&blake2_256, &nonce_key, 0);
	if tx.nonce < expected_nonce {
		return InvalidTransaction::Stale.into();
	}
	if tx.nonce > expected_nonce + 64 {
		return InvalidTransaction::Future.into();
	}

	let encode = |from: &AccountId, nonce: u64| (from, nonce).encode();
	let requires = if tx.nonce != expected_nonce && tx.nonce > 0 {
		vec![encode(&tx.from, tx.nonce - 1)]
	} else {
		vec![]
	};

	let provides = vec![encode(&tx.from, tx.nonce)];

	Ok(ValidTransaction {
		priority: tx.amount,
		requires,
		provides,
		longevity: 64,
		propagate: true,
	})
}

/// Execute a transaction outside of the block execution function.
/// This doesn't attempt to validate anything regarding the block.
pub fn execute_transaction(utx: Extrinsic) -> ApplyExtrinsicResult {
	let extrinsic_index: u32 = storage::unhashed::get(well_known_keys::EXTRINSIC_INDEX).unwrap();
	let result = execute_transaction_backend(&utx, extrinsic_index);
	ExtrinsicData::insert(extrinsic_index, utx.encode());
	storage::unhashed::put(well_known_keys::EXTRINSIC_INDEX, &(extrinsic_index + 1));
	result
}

/// Finalize the block.
pub fn finalize_block() -> Header {
	let extrinsic_index: u32 = storage::unhashed::take(well_known_keys::EXTRINSIC_INDEX).unwrap();
	let txs: Vec<_> = (0..extrinsic_index).map(ExtrinsicData::take).collect();
	let extrinsics_root = trie::blake2_256_ordered_root(txs).into();
	let number = <Number>::take().expect("Number is set by `initialize_block`");
	let parent_hash = <ParentHash>::take();
	let mut digest = <StorageDigest>::take().expect("StorageDigest is set by `initialize_block`");

	let o_new_authorities = <NewAuthorities>::take();
	let new_changes_trie_config = <NewChangesTrieConfig>::take();

	// This MUST come after all changes to storage are done. Otherwise we will fail the
	// “Storage root does not match that calculated” assertion.
	let storage_root = Hash::decode(&mut &storage_root()[..])
		.expect("`storage_root` is a valid hash");
	let storage_changes_root = storage_changes_root(&parent_hash.encode())
		.map(|r| Hash::decode(&mut &r[..]).expect("`storage_changes_root` is a valid hash"));

	if let Some(storage_changes_root) = storage_changes_root {
		digest.push(generic::DigestItem::ChangesTrieRoot(storage_changes_root));
	}

	if let Some(new_authorities) = o_new_authorities {
		digest.push(generic::DigestItem::Consensus(*b"aura", new_authorities.encode()));
		digest.push(generic::DigestItem::Consensus(*b"babe", new_authorities.encode()));
	}

	if let Some(new_config) = new_changes_trie_config {
		digest.push(generic::DigestItem::ChangesTrieSignal(
			generic::ChangesTrieSignal::NewConfiguration(new_config)
		));
	}

	Header {
		number,
		extrinsics_root,
		state_root: storage_root,
		parent_hash,
		digest,
	}
}

#[inline(always)]
fn check_signature(utx: &Extrinsic) -> Result<(), TransactionValidityError> {
	use sp_runtime::traits::BlindCheckable;
	utx.clone().check().map_err(|_| InvalidTransaction::BadProof.into()).map(|_| ())
}

fn execute_transaction_backend(utx: &Extrinsic, extrinsic_index: u32) -> ApplyExtrinsicResult {
	check_signature(utx)?;
	match utx {
		Extrinsic::Transfer { exhaust_resources_when_not_first: true, .. } if extrinsic_index != 0 =>
			Err(InvalidTransaction::ExhaustsResources.into()),
		Extrinsic::Transfer { ref transfer, .. } =>
			execute_transfer_backend(transfer),
		Extrinsic::AuthoritiesChange(ref new_auth) =>
			execute_new_authorities_backend(new_auth),
		Extrinsic::IncludeData(_) => Ok(Ok(())),
		Extrinsic::StorageChange(key, value) =>
			execute_storage_change(key, value.as_ref().map(|v| &**v)),
		Extrinsic::ChangesTrieConfigUpdate(ref new_config) =>
			execute_changes_trie_config_update(new_config.clone()),
		Extrinsic::OffchainIndexSet(key, value) => {
			sp_io::offchain_index::set(&key, &value);
			Ok(Ok(()))
		},
		Extrinsic::OffchainIndexClear(key) => {
			sp_io::offchain_index::clear(&key);
			Ok(Ok(()))
		}
	}
}

fn execute_transfer_backend(tx: &Transfer) -> ApplyExtrinsicResult {
	// check nonce
	let nonce_key = tx.from.to_keyed_vec(NONCE_OF);
	let expected_nonce: u64 = storage::hashed::get_or(&blake2_256, &nonce_key, 0);
	if !(tx.nonce == expected_nonce) {
		return Err(InvalidTransaction::Stale.into());
	}

	// increment nonce in storage
	storage::hashed::put(&blake2_256, &nonce_key, &(expected_nonce + 1));

	// check sender balance
	let from_balance_key = tx.from.to_keyed_vec(BALANCE_OF);
	let from_balance: u64 = storage::hashed::get_or(&blake2_256, &from_balance_key, 0);

	// enact transfer
	if !(tx.amount <= from_balance) {
		return Err(InvalidTransaction::Payment.into());
	}
	let to_balance_key = tx.to.to_keyed_vec(BALANCE_OF);
	let to_balance: u64 = storage::hashed::get_or(&blake2_256, &to_balance_key, 0);
	storage::hashed::put(&blake2_256, &from_balance_key, &(from_balance - tx.amount));
	storage::hashed::put(&blake2_256, &to_balance_key, &(to_balance + tx.amount));
	Ok(Ok(()))
}

fn execute_new_authorities_backend(new_authorities: &[AuthorityId]) -> ApplyExtrinsicResult {
	NewAuthorities::put(new_authorities.to_vec());
	Ok(Ok(()))
}

fn execute_storage_change(key: &[u8], value: Option<&[u8]>) -> ApplyExtrinsicResult {
	match value {
		Some(value) => storage::unhashed::put_raw(key, value),
		None => storage::unhashed::kill(key),
	}
	Ok(Ok(()))
}

fn execute_changes_trie_config_update(new_config: Option<ChangesTrieConfiguration>) -> ApplyExtrinsicResult {
	match new_config.clone() {
		Some(new_config) => storage::unhashed::put_raw(
			well_known_keys::CHANGES_TRIE_CONFIG,
			&new_config.encode(),
		),
		None => storage::unhashed::kill(well_known_keys::CHANGES_TRIE_CONFIG),
	}
	<NewChangesTrieConfig>::put(new_config);
	Ok(Ok(()))
}

#[cfg(feature = "std")]
fn info_expect_equal_hash(given: &Hash, expected: &Hash) {
	use sp_core::hexdisplay::HexDisplay;
	if given != expected {
		println!(
			"Hash: given={}, expected={}",
			HexDisplay::from(given.as_fixed_bytes()),
			HexDisplay::from(expected.as_fixed_bytes()),
		);
	}
}

#[cfg(not(feature = "std"))]
fn info_expect_equal_hash(given: &Hash, expected: &Hash) {
	if given != expected {
		sp_runtime::print("Hash not equal");
		sp_runtime::print(given.as_bytes());
		sp_runtime::print(expected.as_bytes());
	}
}

