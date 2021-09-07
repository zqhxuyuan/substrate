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

// tag::description[]
//! Simple ECDSA API.
// end::description[]

use codec::{Decode, Encode, MaxEncodedLen};
use sp_runtime_interface::pass_by::PassByInner;
use sp_std::cmp::Ordering;

#[cfg(feature = "std")]
use crate::crypto::Ss58Codec;
use crate::crypto::{
	CryptoType, CryptoTypeId, CryptoTypePublicPair, Derive, Public as TraitPublic, UncheckedFrom,
};
#[cfg(feature = "full_crypto")]
use crate::{
	crypto::{DeriveJunction, Pair as TraitPair, SecretStringError},
	hashing::blake2_256,
};
#[cfg(feature = "std")]
use bip39::{Language, Mnemonic, MnemonicType};
#[cfg(feature = "full_crypto")]
use core::convert::{TryFrom, TryInto};
#[cfg(feature = "full_crypto")]
use libsecp256k1::{PublicKey, SecretKey};
#[cfg(feature = "std")]
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
#[cfg(feature = "full_crypto")]
use sp_std::vec::Vec;

/// An identifier used to match public keys against ecdsa keys
pub const CRYPTO_ID2: CryptoTypeId = CryptoTypeId(*b"ecds");

/// A secret seed (which is bytewise essentially equivalent to a SecretKey).
///
/// We need it as a different type because `Seed` is expected to be AsRef<[u8]>.
#[cfg(feature = "full_crypto")]
type Seed2 = [u8; 32];

/// The ECDSA compressed public key.
#[derive(Clone, Encode, Decode, PassByInner, MaxEncodedLen)]
pub struct Public2(pub [u8; 33]);

impl PartialOrd for Public2 {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for Public2 {
	fn cmp(&self, other: &Self) -> Ordering {
		self.as_ref().cmp(&other.as_ref())
	}
}

impl PartialEq for Public2 {
	fn eq(&self, other: &Self) -> bool {
		self.as_ref() == other.as_ref()
	}
}

impl Eq for Public2 {}

// /// An error type for SS58 decoding.
// #[cfg(feature = "std")]
// #[derive(Clone, Copy, Eq, PartialEq, Debug)]
// pub enum PublicError {
// 	/// Bad alphabet.
// 	BadBase58,
// 	/// Bad length.
// 	BadLength,
// 	/// Unknown version.
// 	UnknownVersion,
// 	/// Invalid checksum.
// 	InvalidChecksum,
// }

impl Public2 {
	/// A new instance from the given 33-byte `data`.
	///
	/// NOTE: No checking goes on to ensure this is a real public key. Only use it if
	/// you are certain that the array actually is a pubkey. GIGO!
	pub fn from_raw(data: [u8; 33]) -> Self {
		Self(data)
	}

	/// Create a new instance from the given full public key.
	///
	/// This will convert the full public key into the compressed format.
	#[cfg(feature = "std")]
	pub fn from_full(full: &[u8]) -> Result<Self, ()> {
		libsecp256k1::PublicKey::parse_slice(full, None)
			.map(|k| k.serialize_compressed())
			.map(Self)
			.map_err(|_| ())
	}
}

impl TraitPublic for Public2 {
	/// A new instance from the given slice that should be 33 bytes long.
	///
	/// NOTE: No checking goes on to ensure this is a real public key. Only use it if
	/// you are certain that the array actually is a pubkey. GIGO!
	fn from_slice(data: &[u8]) -> Self {
		let mut r = [0u8; 33];
		r.copy_from_slice(data);
		Self(r)
	}

