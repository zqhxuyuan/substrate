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

//! Runtime Modules shared primitive types.

#![warn(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]
// to allow benchmarking
#![cfg_attr(feature = "bench", feature(test))]
#[cfg(feature = "bench")]
extern crate test;

#[doc(hidden)]
pub use codec;
#[cfg(feature = "std")]
#[doc(hidden)]
pub use serde;
#[doc(hidden)]
pub use sp_std;

#[doc(hidden)]
pub use paste;

#[doc(hidden)]
pub use sp_application_crypto as app_crypto;

#[cfg(feature = "std")]
pub use sp_core::storage::{Storage, StorageChild};

use sp_core::{crypto::{self, Public}, ecdsa, ecdsa2, ed25519, hash::{H256, H512}, sr25519, H160};
use sp_std::{convert::TryFrom, prelude::*};

use codec::{Decode, Encode};

pub mod curve;
pub mod generic;
mod multiaddress;
pub mod offchain;
pub mod runtime_logger;
mod runtime_string;
#[cfg(feature = "std")]
pub mod testing;
pub mod traits;
pub mod transaction_validity;
// #[cfg(feature = "std")]
// mod signature;
// #[cfg(feature = "std")]
// mod signer;
// pub use crate::signature::*;
// pub use crate::signer::*;

pub use crate::runtime_string::*;

// Re-export Multiaddress
pub use multiaddress::MultiAddress;

/// Re-export these since they're only "kind of" generic.
pub use generic::{Digest, DigestItem};

pub use sp_application_crypto::{BoundToRuntimeAppPublic, RuntimeAppPublic};
/// Re-export this since it's part of the API of this crate.
pub use sp_core::{
	crypto::{key_types, AccountId32, CryptoType, CryptoTypeId, KeyTypeId},
	TypeId,
};

/// Re-export `RuntimeDebug`, to avoid dependency clutter.
pub use sp_core::RuntimeDebug;

/// Re-export big_uint stuff.
pub use sp_arithmetic::biguint;
/// Re-export 128 bit helpers.
pub use sp_arithmetic::helpers_128bit;
/// Re-export top-level arithmetic stuff.
pub use sp_arithmetic::{
	traits::SaturatedConversion, FixedI128, FixedI64, FixedPointNumber, FixedPointOperand,
	FixedU128, InnerOf, PerThing, PerU16, Perbill, Percent, Permill, Perquintill, Rational128,
	UpperOf,
};

pub use either::Either;
use sha3::{Keccak256, Digest as ShaDigest};

/// An abstraction over justification for a block's validity under a consensus algorithm.
///
/// Essentially a finality proof. The exact formulation will vary between consensus
/// algorithms. In the case where there are multiple valid proofs, inclusion within
/// the block itself would allow swapping justifications to change the block's hash
/// (and thus fork the chain). Sending a `Justification` alongside a block instead
/// bypasses this problem.
///
/// Each justification is provided as an encoded blob, and is tagged with an ID
/// to identify the consensus engine that generated the proof (we might have
/// multiple justifications from different engines for the same block).
pub type Justification = (ConsensusEngineId, EncodedJustification);

/// The encoded justification specific to a consensus engine.
pub type EncodedJustification = Vec<u8>;

/// Collection of justifications for a given block, multiple justifications may
/// be provided by different consensus engines for the same block.
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Justifications(Vec<Justification>);

impl Justifications {
	/// Return an iterator over the justifications.
	pub fn iter(&self) -> impl Iterator<Item = &Justification> {
		self.0.iter()
	}

	/// Append a justification. Returns false if a justification with the same
	/// `ConsensusEngineId` already exists, in which case the justification is
	/// not inserted.
	pub fn append(&mut self, justification: Justification) -> bool {
		if self.get(justification.0).is_some() {
			return false
		}
		self.0.push(justification);
		true
	}

	/// Return the encoded justification for the given consensus engine, if it
	/// exists.
	pub fn get(&self, engine_id: ConsensusEngineId) -> Option<&EncodedJustification> {
		self.iter().find(|j| j.0 == engine_id).map(|j| &j.1)
	}

	/// Return a copy of the encoded justification for the given consensus
	/// engine, if it exists.
	pub fn into_justification(self, engine_id: ConsensusEngineId) -> Option<EncodedJustification> {
		self.into_iter().find(|j| j.0 == engine_id).map(|j| j.1)
	}
}

impl IntoIterator for Justifications {
	type Item = Justification;
	type IntoIter = sp_std::vec::IntoIter<Self::Item>;

	fn into_iter(self) -> Self::IntoIter {
		self.0.into_iter()
	}
}

impl From<Justification> for Justifications {
	fn from(justification: Justification) -> Self {
		Self(vec![justification])
	}
}

use traits::{Lazy, Verify};

use crate::traits::IdentifyAccount;
#[cfg(feature = "std")]
pub use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Complex storage builder stuff.
#[cfg(feature = "std")]
pub trait BuildStorage {
	/// Build the storage out of this builder.
	fn build_storage(&self) -> Result<sp_core::storage::Storage, String> {
		let mut storage = Default::default();
		self.assimilate_storage(&mut storage)?;
		Ok(storage)
	}
	/// Assimilate the storage for this module into pre-existing overlays.
	fn assimilate_storage(&self, storage: &mut sp_core::storage::Storage) -> Result<(), String>;
}

/// Something that can build the genesis storage of a module.
#[cfg(feature = "std")]
pub trait BuildModuleGenesisStorage<T, I>: Sized {
	/// Create the module genesis storage into the given `storage` and `child_storage`.
	fn build_module_genesis_storage(
		&self,
		storage: &mut sp_core::storage::Storage,
	) -> Result<(), String>;
}

#[cfg(feature = "std")]
impl BuildStorage for sp_core::storage::Storage {
	fn assimilate_storage(&self, storage: &mut sp_core::storage::Storage) -> Result<(), String> {
		storage.top.extend(self.top.iter().map(|(k, v)| (k.clone(), v.clone())));
		for (k, other_map) in self.children_default.iter() {
			let k = k.clone();
			if let Some(map) = storage.children_default.get_mut(&k) {
				map.data.extend(other_map.data.iter().map(|(k, v)| (k.clone(), v.clone())));
				if !map.child_info.try_update(&other_map.child_info) {
					return Err("Incompatible child info update".to_string())
				}
			} else {
				storage.children_default.insert(k, other_map.clone());
			}
		}
		Ok(())
	}
}

#[cfg(feature = "std")]
impl BuildStorage for () {
	fn assimilate_storage(&self, _: &mut sp_core::storage::Storage) -> Result<(), String> {
		Err("`assimilate_storage` not implemented for `()`".into())
	}
}

/// Consensus engine unique ID.
pub type ConsensusEngineId = [u8; 4];

/// Signature verify that can work with any known signature types..
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Eq, PartialEq, Clone, Encode, Decode, RuntimeDebug)]
pub enum MultiSignature {
	/// An Ed25519 signature.
	Ed25519(ed25519::Signature),
	/// An Sr25519 signature.
	Sr25519(sr25519::Signature),
	/// An ECDSA/SECP256k1 signature.
	Ecdsa(ecdsa::Signature),
	/// eth signature
	EthereumSignature(ecdsa2::Signature2)
}

