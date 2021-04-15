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

//! The Substrate runtime. This can be compiled with `#[no_std]`, ready for Wasm.

#![cfg_attr(not(feature = "std"), no_std)]
// `construct_runtime!` does a lot of recursion and requires us to increase the limit to 256.
#![recursion_limit = "256"]


use sp_std::prelude::*;
use frame_support::{
	construct_runtime, parameter_types, RuntimeDebug,
	weights::{
		Weight, IdentityFee,
		constants::{BlockExecutionWeight, ExtrinsicBaseWeight, RocksDbWeight, WEIGHT_PER_SECOND},
		DispatchClass,
	},
	traits::{
		Currency, Imbalance, KeyOwnerProofSystem, OnUnbalanced, Randomness, LockIdentifier,
		U128CurrencyToVote,
	},
};
use frame_system::{
	EnsureRoot, EnsureOneOf,
	limits::{BlockWeights, BlockLength}
};
use frame_support::traits::InstanceFilter;
use codec::{Encode, Decode};
use sp_core::{
	crypto::KeyTypeId,
	u32_trait::{_1, _2, _3, _4, _5},
	OpaqueMetadata,
};
pub use node_primitives::{AccountId, Signature};
use node_primitives::{AccountIndex, Balance, BlockNumber, Hash, Index, Moment};
use sp_api::impl_runtime_apis;
use sp_runtime::{
	Permill, Perbill, Perquintill, Percent, ApplyExtrinsicResult, impl_opaque_keys, generic,
	create_runtime_str, ModuleId, FixedPointNumber,
};
use sp_runtime::curve::PiecewiseLinear;
use sp_runtime::transaction_validity::{TransactionValidity, TransactionSource, TransactionPriority};
use sp_runtime::traits::{
	self, BlakeTwo256, Block as BlockT, StaticLookup, SaturatedConversion, ConvertInto, OpaqueKeys,
	NumberFor, AccountIdLookup,
};
use sp_version::RuntimeVersion;
#[cfg(any(feature = "std", test))]
use sp_version::NativeVersion;
use pallet_grandpa::{AuthorityId as GrandpaId, AuthorityList as GrandpaAuthorityList};
use pallet_grandpa::fg_primitives;
use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
use sp_authority_discovery::AuthorityId as AuthorityDiscoveryId;
// use pallet_transaction_payment::{FeeDetails, RuntimeDispatchInfo};
// pub use pallet_transaction_payment::{Multiplier, TargetedFeeAdjustment, CurrencyAdapter};
use sp_inherents::{InherentData, CheckInherentsResult};
use static_assertions::const_assert;
use pallet_contracts::weights::WeightInfo;

#[cfg(any(feature = "std", test))]
pub use sp_runtime::BuildStorage;
#[cfg(any(feature = "std", test))]
pub use pallet_balances::Call as BalancesCall;
#[cfg(any(feature = "std", test))]
pub use frame_system::Call as SystemCall;
#[cfg(any(feature = "std", test))]
pub use pallet_staking::StakerStatus;

#[macro_use]
extern crate std;

// #[allow(rust_2018_idioms, clippy::useless_attribute)]
// extern crate serde as _serde;
// use serde as _serde;

/// Implementations of some helper traits passed into runtime modules as associated types.
// use impls::Author;
// use sp_consensus_aura::sr25519::AuthorityId as AuraId;

/// Constant values used within the runtime.
// pub mod constants;
// pub mod impls;
pub mod node_runtime_simple;

// use constants::{time::*, currency::*};
// use sp_runtime::generic::Era;
//
// // Make the WASM binary available.
// #[cfg(feature = "std")]
// include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));
//
// /// Wasm binary unwrapped. If built with `SKIP_WASM_BUILD`, the function panics.
// #[cfg(feature = "std")]
// pub fn wasm_binary_unwrap() -> &'static [u8] {
// 	WASM_BINARY.expect("Development wasm binary is not available. This means the client is \
// 						built with `SKIP_WASM_BUILD` flag and it is only usable for \
// 						production chains. Please rebuild with the flag disabled.")
// }