	fn to_public_crypto_pair(&self) -> CryptoTypePublicPair {
		CryptoTypePublicPair(CRYPTO_ID2, self.to_raw_vec())
	}
}

impl From<Public2> for CryptoTypePublicPair {
	fn from(key: Public2) -> Self {
		(&key).into()
	}
}

impl From<&Public2> for CryptoTypePublicPair {
	fn from(key: &Public2) -> Self {
		CryptoTypePublicPair(CRYPTO_ID2, key.to_raw_vec())
	}
}

impl Derive for Public2 {}

impl Default for Public2 {
	fn default() -> Self {
		Public2([0u8; 33])
	}
}

impl AsRef<[u8]> for Public2 {
	fn as_ref(&self) -> &[u8] {
		&self.0[..]
	}
}

impl AsMut<[u8]> for Public2 {
	fn as_mut(&mut self) -> &mut [u8] {
		&mut self.0[..]
	}
}

impl sp_std::convert::TryFrom<&[u8]> for Public2 {
	type Error = ();

	fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
		if data.len() == 33 {
			Ok(Self::from_slice(data))
		} else {
			Err(())
		}
	}
}

#[cfg(feature = "full_crypto")]
impl From<Pair2> for Public2 {
	fn from(x: Pair2) -> Self {
		x.public()
	}
}

impl UncheckedFrom<[u8; 33]> for Public2 {
	fn unchecked_from(x: [u8; 33]) -> Self {
		Public2(x)
	}
}

#[cfg(feature = "std")]
impl std::fmt::Display for Public2 {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{}", self.to_ss58check())
	}
}

impl sp_std::fmt::Debug for Public2 {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		let s = self.to_ss58check();
		write!(f, "{} ({}...)", crate::hexdisplay::HexDisplay::from(&self.as_ref()), &s[0..8])
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut sp_std::fmt::Formatter) -> sp_std::fmt::Result {
		Ok(())
	}
}

#[cfg(feature = "std")]
impl Serialize for Public2 {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_str(&self.to_ss58check())
	}
}

#[cfg(feature = "std")]
impl<'de> Deserialize<'de> for Public2 {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		Public2::from_ss58check(&String::deserialize(deserializer)?)
			.map_err(|e| de::Error::custom(format!("{:?}", e)))
	}
}

#[cfg(feature = "full_crypto")]
impl sp_std::hash::Hash for Public2 {
	fn hash<H: sp_std::hash::Hasher>(&self, state: &mut H) {
		self.as_ref().hash(state);
	}
}

/// A signature (a 512-bit value, plus 8 bits for recovery ID).
#[derive(Encode, Decode, PassByInner)]
pub struct Signature2(pub [u8; 65]);

impl sp_std::convert::TryFrom<&[u8]> for Signature2 {
	type Error = ();

	fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
		if data.len() == 65 {
			let mut inner = [0u8; 65];
			inner.copy_from_slice(data);
			Ok(Signature2(inner))
		} else {
			Err(())
		}
	}
}

#[cfg(feature = "std")]
impl Serialize for Signature2 {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_str(&hex::encode(self))
	}
}

#[cfg(feature = "std")]
impl<'de> Deserialize<'de> for Signature2 {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let signature_hex = hex::decode(&String::deserialize(deserializer)?)
			.map_err(|e| de::Error::custom(format!("{:?}", e)))?;
		Signature2::try_from(signature_hex.as_ref())
			.map_err(|e| de::Error::custom(format!("{:?}", e)))
	}
}

impl Clone for Signature2 {
	fn clone(&self) -> Self {
		let mut r = [0u8; 65];
		r.copy_from_slice(&self.0[..]);
		Signature2(r)
	}
}

impl Default for Signature2 {
	fn default() -> Self {
		Signature2([0u8; 65])
	}
}

impl PartialEq for Signature2 {
	fn eq(&self, b: &Self) -> bool {
		self.0[..] == b.0[..]
	}
}

impl Eq for Signature2 {}

impl From<Signature2> for [u8; 65] {
	fn from(v: Signature2) -> [u8; 65] {
		v.0
	}
}

impl AsRef<[u8; 65]> for Signature2 {
	fn as_ref(&self) -> &[u8; 65] {
		&self.0
	}
}

impl AsRef<[u8]> for Signature2 {
	fn as_ref(&self) -> &[u8] {
		&self.0[..]
	}
}

impl AsMut<[u8]> for Signature2 {
	fn as_mut(&mut self) -> &mut [u8] {
		&mut self.0[..]
	}
}