/// Public key for any known crypto algorithm.
#[derive(Eq, PartialEq, Ord, PartialOrd, Clone, Encode, Decode, RuntimeDebug)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub enum MultiSigner {
	/// An Ed25519 identity.
	Ed25519(ed25519::Public),
	/// An Sr25519 identity.
	Sr25519(sr25519::Public),
	/// An SECP256k1/ECDSA identity (actually, the Blake2 hash of the compressed pub key).
	Ecdsa(ecdsa::Public),
	// EthereumSigner(ecdsa2::Public2),
	EthereumSigner2(EthSigner)
}

/// Public key for an Ethereum / H160 compatible account
#[derive(Eq, PartialEq, Ord, PartialOrd, Clone, Copy, Encode, Decode, sp_core::RuntimeDebug)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct EthSigner([u8; 32]);

impl From<ed25519::Signature> for MultiSignature {
	fn from(x: ed25519::Signature) -> Self {
		Self::Ed25519(x)
	}
}
impl From<sr25519::Signature> for MultiSignature {
	fn from(x: sr25519::Signature) -> Self {
		Self::Sr25519(x)
	}
}
impl From<ecdsa::Signature> for MultiSignature {
	fn from(x: ecdsa::Signature) -> Self {
		Self::Ecdsa(x)
	}
}
impl From<ecdsa2::Signature2> for MultiSignature {
	fn from(x: ecdsa2::Signature2) -> Self {
		Self::EthereumSignature(x)
	}
}

impl TryFrom<MultiSignature> for ed25519::Signature {
	type Error = ();
	fn try_from(m: MultiSignature) -> Result<Self, Self::Error> {
		if let MultiSignature::Ed25519(x) = m {
			Ok(x)
		} else {
			Err(())
		}
	}
}

impl TryFrom<MultiSignature> for sr25519::Signature {
	type Error = ();
	fn try_from(m: MultiSignature) -> Result<Self, Self::Error> {
		if let MultiSignature::Sr25519(x) = m {
			Ok(x)
		} else {
			Err(())
		}
	}
}

impl TryFrom<MultiSignature> for ecdsa::Signature {
	type Error = ();
	fn try_from(m: MultiSignature) -> Result<Self, Self::Error> {
		if let MultiSignature::Ecdsa(x) = m {
			Ok(x)
		} else {
			Err(())
		}
	}
}

impl TryFrom<MultiSignature> for ecdsa2::Signature2 {
	type Error = ();
	fn try_from(m: MultiSignature) -> Result<Self, Self::Error> {
		if let MultiSignature::EthereumSignature(x) = m {
			Ok(x)
		} else {
			Err(())
		}
	}
}

impl Default for MultiSignature {
	fn default() -> Self {
		Self::Ed25519(Default::default())
	}
}

impl Default for MultiSigner {
	fn default() -> Self {
		Self::Ed25519(Default::default())
	}
}

/// NOTE: This implementations is required by `SimpleAddressDeterminer`,
/// we convert the hash into some AccountId, it's fine to use any scheme.
impl<T: Into<H256>> crypto::UncheckedFrom<T> for MultiSigner {
	fn unchecked_from(x: T) -> Self {
		ed25519::Public::unchecked_from(x.into()).into()
	}
}

impl AsRef<[u8]> for MultiSigner {
	fn as_ref(&self) -> &[u8] {
		match *self {
			Self::Ed25519(ref who) => who.as_ref(),
			Self::Sr25519(ref who) => who.as_ref(),
			Self::Ecdsa(ref who) => who.as_ref(),
			// Self::EthereumSigner(ref who) => who.as_ref(),
			Self::EthereumSigner2(ref who) => who.as_ref(),
		}
	}
}

impl traits::IdentifyAccount for MultiSigner {
	type AccountId = AccountId32;
	fn into_account(self) -> AccountId32 {
		match self {
			Self::Ed25519(who) => <[u8; 32]>::from(who).into(),
			Self::Sr25519(who) => <[u8; 32]>::from(who).into(),
			Self::Ecdsa(who) => sp_io::hashing::blake2_256(who.as_ref()).into(),
			// the signer's Public2 has 33 bytes, can't directly use: <[u8; 32]>::from(who).into()
			// as Ecdsa, first hash to 256 bits, which has type: [u8; 32], then into() AccountId32
			// Self::EthereumSigner(who) => sp_io::hashing::keccak_256(who.as_ref()).into(),
			Self::EthereumSigner2(who) => <[u8; 32]>::from(who).into(),
		}
	}
}

impl From<ed25519::Public> for MultiSigner {
	fn from(x: ed25519::Public) -> Self {
		Self::Ed25519(x)
	}
}
impl From<sr25519::Public> for MultiSigner {
	fn from(x: sr25519::Public) -> Self {
		Self::Sr25519(x)
	}
}
impl From<ecdsa::Public> for MultiSigner {
	fn from(x: ecdsa::Public) -> Self {
		Self::Ecdsa(x)
	}
}
//todo: ecdsa2::Public2 to EthereumSigner2
impl From<ecdsa2::Public2> for MultiSigner {
	fn from(x: ecdsa2::Public2) -> Self {
		let decompressed =
			secp256k1::PublicKey::parse_slice(&x.0, Some(secp256k1::PublicKeyFormat::Compressed))
				.expect("Wrong compressed public key provided")
				.serialize();
		let mut m = [0u8; 64];
		m.copy_from_slice(&decompressed[1..65]);
		let acc = H256::from_slice(Keccak256::digest(&m).as_slice());
		let eth = EthSigner(acc.0);
		Self::EthereumSigner2(eth)
		// Self::EthereumSigner(x)
	}
}
impl From<EthSigner> for MultiSigner {
	fn from(x: EthSigner) -> Self {
		Self::EthereumSigner2(x)
	}
}

impl TryFrom<MultiSigner> for ed25519::Public {
	type Error = ();
	fn try_from(m: MultiSigner) -> Result<Self, Self::Error> {
		if let MultiSigner::Ed25519(x) = m {
			Ok(x)
		} else {
			Err(())
		}
	}
}

impl TryFrom<MultiSigner> for sr25519::Public {
	type Error = ();
	fn try_from(m: MultiSigner) -> Result<Self, Self::Error> {
		if let MultiSigner::Sr25519(x) = m {
			Ok(x)
		} else {
			Err(())
		}
	}
}

impl TryFrom<MultiSigner> for ecdsa::Public {
	type Error = ();
	fn try_from(m: MultiSigner) -> Result<Self, Self::Error> {
		if let MultiSigner::Ecdsa(x) = m {
			Ok(x)
		} else {
			Err(())
		}
	}
}

// impl TryFrom<MultiSigner> for ecdsa2::Public2 {
// 	type Error = ();
// 	fn try_from(m: MultiSigner) -> Result<Self, Self::Error> {
// 		if let MultiSigner::EthereumSigner(x) = m {
// 			Ok(x)
// 		} else {
// 			Err(())
// 		}
// 	}
// }

impl TryFrom<MultiSigner> for EthSigner {
	type Error = ();
	fn try_from(m: MultiSigner) -> Result<Self, Self::Error> {
		if let MultiSigner::EthereumSigner2(x) = m {
			Ok(x)
		} else {
			Err(())
		}
	}
}

impl AsRef<[u8]> for EthSigner {
	fn as_ref(&self) -> &[u8] {
		&self.0[..]
	}
}

impl From<EthSigner> for [u8; 32] {
	fn from(x: EthSigner) -> [u8; 32] {
		x.0
	}
}