impl sp_std::fmt::Debug for Signature2 {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut sp_std::fmt::Formatter) -> sp_std::fmt::Result {
		write!(f, "{}", crate::hexdisplay::HexDisplay::from(&self.0))
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut sp_std::fmt::Formatter) -> sp_std::fmt::Result {
		Ok(())
	}
}

#[cfg(feature = "full_crypto")]
impl sp_std::hash::Hash for Signature2 {
	fn hash<H: sp_std::hash::Hasher>(&self, state: &mut H) {
		sp_std::hash::Hash::hash(&self.0[..], state);
	}
}

impl Signature2 {
	/// A new instance from the given 65-byte `data`.
	///
	/// NOTE: No checking goes on to ensure this is a real signature. Only use it if
	/// you are certain that the array actually is a signature. GIGO!
	pub fn from_raw(data: [u8; 65]) -> Signature2 {
		Signature2(data)
	}

	/// A new instance from the given slice that should be 65 bytes long.
	///
	/// NOTE: No checking goes on to ensure this is a real signature. Only use it if
	/// you are certain that the array actually is a signature. GIGO!
	pub fn from_slice(data: &[u8]) -> Self {
		let mut r = [0u8; 65];
		r.copy_from_slice(data);
		Signature2(r)
	}

	/// Recover the public key from this signature and a message.
	#[cfg(feature = "full_crypto")]
	pub fn recover<M: AsRef<[u8]>>(&self, message: M) -> Option<Public2> {
		let message = libsecp256k1::Message::parse(&blake2_256(message.as_ref()));
		let sig: (_, _) = self.try_into().ok()?;
		libsecp256k1::recover(&message, &sig.0, &sig.1)
			.ok()
			.map(|recovered| Public2(recovered.serialize_compressed()))
	}

	/// Recover the public key from this signature and a pre-hashed message.
	#[cfg(feature = "full_crypto")]
	pub fn recover_prehashed(&self, message: &[u8; 32]) -> Option<Public2> {
		let message = libsecp256k1::Message::parse(message);

		let sig: (_, _) = self.try_into().ok()?;

		libsecp256k1::recover(&message, &sig.0, &sig.1)
			.ok()
			.map(|key| Public2(key.serialize_compressed()))
	}
}

#[cfg(feature = "full_crypto")]
impl From<(libsecp256k1::Signature, libsecp256k1::RecoveryId)> for Signature2 {
	fn from(x: (libsecp256k1::Signature, libsecp256k1::RecoveryId)) -> Signature2 {
		let mut r = Self::default();
		r.0[0..64].copy_from_slice(&x.0.serialize()[..]);
		r.0[64] = x.1.serialize();
		r
	}
}

#[cfg(feature = "full_crypto")]
impl<'a> TryFrom<&'a Signature2> for (libsecp256k1::Signature, libsecp256k1::RecoveryId) {
	type Error = ();
	fn try_from(
		x: &'a Signature2,
	) -> Result<(libsecp256k1::Signature, libsecp256k1::RecoveryId), Self::Error> {
		parse_signature_standard(&x.0).map_err(|_| ())
	}
}

/// Derive a single hard junction.
#[cfg(feature = "full_crypto")]
fn derive_hard_junction(secret_seed: &Seed2, cc: &[u8; 32]) -> Seed2 {
	("Secp256k1HDKD", secret_seed, cc).using_encoded(|data| {
		let mut res = [0u8; 32];
		res.copy_from_slice(blake2_rfc::blake2b::blake2b(32, &[], data).as_bytes());
		res
	})
}

/// An error when deriving a key.
#[cfg(feature = "full_crypto")]
pub enum DeriveError2 {
	/// A soft key was found in the path (and is unsupported).
	SoftKeyInPath,
}

/// A key pair.
#[cfg(feature = "full_crypto")]
#[derive(Clone)]
pub struct Pair2 {
	public: PublicKey,
	secret: SecretKey,
}

#[cfg(feature = "full_crypto")]
impl TraitPair for Pair2 {
	type Public = Public2;
	type Seed = Seed2;
	type Signature = Signature2;
	type DeriveError = DeriveError2;