#[cfg(feature = "std")]
impl std::fmt::Display for MultiSigner {
	fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
		match *self {
			Self::Ed25519(ref who) => write!(fmt, "ed25519: {}", who),
			Self::Sr25519(ref who) => write!(fmt, "sr25519: {}", who),
			Self::Ecdsa(ref who) => write!(fmt, "ecdsa: {}", who),
			// Self::EthereumSigner(ref who) => write!(fmt, "ethereum: {}", who),
			Self::EthereumSigner2(ref who) => write!(fmt, "ethereum2: {:?}", who),
		}
	}
}

impl Verify for MultiSignature {
	type Signer = MultiSigner;
	fn verify<L: Lazy<[u8]>>(&self, mut msg: L, signer: &AccountId32) -> bool {
		match (self, signer) {
			(Self::Ed25519(ref sig), who) =>
				sig.verify(msg, &ed25519::Public::from_slice(who.as_ref())),
			(Self::Sr25519(ref sig), who) =>
				sig.verify(msg, &sr25519::Public::from_slice(who.as_ref())),
			(Self::Ecdsa(ref sig), who) => {
				let m = sp_io::hashing::blake2_256(msg.get());
				match sp_io::crypto::secp256k1_ecdsa_recover_compressed(sig.as_ref(), &m) {
					Ok(pubkey) =>
						&sp_io::hashing::blake2_256(pubkey.as_ref()) ==
							<dyn AsRef<[u8; 32]>>::as_ref(who),
					_ => false,
				}
			},
			// eth signer is H160, here use unified AccountId32==H256
			(Self::EthereumSignature(ref sig), who) => {
				match sp_io::crypto::secp256k1_ecdsa_recover(
					sig.as_ref(),
				    &sp_io::hashing::blake2_256(msg.get())
				) {
					Ok(pubkey) => {
						H256::from_slice(Keccak256::digest(&pubkey).as_slice())
							== H256::from_slice(who.as_ref())
					}
					Err(sp_io::EcdsaVerifyError::BadRS) |
					Err(sp_io::EcdsaVerifyError::BadV) |
					Err(sp_io::EcdsaVerifyError::BadSignature) => {
						log::error!(target: "evm", "Error recovering");
						false
					}
				}
			}
		}
	}
}

/// Signature verify that can work with any known signature types..
#[derive(Eq, PartialEq, Clone, Default, Encode, Decode, RuntimeDebug)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub struct AnySignature(pub H512);

impl From<sr25519::Signature> for AnySignature {
	fn from(s: sr25519::Signature) -> Self {
		Self(s.into())
	}
}

impl From<ed25519::Signature> for AnySignature {
	fn from(s: ed25519::Signature) -> Self {
		Self(s.into())
	}
}

impl Verify for AnySignature {
	type Signer = sr25519::Public;
	fn verify<L: Lazy<[u8]>>(&self, mut msg: L, signer: &sr25519::Public) -> bool {
		let msg = msg.get();
		sr25519::Signature::try_from(self.0.as_fixed_bytes().as_ref())
			.map(|s| s.verify(msg, signer))
			.unwrap_or(false) ||
			ed25519::Signature::try_from(self.0.as_fixed_bytes().as_ref())
				.map(|s| s.verify(msg, &ed25519::Public::from_slice(signer.as_ref())))
				.unwrap_or(false)
	}
}

impl From<DispatchError> for DispatchOutcome {
	fn from(err: DispatchError) -> Self {
		Err(err)
	}
}

/// This is the legacy return type of `Dispatchable`. It is still exposed for compatibility reasons.
/// The new return type is `DispatchResultWithInfo`. FRAME runtimes should use
/// `frame_support::dispatch::DispatchResult`.
pub type DispatchResult = sp_std::result::Result<(), DispatchError>;

/// Return type of a `Dispatchable` which contains the `DispatchResult` and additional information
/// about the `Dispatchable` that is only known post dispatch.
pub type DispatchResultWithInfo<T> = sp_std::result::Result<T, DispatchErrorWithPostInfo<T>>;

/// Reason why a dispatch call failed.
#[derive(Eq, Clone, Copy, Encode, Decode, Debug)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub enum DispatchError {
	/// Some error occurred.
	Other(
		#[codec(skip)]
		#[cfg_attr(feature = "std", serde(skip_deserializing))]
		&'static str,
	),
	/// Failed to lookup some data.
	CannotLookup,
	/// A bad origin.
	BadOrigin,
	/// A custom error in a module.
	Module {
		/// Module index, matching the metadata module index.
		index: u8,
		/// Module specific error value.
		error: u8,
		/// Optional error message.
		#[codec(skip)]
		#[cfg_attr(feature = "std", serde(skip_deserializing))]
		message: Option<&'static str>,
	},
	/// At least one consumer is remaining so the account cannot be destroyed.
	ConsumerRemaining,
	/// There are no providers so the account cannot be created.
	NoProviders,
	/// An error to do with tokens.
	Token(TokenError),
	/// An arithmetic error.
	Arithmetic(ArithmeticError),
}

/// Result of a `Dispatchable` which contains the `DispatchResult` and additional information about
/// the `Dispatchable` that is only known post dispatch.
#[derive(Eq, PartialEq, Clone, Copy, Encode, Decode, RuntimeDebug)]
pub struct DispatchErrorWithPostInfo<Info>
where
	Info: Eq + PartialEq + Clone + Copy + Encode + Decode + traits::Printable,
{
	/// Additional information about the `Dispatchable` which is only known post dispatch.
	pub post_info: Info,
	/// The actual `DispatchResult` indicating whether the dispatch was successful.
	pub error: DispatchError,
}

impl DispatchError {
	/// Return the same error but without the attached message.
	pub fn stripped(self) -> Self {
		match self {
			DispatchError::Module { index, error, message: Some(_) } =>
				DispatchError::Module { index, error, message: None },
			m => m,
		}
	}
}

impl<T, E> From<E> for DispatchErrorWithPostInfo<T>
where
	T: Eq + PartialEq + Clone + Copy + Encode + Decode + traits::Printable + Default,
	E: Into<DispatchError>,
{
	fn from(error: E) -> Self {
		Self { post_info: Default::default(), error: error.into() }
	}
}

impl From<crate::traits::LookupError> for DispatchError {
	fn from(_: crate::traits::LookupError) -> Self {
		Self::CannotLookup
	}
}

impl From<crate::traits::BadOrigin> for DispatchError {
	fn from(_: crate::traits::BadOrigin) -> Self {
		Self::BadOrigin
	}
}

/// Description of what went wrong when trying to complete an operation on a token.
#[derive(Eq, PartialEq, Clone, Copy, Encode, Decode, Debug)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub enum TokenError {
	/// Funds are unavailable.
	NoFunds,
	/// Account that must exist would die.
	WouldDie,
	/// Account cannot exist with the funds that would be given.
	BelowMinimum,
	/// Account cannot be created.
	CannotCreate,
	/// The asset in question is unknown.
	UnknownAsset,
	/// Funds exist but are frozen.
	Frozen,
	/// Operation is not supported by the asset.
	Unsupported,
}

impl From<TokenError> for &'static str {
	fn from(e: TokenError) -> &'static str {
		match e {
			TokenError::NoFunds => "Funds are unavailable",
			TokenError::WouldDie => "Account that must exist would die",
			TokenError::BelowMinimum => "Account cannot exist with the funds that would be given",
			TokenError::CannotCreate => "Account cannot be created",
			TokenError::UnknownAsset => "The asset in question is unknown",
			TokenError::Frozen => "Funds exist but are frozen",
			TokenError::Unsupported => "Operation is not supported by the asset",
		}
	}
}