	/// Generate new secure (random) key pair and provide the recovery phrase.
	///
	/// You can recover the same key later with `from_phrase`.
	#[cfg(feature = "std")]
	fn generate_with_phrase(password: Option<&str>) -> (Pair2, String, Seed2) {
		let mnemonic = Mnemonic::new(MnemonicType::Words12, Language::English);
		let phrase = mnemonic.phrase();
		let (pair, seed) = Self::from_phrase(phrase, password)
			.expect("All phrases generated by Mnemonic are valid; qed");
		(pair, phrase.to_owned(), seed)
	}

	/// Generate key pair from given recovery phrase and password.
	#[cfg(feature = "std")]
	fn from_phrase(
		phrase: &str,
		password: Option<&str>,
	) -> Result<(Pair2, Seed2), SecretStringError> {
		let big_seed = substrate_bip39::seed_from_entropy(
			Mnemonic::from_phrase(phrase, Language::English)
				.map_err(|_| SecretStringError::InvalidPhrase)?
				.entropy(),
			password.unwrap_or(""),
		)
		.map_err(|_| SecretStringError::InvalidSeed)?;
		let mut seed = Seed2::default();
		seed.copy_from_slice(&big_seed[0..32]);
		Self::from_seed_slice(&big_seed[0..32]).map(|x| (x, seed))
	}

	/// Make a new key pair from secret seed material.
	///
	/// You should never need to use this; generate(), generate_with_phrase
	fn from_seed(seed: &Seed2) -> Pair2 {
		Self::from_seed_slice(&seed[..]).expect("seed has valid length; qed")
	}

	/// Make a new key pair from secret seed material. The slice must be 32 bytes long or it
	/// will return `None`.
	///
	/// You should never need to use this; generate(), generate_with_phrase
	fn from_seed_slice(seed_slice: &[u8]) -> Result<Pair2, SecretStringError> {
		let secret =
			SecretKey::parse_slice(seed_slice).map_err(|_| SecretStringError::InvalidSeedLength)?;
		let public = PublicKey::from_secret_key(&secret);
		Ok(Pair2 { public, secret })
	}

	/// Derive a child key from a series of given junctions.
	fn derive<Iter: Iterator<Item = DeriveJunction>>(
		&self,
		path: Iter,
		_seed: Option<Seed2>,
	) -> Result<(Pair2, Option<Seed2>), DeriveError2> {
		let mut acc = self.secret.serialize();
		for j in path {
			match j {
				DeriveJunction::Soft(_cc) => return Err(DeriveError2::SoftKeyInPath),
				DeriveJunction::Hard(cc) => acc = derive_hard_junction(&acc, &cc),
			}
		}
		Ok((Self::from_seed(&acc), Some(acc)))
	}

	/// Get the public key.
	fn public(&self) -> Public2 {
		Public2(self.public.serialize_compressed())
	}

	/// Sign a message.
	fn sign(&self, message: &[u8]) -> Signature2 {
		let message = libsecp256k1::Message::parse(&blake2_256(message));
		libsecp256k1::sign(&message, &self.secret).into()
	}

	/// Verify a signature on a message. Returns true if the signature is good.
	fn verify<M: AsRef<[u8]>>(sig: &Self::Signature, message: M, pubkey: &Self::Public) -> bool {
		let message = libsecp256k1::Message::parse(&blake2_256(message.as_ref()));
		let sig: (_, _) = match sig.try_into() {
			Ok(x) => x,
			_ => return false,
		};
		match libsecp256k1::recover(&message, &sig.0, &sig.1) {
			Ok(actual) => pubkey.0[..] == actual.serialize_compressed()[..],
			_ => false,
		}
	}

	/// Verify a signature on a message. Returns true if the signature is good.
	///
	/// This doesn't use the type system to ensure that `sig` and `pubkey` are the correct
	/// size. Use it only if you're coming from byte buffers and need the speed.
	fn verify_weak<P: AsRef<[u8]>, M: AsRef<[u8]>>(sig: &[u8], message: M, pubkey: P) -> bool {
		let message = libsecp256k1::Message::parse(&blake2_256(message.as_ref()));
		if sig.len() != 65 {
			return false
		}
		let (sig, ri) = match parse_signature_standard(&sig) {
			Ok(sigri) => sigri,
			_ => return false,
		};
		match libsecp256k1::recover(&message, &sig, &ri) {
			Ok(actual) => pubkey.as_ref() == &actual.serialize()[1..],
			_ => false,
		}
	}

	/// Return a vec filled with raw data.
	fn to_raw_vec(&self) -> Vec<u8> {
		self.seed().to_vec()
	}
}

#[cfg(feature = "full_crypto")]
impl Pair2 {
	/// Get the seed for this key.
	pub fn seed(&self) -> Seed2 {
		self.secret.serialize()
	}

	/// Exactly as `from_string` except that if no matches are found then, the the first 32
	/// characters are taken (padded with spaces as necessary) and used as the MiniSecretKey.
	#[cfg(feature = "std")]
	pub fn from_legacy_string(s: &str, password_override: Option<&str>) -> Pair2 {
		Self::from_string(s, password_override).unwrap_or_else(|_| {
			let mut padded_seed: Seed2 = [b' '; 32];
			let len = s.len().min(32);
			padded_seed[..len].copy_from_slice(&s.as_bytes()[..len]);
			Self::from_seed(&padded_seed)
		})
	}

	/// Sign a pre-hashed message
	pub fn sign_prehashed(&self, message: &[u8; 32]) -> Signature2 {
		let message = libsecp256k1::Message::parse(message);
		libsecp256k1::sign(&message, &self.secret).into()
	}

	/// Verify a signature on a pre-hashed message. Return `true` if the signature is valid
	/// and thus matches the given `public` key.
	pub fn verify_prehashed(sig: &Signature2, message: &[u8; 32], public: &Public2) -> bool {
		let message = libsecp256k1::Message::parse(message);

		let sig: (_, _) = match sig.try_into() {
			Ok(x) => x,
			_ => return false,
		};

		match libsecp256k1::recover(&message, &sig.0, &sig.1) {
			Ok(actual) => public.0[..] == actual.serialize_compressed()[..],
			_ => false,
		}
	}

	/// Verify a signature on a message. Returns true if the signature is good.
	/// Parses Signature using parse_overflowing_slice
	pub fn verify_deprecated<M: AsRef<[u8]>>(sig: &Signature2, message: M, pubkey: &Public2) -> bool {
		let message = libsecp256k1::Message::parse(&blake2_256(message.as_ref()));
		let (sig, ri) = match parse_signature_overflowing(&sig.0) {
			Ok(sigri) => sigri,
			_ => return false,
		};
		match libsecp256k1::recover(&message, &sig, &ri) {
			Ok(actual) => pubkey.0[..] == actual.serialize_compressed()[..],
			_ => false,
		}
	}
}

#[cfg(feature = "full_crypto")]
fn parse_signature_standard(
	x: &[u8],
) -> Result<(libsecp256k1::Signature, libsecp256k1::RecoveryId), libsecp256k1::Error> {
	let sig = libsecp256k1::Signature::parse_standard_slice(&x[..64])?;
	let ri = libsecp256k1::RecoveryId::parse(x[64])?;
	Ok((sig, ri))
}

#[cfg(feature = "full_crypto")]
fn parse_signature_overflowing(
	x: &[u8],
) -> Result<(libsecp256k1::Signature, libsecp256k1::RecoveryId), libsecp256k1::Error> {
	let sig = libsecp256k1::Signature::parse_overflowing_slice(&x[..64])?;
	let ri = libsecp256k1::RecoveryId::parse(x[64])?;
	Ok((sig, ri))
}

impl CryptoType for Public2 {
	#[cfg(feature = "full_crypto")]
	type Pair = Pair2;
}

impl CryptoType for Signature2 {
	#[cfg(feature = "full_crypto")]
	type Pair = Pair2;
}

#[cfg(feature = "full_crypto")]
impl CryptoType for Pair2 {
	type Pair = Pair2;
}