impl From<TokenError> for DispatchError {
	fn from(e: TokenError) -> DispatchError {
		Self::Token(e)
	}
}

/// Arithmetic errors.
#[derive(Eq, PartialEq, Clone, Copy, Encode, Decode, Debug)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub enum ArithmeticError {
	/// Underflow.
	Underflow,
	/// Overflow.
	Overflow,
	/// Division by zero.
	DivisionByZero,
}

impl From<ArithmeticError> for &'static str {
	fn from(e: ArithmeticError) -> &'static str {
		match e {
			ArithmeticError::Underflow => "An underflow would occur",
			ArithmeticError::Overflow => "An overflow would occur",
			ArithmeticError::DivisionByZero => "Division by zero",
		}
	}
}

impl From<ArithmeticError> for DispatchError {
	fn from(e: ArithmeticError) -> DispatchError {
		Self::Arithmetic(e)
	}
}

impl From<&'static str> for DispatchError {
	fn from(err: &'static str) -> DispatchError {
		Self::Other(err)
	}
}

impl From<DispatchError> for &'static str {
	fn from(err: DispatchError) -> &'static str {
		match err {
			DispatchError::Other(msg) => msg,
			DispatchError::CannotLookup => "Cannot lookup",
			DispatchError::BadOrigin => "Bad origin",
			DispatchError::Module { message, .. } => message.unwrap_or("Unknown module error"),
			DispatchError::ConsumerRemaining => "Consumer remaining",
			DispatchError::NoProviders => "No providers",
			DispatchError::Token(e) => e.into(),
			DispatchError::Arithmetic(e) => e.into(),
		}
	}
}

impl<T> From<DispatchErrorWithPostInfo<T>> for &'static str
where
	T: Eq + PartialEq + Clone + Copy + Encode + Decode + traits::Printable,
{
	fn from(err: DispatchErrorWithPostInfo<T>) -> &'static str {
		err.error.into()
	}
}

impl traits::Printable for DispatchError {
	fn print(&self) {
		"DispatchError".print();
		match self {
			Self::Other(err) => err.print(),
			Self::CannotLookup => "Cannot lookup".print(),
			Self::BadOrigin => "Bad origin".print(),
			Self::Module { index, error, message } => {
				index.print();
				error.print();
				if let Some(msg) = message {
					msg.print();
				}
			},
			Self::ConsumerRemaining => "Consumer remaining".print(),
			Self::NoProviders => "No providers".print(),
			Self::Token(e) => {
				"Token error: ".print();
				<&'static str>::from(*e).print();
			},
			Self::Arithmetic(e) => {
				"Arithmetic error: ".print();
				<&'static str>::from(*e).print();
			},
		}
	}
}

impl<T> traits::Printable for DispatchErrorWithPostInfo<T>
where
	T: Eq + PartialEq + Clone + Copy + Encode + Decode + traits::Printable,
{
	fn print(&self) {
		self.error.print();
		"PostInfo: ".print();
		self.post_info.print();
	}
}

impl PartialEq for DispatchError {
	fn eq(&self, other: &Self) -> bool {
		use DispatchError::*;

		match (self, other) {
			(CannotLookup, CannotLookup) |
			(BadOrigin, BadOrigin) |
			(ConsumerRemaining, ConsumerRemaining) |
			(NoProviders, NoProviders) => true,

			(Token(l), Token(r)) => l == r,
			(Other(l), Other(r)) => l == r,
			(Arithmetic(l), Arithmetic(r)) => l == r,

			(
				Module { index: index_l, error: error_l, .. },
				Module { index: index_r, error: error_r, .. },
			) => (index_l == index_r) && (error_l == error_r),

			_ => false,
		}
	}
}

/// This type specifies the outcome of dispatching a call to a module.
///
/// In case of failure an error specific to the module is returned.
///
/// Failure of the module call dispatching doesn't invalidate the extrinsic and it is still included
/// in the block, therefore all state changes performed by the dispatched call are still persisted.
///
/// For example, if the dispatching of an extrinsic involves inclusion fee payment then these
/// changes are going to be preserved even if the call dispatched failed.
pub type DispatchOutcome = Result<(), DispatchError>;

/// The result of applying of an extrinsic.
///
/// This type is typically used in the context of `BlockBuilder` to signal that the extrinsic
/// in question cannot be included.
///
/// A block containing extrinsics that have a negative inclusion outcome is invalid. A negative
/// result can only occur during the block production, where such extrinsics are detected and
/// removed from the block that is being created and the transaction pool.
///
/// To rehash: every extrinsic in a valid block must return a positive `ApplyExtrinsicResult`.
///
/// Examples of reasons preventing inclusion in a block:
/// - More block weight is required to process the extrinsic than is left in the block being built.
///   This doesn't necessarily mean that the extrinsic is invalid, since it can still be included in
///   the next block if it has enough spare weight available.
/// - The sender doesn't have enough funds to pay the transaction inclusion fee. Including such a
///   transaction in the block doesn't make sense.
/// - The extrinsic supplied a bad signature. This transaction won't become valid ever.
pub type ApplyExtrinsicResult =
	Result<DispatchOutcome, transaction_validity::TransactionValidityError>;

/// Same as `ApplyExtrinsicResult` but augmented with `PostDispatchInfo` on success.
pub type ApplyExtrinsicResultWithInfo<T> =
	Result<DispatchResultWithInfo<T>, transaction_validity::TransactionValidityError>;

/// Verify a signature on an encoded value in a lazy manner. This can be
/// an optimization if the signature scheme has an "unsigned" escape hash.
pub fn verify_encoded_lazy<V: Verify, T: codec::Encode>(
	sig: &V,
	item: &T,
	signer: &<V::Signer as IdentifyAccount>::AccountId,
) -> bool {
	// The `Lazy<T>` trait expresses something like `X: FnMut<Output = for<'a> &'a T>`.
	// unfortunately this is a lifetime relationship that can't
	// be expressed without generic associated types, better unification of HRTBs in type position,
	// and some kind of integration into the Fn* traits.
	struct LazyEncode<F> {
		inner: F,
		encoded: Option<Vec<u8>>,
	}

	impl<F: Fn() -> Vec<u8>> traits::Lazy<[u8]> for LazyEncode<F> {
		fn get(&mut self) -> &[u8] {
			self.encoded.get_or_insert_with(&self.inner).as_slice()
		}
	}

	sig.verify(LazyEncode { inner: || item.encode(), encoded: None }, signer)
}

/// Checks that `$x` is equal to `$y` with an error rate of `$error`.
///
/// # Example
///
/// ```rust
/// # fn main() {
/// sp_runtime::assert_eq_error_rate!(10, 10, 0);
/// sp_runtime::assert_eq_error_rate!(10, 11, 1);
/// sp_runtime::assert_eq_error_rate!(12, 10, 2);
/// # }
/// ```
///
/// ```rust,should_panic
/// # fn main() {
/// sp_runtime::assert_eq_error_rate!(12, 10, 1);
/// # }
/// ```
#[macro_export]
#[cfg(feature = "std")]
macro_rules! assert_eq_error_rate {
	($x:expr, $y:expr, $error:expr $(,)?) => {
		assert!(
			($x) >= (($y) - ($error)) && ($x) <= (($y) + ($error)),
			"{:?} != {:?} (with error rate {:?})",
			$x,
			$y,
			$error,
		);
	};
}

/// Simple blob to hold an extrinsic without committing to its format and ensure it is serialized
/// correctly.
#[derive(PartialEq, Eq, Clone, Default, Encode, Decode)]
pub struct OpaqueExtrinsic(Vec<u8>);

impl OpaqueExtrinsic {
	/// Convert an encoded extrinsic to an `OpaqueExtrinsic`.
	pub fn from_bytes(mut bytes: &[u8]) -> Result<Self, codec::Error> {
		Self::decode(&mut bytes)
	}
}

#[cfg(feature = "std")]
impl parity_util_mem::MallocSizeOf for OpaqueExtrinsic {
	fn size_of(&self, ops: &mut parity_util_mem::MallocSizeOfOps) -> usize {
		self.0.size_of(ops)
	}
}

impl sp_std::fmt::Debug for OpaqueExtrinsic {
	#[cfg(feature = "std")]
	fn fmt(&self, fmt: &mut sp_std::fmt::Formatter) -> sp_std::fmt::Result {
		write!(fmt, "{}", sp_core::hexdisplay::HexDisplay::from(&self.0))
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _fmt: &mut sp_std::fmt::Formatter) -> sp_std::fmt::Result {
		Ok(())
	}
}

#[cfg(feature = "std")]
impl ::serde::Serialize for OpaqueExtrinsic {
	fn serialize<S>(&self, seq: S) -> Result<S::Ok, S::Error>
	where
		S: ::serde::Serializer,
	{
		codec::Encode::using_encoded(&self.0, |bytes| ::sp_core::bytes::serialize(bytes, seq))
	}
}

#[cfg(feature = "std")]
impl<'a> ::serde::Deserialize<'a> for OpaqueExtrinsic {
	fn deserialize<D>(de: D) -> Result<Self, D::Error>
	where
		D: ::serde::Deserializer<'a>,
	{
		let r = ::sp_core::bytes::deserialize(de)?;
		Decode::decode(&mut &r[..])
			.map_err(|e| ::serde::de::Error::custom(format!("Decode error: {}", e)))
	}
}

impl traits::Extrinsic for OpaqueExtrinsic {
	type Call = ();
	type SignaturePayload = ();
	type Address = ();
}

/// Print something that implements `Printable` from the runtime.
pub fn print(print: impl traits::Printable) {
	print.print();
}

/// Batching session.
///
/// To be used in runtime only. Outside of runtime, just construct
/// `BatchVerifier` directly.
#[must_use = "`verify()` needs to be called to finish batch signature verification!"]
pub struct SignatureBatching(bool);

impl SignatureBatching {
	/// Start new batching session.
	pub fn start() -> Self {
		sp_io::crypto::start_batch_verify();
		SignatureBatching(false)
	}

	/// Verify all signatures submitted during the batching session.
	#[must_use]
	pub fn verify(mut self) -> bool {
		self.0 = true;
		sp_io::crypto::finish_batch_verify()
	}
}

impl Drop for SignatureBatching {
	fn drop(&mut self) {
		// Sanity check. If user forgets to actually call `verify()`.
		//
		// We should not panic if the current thread is already panicking,
		// because Rust otherwise aborts the process.
		if !self.0 && !sp_std::thread::panicking() {
			panic!("Signature verification has not been called before `SignatureBatching::drop`")
		}
	}
}

/// Describes on what should happen with a storage transaction.
pub enum TransactionOutcome<R> {
	/// Commit the transaction.
	Commit(R),
	/// Rollback the transaction.
	Rollback(R),
}

impl<R> TransactionOutcome<R> {
	/// Convert into the inner type.
	pub fn into_inner(self) -> R {
		match self {
			Self::Commit(r) => r,
			Self::Rollback(r) => r,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use codec::{Decode, Encode};
	use sp_core::crypto::Pair;
	use crate::sp_std::convert::TryInto;
	use crate::traits::{BlakeTwo256, Hash};

	#[test]
	fn opaque_extrinsic_serialization() {
		let ex = super::OpaqueExtrinsic(vec![1, 2, 3, 4]);
		assert_eq!(serde_json::to_string(&ex).unwrap(), "\"0x1001020304\"".to_owned());
	}

	#[test]
	fn dispatch_error_encoding() {
		let error = DispatchError::Module { index: 1, error: 2, message: Some("error message") };
		let encoded = error.encode();
		let decoded = DispatchError::decode(&mut &encoded[..]).unwrap();
		assert_eq!(encoded, vec![3, 1, 2]);
		assert_eq!(decoded, DispatchError::Module { index: 1, error: 2, message: None });
	}

	#[test]
	fn dispatch_error_equality() {
		use DispatchError::*;

		let variants = vec![
			Other("foo"),
			Other("bar"),
			CannotLookup,
			BadOrigin,
			Module { index: 1, error: 1, message: None },
			Module { index: 1, error: 2, message: None },
			Module { index: 2, error: 1, message: None },
			ConsumerRemaining,
			NoProviders,
			Token(TokenError::NoFunds),
			Token(TokenError::WouldDie),
			Token(TokenError::BelowMinimum),
			Token(TokenError::CannotCreate),
			Token(TokenError::UnknownAsset),
			Token(TokenError::Frozen),
			Arithmetic(ArithmeticError::Overflow),
			Arithmetic(ArithmeticError::Underflow),
			Arithmetic(ArithmeticError::DivisionByZero),
		];
		for (i, variant) in variants.iter().enumerate() {
			for (j, other_variant) in variants.iter().enumerate() {
				if i == j {
					assert_eq!(variant, other_variant);
				} else {
					assert_ne!(variant, other_variant);
				}
			}
		}

		// Ignores `message` field in `Module` variant.
		assert_eq!(
			Module { index: 1, error: 1, message: Some("foo") },
			Module { index: 1, error: 1, message: None },
		);
	}

	#[test]
	fn multi_signature_ecdsa_verify_works() {
		let msg = &b"test-message"[..];
		let (pair, _) = ecdsa::Pair::generate();

		let signature = pair.sign(&msg);
		assert!(ecdsa::Pair::verify(&signature, msg, &pair.public()));

		let multi_sig = MultiSignature::from(signature);
		let multi_signer = MultiSigner::from(pair.public());
		assert!(multi_sig.verify(msg, &multi_signer.into_account()));

		let multi_signer = MultiSigner::from(pair.public());
		assert!(multi_sig.verify(msg, &multi_signer.into_account()));
	}

	#[test]
	fn test_spio_blake2_256() {
		// 要签名的消息
		let msg = &b"test-message"[..];
		// 随机生成一对公私钥
		let (pair2, _) = ecdsa::Pair::generate();

		// 签名
		let signature2 = pair2.sign(&msg);
		// 验证签名
		assert!(ecdsa::Pair::verify(&signature2, msg, &pair2.public()));

		let multi_signer = MultiSigner::from(pair2.public());
		let who = multi_signer.into_account();
		println!("who:{:?}", who);

		// 消息的摘要，blake2_256
		let m = sp_io::hashing::blake2_256(msg);

		// compressed
		match sp_io::crypto::secp256k1_ecdsa_recover_compressed(signature2.as_ref(), &m) {
			Ok(pubkey) => {
				let k1 = sp_io::hashing::blake2_256(pubkey.as_ref());
				let k2 = <dyn AsRef<[u8; 32]>>::as_ref(&who);
				// println!("k1:{:?}", k1);
				// println!("k2:{:?}", k2);
				assert_eq!(&k1, k2);
			}
			_ => {
			},
		};

		// un-compressed
		match sp_io::crypto::secp256k1_ecdsa_recover(signature2.as_ref(), &m) {
			Ok(pubkey) => {
				let k1 = sp_io::hashing::blake2_256(pubkey.as_ref());
				let k2 = <dyn AsRef<[u8; 32]>>::as_ref(&who);
				// println!("k1:{:?}", k1);
				// println!("k2:{:?}", k2);
				assert_ne!(&k1, k2);
			}
			_ => {
			},
		};
	}

	#[test]
	fn test_spio_keccak_256() {
		// 要签名的消息
		let msg = &b"test-message"[..];
		// 随机生成一对公私钥
		let (pair2, _) = ecdsa::Pair::generate();

		// 签名
		let signature2 = pair2.sign(&msg);
		// 验证签名
		assert!(ecdsa::Pair::verify(&signature2, msg, &pair2.public()));

		let multi_signer = MultiSigner::from(pair2.public());
		// into_account() 用的是 blake2_256 算法，所以下面如果用其他算法，则不等
		let who = multi_signer.into_account();
		println!("who:{:?}", who);

		// 消息的摘要，keccak_256
		let m = sp_io::hashing::keccak_256(msg);

		// compressed
		match sp_io::crypto::secp256k1_ecdsa_recover_compressed(signature2.as_ref(), &m) {
			Ok(pubkey) => {
				let k1 = sp_io::hashing::blake2_256(pubkey.as_ref());
				let k2 = <dyn AsRef<[u8; 32]>>::as_ref(&who);
				// println!("k1:{:?}", k1);
				// println!("k2:{:?}", k2);
				assert_ne!(&k1, k2);
			}
			_ => {
			},
		};

		// un-compressed
		match sp_io::crypto::secp256k1_ecdsa_recover(signature2.as_ref(), &m) {
			Ok(pubkey) => {
				let k1 = sp_io::hashing::blake2_256(pubkey.as_ref());
				let k2 = <dyn AsRef<[u8; 32]>>::as_ref(&who);
				// println!("k1:{:?}", k1);
				// println!("k2:{:?}", k2);
				assert_ne!(&k1, k2);
			}
			_ => {
			},
		};
	}

	fn mock_pair() -> (ecdsa2::Pair2, Vec<u8>) {
		let secret_key =
			hex::decode("502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875")
				.unwrap();
		let expected_hex_account = hex::decode("976f8456e4e2034179b284a23c0e0c8f6d3da50c").unwrap();

		let pair = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();
		(pair, expected_hex_account)
	}

	#[test]
	fn test_account_derivation_2() {
		// Test from https://asecuritysite.com/encryption/ethadd
		let secret_key =
			hex::decode("502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875")
				.unwrap();
		let expected_hex_account = hex::decode("976f8456e4e2034179b284a23c0e0c8f6d3da50c").unwrap();

		let pair = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();
		let public_key = pair.public();

		let account: MultiSigner = public_key.into();
		let acc = account.into_account();
		// acc:5CGhoq2Gx4vyiDDeTLCTRyw9hymToS7ix3tWA8xC3Yi41xae ==> to_ss58check
		println!("acc:{}", acc);
		// 0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c (5CGhoq2G...)
		println!("acc:{:?}", acc);
		// 0926c7af7ca5a41b6ca9ffde 976f8456e4e2034179b284a23c0e0c8f6d3da50c
		let acc_hex = hex::encode(acc);
		println!("hex:{:?}", acc_hex);
	}

	// 0926c7af7ca5a41b6ca9ffde 976f8456e4e2034179b284a23c0e0c8f6d3da50c
	// acc:0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c (5CGhoq2G...)
	// acc:17fe7561a96b5005c606028ef24ff3a9cf04c71dbc94d0b566f7a27b94566cac (5CcAYmLQ...)
	// acc:b4764bb2678e50431a87d7273cd0a705a2dc65e5b1e1205896baa2be8a07c6e0 (5G9KcmkW...)
	// acc:0673e488ea89e48bd6fe656f798d4ba9baf0064ec19eb4f0a1a45785ae9d6dfc (5CDAa7mt...)
	// acc:e78b67b7f8a7cc2bda83a45d773539d4ac0e786233d90a233654ccee26a613d9 (5HJJL85Y...)
	// acc:53ea0d19c73a382eaaf8988bff64d3f6efe2317ee2807d223a0bdc4c0c49dfdb (5DxjMnyq...)
	// acc:d1d120e2712d174059b1536ec0f0f4ab324c46e55d02d0033343b4be8a55532d (5GoozKtj...)
	// acc:422673dd5ceb426b0bd748897bf369283338e12c90514468aa3868a551ab2929 (5DZSSVcn...)
	// acc:b2fa19e6cf29e781ca16f575931f3600a299fd9b24cefb3bff79388d19804bea (5G7NgB1d...)
	// acc:04b5fadeeb343abeaec18b2ac41c5f1123eccd5ce233578b2e7ebd5693869d73 (5CAt7Dhc...)
	// acc:d1e53b49485c566f6cca44092898fe7a42be376c8bc7af536a940f7fd5add423 (5GouxhHh...)
	#[test]
	fn test_prefound_keys() {
		let vec = vec![
			"502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875",
			"5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133",
			"8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b",
			"0b6e18cafb6ed99687ec547bd28139cafdd2bffe70e6b688025de6b445aa5c5b",
			"39539ab1876910bbf3a223d84a29e28f1cb4e2e456503e7e91ed39b2e7223d68",
			"7dce9bc8babb68fec1409be38c8e1a52650206a7ed90ff956ae8a6d15eeaaef4",
			"b9d2ea9a615f3165812e8d44de0d24da9bbd164b65c4f0573e1ce2c8dbd9c8df",
			"96b8a38e12e1a31dee1eab2fffdf9d9990045f5b37e44d8cc27766ef294acf18",
			"0d6dcaaef49272a5411896be8ad16c01c35d6f8c18873387b71fbc734759b0ab",
			"4c42532034540267bf568198ccec4cb822a025da542861fcb146a5fab6433ff8",
			"94c49300a58d576011096bcb006aa06f5a91b34b4383891e8029c21dc39fbb8b",
		];
		for sk in vec {
			let secret_key = hex::decode(sk).unwrap();
			let pair = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();
			let public_key = pair.public();

			let account: MultiSigner = public_key.into();
			let acc = account.into_account();
			println!("acc:{:?}", acc);
		}

	}

	pub fn get_from_seed_pair<TPublic: Public>(seed: &str) -> TPublic::Pair {
		TPublic::Pair::from_string(&format!("//{}", seed), None)
			.expect("static values are valid; qed")
	}

	// Alice: 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
	// account:Alice
	//  acc_sr25519:d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d (5GrwvaEF...)
	//  acc_ed25519:88dc3417d5058ec4b4503e0c12ea1a0a89be200fe98922423d4334014fa6b0ee (5FA9nQDV...)
	//  acc_ethereum:80280ca34c7ad2f3e1642606e04cc55ebee1cbce552f250e85c57b70b2e2625b (5ExjtCWm...)
	//  acc_ecdsa:01e552298e47454041ea31273b4b630c64c104e4514aa3643490b8aaca9cf8ed (5C7C2Z5s...)
	// account:Bob
	//  acc_sr25519:8eaf04151687736326c9fea17e25fc5287613693c912909cb226aa4794f26a48 (5FHneW46...)
	//  acc_ed25519:d17c2d7823ebf260fd138f2d7e27d114c0145d968b5ff5006125f2414fadae69 (5GoNkf6W...)
	//  acc_ethereum:43127f1db08048ca8da5277825451a4de12dccc2d166922fa938e900fcc4ed24 (5DaeZSHd...)
	//  acc_ecdsa:3f6eaf1be5add88d84ca8b02d350074935dbf04f53f4287cb6abfd6b33413f8f (5DVskgSC...)
	#[test]
	fn test_pair_generate() {
		let accounts = vec!["Alice", "Bob"];
		for account in accounts {
			let pair1 = get_from_seed_pair::<sr25519::Public>(account);
			let pair2 = get_from_seed_pair::<ed25519::Public>(account);
			let pair3 = get_from_seed_pair::<ecdsa2::Public2>(account);
			let pair4 = get_from_seed_pair::<ecdsa::Public>(account);

			println!("account:{}", account);
			let public_key1 = pair1.public();
			let account: MultiSigner = public_key1.into();
			let acc = account.into_account();
			println!(" acc_sr25519:{:?}", acc);

			let public_key2 = pair2.public();
			let account: MultiSigner = public_key2.into();
			let acc = account.into_account();
			println!(" acc_ed25519:{:?}", acc);

			let public_key3 = pair3.public();
			let account: MultiSigner = public_key3.into();
			let acc = account.into_account();
			println!(" acc_ethereum:{:?}", acc);

			let public_key4 = pair4.public();
			let account: MultiSigner = public_key4.into();
			let acc = account.into_account();
			println!(" acc_ecdsa:{:?}", acc);
		}
	}

	// use keyring::AccountKeyring;
	// use keyring::Ed25519Keyring;

	#[test]
	fn test_key_pair() {
		// d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d (5GrwvaEF...)
		// 88dc3417d5058ec4b4503e0c12ea1a0a89be200fe98922423d4334014fa6b0ee (5FA9nQDV...)
		// let alice1: MultiSigner = AccountKeyring::Alice.pair().public().into();
		// let alice2: MultiSigner = Ed25519Keyring::Alice.pair().public().into();
		// print_info(alice1);
		// print_info(alice2);

		// 01e552298e47454041ea31273b4b630c64c104e4514aa3643490b8aaca9cf8ed (5C7C2Z5s...)
		let alice3: MultiSigner = get_from_seed_pair::<ecdsa::Public>("Alice").public().into();
		print_info(alice3);
		// 80280ca34c7ad2f3e1642606e04cc55ebee1cbce552f250e85c57b70b2e2625b (5ExjtCWm...)
		let alice3: MultiSigner = get_from_seed_pair::<ecdsa2::Public2>("Alice").public().into();
		print_info(alice3);
		println!();

		// 不能用上面打印的来作为 secret_key，并验证最后生成的地址是否一致！
		// 4afa5b7b13cabc96fe93321322c39f0c6e15cd15145c0decb2968c7bae0b0ae8 (5Dm1mkFh...)
		// let secret_key = hex::decode(
		// 	"01e552298e47454041ea31273b4b630c64c104e4514aa3643490b8aaca9cf8ed").unwrap();
		// let alice = ecdsa::Pair::from_seed_slice(&secret_key).unwrap();
		// let alice = alice.public().into();
		// print_info(alice);

		// ecdsa with-out btc/ethereum compatible
		// 7b678f9d2e94b7ea0aee59313ae4971499383d5767ad8a3d5fdc30f2b776bafa (5ErWWbKi...)
		let secret_key = hex::decode(
			"502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875").unwrap();
		let alice = ecdsa::Pair::from_seed_slice(&secret_key).unwrap();
		let alice = alice.public().into();
		print_info(alice);

		// ecdsa2 with ethereum compatible
		// 0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c (5CGhoq2G...) ✅
		let secret_key = hex::decode(
			"502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875").unwrap();
		let alice = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();
		let alice = alice.public().into();
		print_info(alice);
	}
	pub fn print_info(account: MultiSigner) {
		let acc = account.into_account();
		println!("{:?}", acc);
	}

	#[test]
	fn test_account_derivation_1() {
		// Test from https://asecuritysite.com/encryption/ethadd
		let secret_key =
			hex::decode("502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875")
				.unwrap();
		let expected_hex_account = hex::decode("976f8456e4e2034179b284a23c0e0c8f6d3da50c").unwrap();
		let pair = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();

		// Public Key is [u8; 33]
		let public_key = pair.public();

		// Public Key 转成 32 bytes，并且以地址格式有关 的步骤：
		// - 先将压缩的PublicKey[32]转成未压缩的PublicKey[64]
		// - Keccak256::digest(转成未压缩的PublicKey)
		// - H256::from_slice(digest)
		let decompressed =
			// parse_slice 的第一个参数是 public key, 类型为 [u8; 33]
			// 所以对应第二个参数格式为 Compressed，对应的是 compressed public key, 33
			// 长度 33 bytes 的 compressed public key 转为 长度 65 bytes 的 Full length Public key
			// 所以叫做 decompressed。如果是 64 bytes，则叫做 Raw Public key
			secp256k1::PublicKey::parse_slice(&public_key.0,
											  Some(secp256k1::PublicKeyFormat::Compressed))
				.expect("Wrong compressed public key provided")
				.serialize(); // 序列化方法，返回 [u8;65]
		let mut m = [0u8; 64];
		m.copy_from_slice(&decompressed[1..65]);

		// acc_256:0x0926c7af7ca5a41b6ca9ffde 976f8456e4e2034179b284a23c0e0c8f6d3da50c
		let acc_256 = H256::from_slice(Keccak256::digest(&m).as_slice());
		println!("acc_256:{:?}", acc_256);
		let account = H160::from(acc_256);
		println!("acc_160:{:?}", account);

		// 换成 blake2_256
		// 0e7f0ab21501c65d49b1388c283fdf1b2fbae75b0186cccad68a1080dd3312f0
		let h1 = sp_core::hashing::blake2_256(&m);
		let h1 = hex::encode(h1);
		println!("h1:{}", h1);

		let h2 = sp_io::hashing::blake2_256(&m);
		let h2 = hex::encode(h2);
		println!("h2:{}", h2);
		// assert_eq!(h1, h2);
		// assert_ne!(h1, acc_256);
	}

	#[test]
	fn test_signature_sign_verify() {
		// Test from https://asecuritysite.com/encryption/ethadd
		let expected_hex_account = hex::decode("976f8456e4e2034179b284a23c0e0c8f6d3da50c").unwrap();

		let secret_key =
			hex::decode("502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875")
				.unwrap();
		let pair = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();
		let public_key = pair.public();

		// 签名
		let msg = &b"test-message"[..];
		let signature = pair.sign(&msg);
		// 验证签名
		// 验证过程中，从签名进行恢复出来的实际pubkey，经过serialize_compressed后，也刚好是33bytes
		// 比较实际pubkey.serialize_compressed == 传入的public_key，相等则验证通过
		assert!(ecdsa2::Pair2::verify(&signature, msg, &public_key));
	}

	#[test]
	fn multi_signature_two_types_comp() {
		let msg = &b"test-message"[..];
		// let (pair, _) = ecdsa::Pair::generate();
		// let (pair2, _) = ecdsa2::Pair2::generate();

		// ecdsa exist in substrate already
		// ethereum signature use ecdsa2 which copy from ecdsa

		let secret_key =
			hex::decode("502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875")
				.unwrap();
		let pair = ecdsa::Pair::from_seed_slice(&secret_key).unwrap();
		let pair2 = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();

		let signature = pair.sign(&msg);
		assert!(ecdsa::Pair::verify(&signature, msg, &pair.public()));

		let signature2 = pair2.sign(&msg);
		assert!(ecdsa2::Pair2::verify(&signature2, msg, &pair2.public()));

		let multi_sig = MultiSignature::from(signature);
		let multi_signer = MultiSigner::from(pair.public());
		// println!("multi_signer1 signer :{}", multi_signer.clone());
		// 7b678f9d2e94b7ea0aee59313ae4971499383d5767ad8a3d5fdc30f2b776bafa
		println!("multi_signer1 account:{:?}", &multi_signer.into_account());

		let multi_sig2 = MultiSignature::from(signature2);
		let multi_signer2 = MultiSigner::from(pair2.public());
		// println!("multi_signer2 signer :{}", multi_signer2.clone());
		// 0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c
		println!("multi_signer2 account:{:?}", multi_signer2.into_account());
	}

	#[test]
	fn test_account_gen_messages() {
		use std::convert::TryInto;

		let expected_hex_account = hex::decode(
			"0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c").unwrap();
		let secret_key =
			hex::decode("502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875")
				.unwrap();
		let pair = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();
		let decompressed = secp256k1::PublicKey::parse_slice(&pair.public().0, Some(secp256k1::PublicKeyFormat::Compressed))
				.expect("Wrong compressed public key provided").serialize(); // 序列化方法，返回 [u8;65]
		let mut m = [0u8; 64];
		m.copy_from_slice(&decompressed[1..65]);
		let acc_256 = H256::from_slice(Keccak256::digest(&m).as_slice());
		println!("pubAddr:{:?}", acc_256);

		// 对任意消息进行签名
		//   BlakeTwo256: 0x0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c
		//   blake2_256(&): 0x0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c
		// let test_message = [100u8; 1];
		// let test_message = [100u8; 32]; // OK: BlakeTwo256

		// b"" -> &[u8; XXX]，只能使用 test_message，使用 & 会报错：expected slice `[u8]`, found `&[u8; 12]`
		//   expected reference `&[u8]` found reference `&&[u8; 12]`
		//   BlakeTwo256:
		//   blake2_256:
		// let test_message = b"test-message";
		// let signature: [u8; 65] = pair.sign(test_message).0;

		// 字符串转成&[u8]:
		//   BlakeTwo256: 0xc25994d96143831213a99e01ba402dc510ffe01e59feb53775be16184b570e14
		//   blake2_256:  0x0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c
		//   keccak_256:  0x9fa7277ed8c17709bef5c827301011460abe6ddea878ef45a8e071d4e042c258
		// let test_message = "test-message".as_bytes();

		// byte-string:
		//   BlakeTwo256: 0xc25994d96143831213a99e01ba402dc510ffe01e59feb53775be16184b570e14
		//   blake2_256:  0x0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c
		//   &b""[..] -> &[u8],	  可以使用 &test_message, 或者 test_message
		let test_message = &b"test-message"[..];

		// let signature: [u8; 65] = pair.sign(&test_message).as_ref().try_into().ok()?;
		let signature: [u8; 65] = pair.sign(&test_message).0;

		let pubkey = sp_io::crypto::secp256k1_ecdsa_recover(
			&signature,
			// BlakeTwo256::hash_of(&test_message).as_fixed_bytes(),
			&sp_io::hashing::blake2_256(&test_message),
		);
		match pubkey {
			Ok(pubkey) => {
				let account = H256::from_slice(Keccak256::digest(&pubkey).as_slice());
				println!("account:{:?}", account);

				let k2 = hex::encode(sp_io::hashing::keccak_256(pubkey.as_ref()));
				println!("keccak :{:?}", k2);
			}
			Err(_) => {}
		}
	}

	#[test]
	fn test_spio_verify_pubkey() {
		let msg = &b"test-message"[..];
		let expected_hex_account = hex::decode("0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c").unwrap();
		let secret_key =
			hex::decode("502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875").unwrap();
		let pair2 = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();

		// 签名与验证签名
		let signature2 = pair2.sign(&msg);
		assert!(ecdsa2::Pair2::verify(&signature2, msg, &pair2.public()));

		// 直接从 PublicKey 导出账户地址
		let multi_signer = MultiSigner::from(pair2.public());
		let who = multi_signer.into_account();
		println!("who:{:?}", who);

		match sp_io::crypto::secp256k1_ecdsa_recover(
			// &signature2.0,
			signature2.as_ref(),
			&sp_io::hashing::blake2_256(&msg)) {
			Ok(pubkey) => {
				// 得到pubkey后，只能使用kec: Keccak256::digest 或者 keccak_256 都可以解析出账户地址
				// 0x0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c
				let k1 = H256::from_slice(Keccak256::digest(&pubkey).as_slice());
				println!("acc:{:?}", k1);
				// 0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c
				let k2 = hex::encode(sp_io::hashing::keccak_256(pubkey.as_ref()));
				println!("kec:{:?}", k2);
				// 0e7f0ab21501c65d49b1388c283fdf1b2fbae75b0186cccad68a1080dd3312f0
				let k3 = hex::encode(sp_io::hashing::blake2_256(pubkey.as_ref()));
				println!("blk:{:?}", k3);
			}
			_ => {
			},
		};
	}

	#[test]
	fn test_spio_verify_pubkey_signature() {
		let msg = &b"test-message"[..];
		let expected_hex_account = hex::decode(
			"0926c7af7ca5a41b6ca9ffde976f8456e4e2034179b284a23c0e0c8f6d3da50c").unwrap();
		let secret_key =
			// hex::decode("502f97299c472b88754accd412b7c9a6062ef3186fba0c0388365e1edec24875").unwrap();
			hex::decode("5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133").unwrap();
		let pair2 = ecdsa2::Pair2::from_seed_slice(&secret_key).unwrap();

		// 签名与验证签名
		let signature = pair2.sign(&msg);
		assert!(ecdsa2::Pair2::verify(&signature, msg, &pair2.public()));

		// 直接从 PublicKey 导出账户地址
		let multi_signer = MultiSigner::from(pair2.public());
		// let who = &multi_signer.into_account();
		// println!("account:{:?}", who);

		// MultiSignature 验签
		let multi_sig = MultiSignature::from(signature);
		let result = multi_sig.verify(msg, &multi_signer.into_account());
		assert_eq!(result, true);
	}

	#[test]
	#[should_panic(expected = "Signature verification has not been called")]
	fn batching_still_finishes_when_not_called_directly() {
		let mut ext = sp_state_machine::BasicExternalities::default();
		ext.register_extension(sp_core::traits::TaskExecutorExt::new(
			sp_core::testing::TaskExecutor::new(),
		));

		ext.execute_with(|| {
			let _batching = SignatureBatching::start();
			sp_io::crypto::sr25519_verify(&Default::default(), &Vec::new(), &Default::default());
		});
	}

	#[test]
	#[should_panic(expected = "Hey, I'm an error")]
	fn batching_does_not_panic_while_thread_is_already_panicking() {
		let mut ext = sp_state_machine::BasicExternalities::default();
		ext.register_extension(sp_core::traits::TaskExecutorExt::new(
			sp_core::testing::TaskExecutor::new(),
		));

		ext.execute_with(|| {
			let _batching = SignatureBatching::start();
			panic!("Hey, I'm an error");
		});
	}
}
