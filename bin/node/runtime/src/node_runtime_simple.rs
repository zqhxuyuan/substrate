#![feature(prelude_import)]
//! The Substrate runtime. This can be compiled with `#[no_std]`, ready for Wasm.
#![recursion_limit = "256"]
#[prelude_import]
use std::prelude::rust_2018::*;
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
    limits::{BlockWeights, BlockLength},
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
use sp_inherents::{InherentData, CheckInherentsResult};
// use static_assertions::const_assert;
use pallet_contracts::weights::WeightInfo;
#[cfg(any(feature = "std", test))]
pub use sp_runtime::BuildStorage;
#[cfg(any(feature = "std", test))]
pub use pallet_balances::Call as BalancesCall;
#[cfg(any(feature = "std", test))]
pub use frame_system::Call as SystemCall;
#[cfg(any(feature = "std", test))]
pub use pallet_staking::StakerStatus;
/// Implementations of some helper traits passed into runtime modules as associated types.
pub mod impls {
    //! Some configurable implementations as associated type for the substrate runtime.
    use frame_support::traits::{OnUnbalanced, Currency};
}
/// Constant values used within the runtime.
pub mod constants {
    //! A set of constant values used in substrate runtime.
    /// Money matters.
    pub mod currency {
        use node_primitives::Balance;
        pub const MILLICENTS: Balance = 1_000_000_000;
        pub const CENTS: Balance = 1_000 * MILLICENTS;
        pub const DOLLARS: Balance = 100 * CENTS;
        pub const fn deposit(items: u32, bytes: u32) -> Balance {
            items as Balance * 15 * CENTS + (bytes as Balance) * 6 * CENTS
        }
    }
    /// Time.
    pub mod time {
        use node_primitives::{Moment, BlockNumber};
        /// Since BABE is probabilistic this is the average expected block time that
        /// we are targeting. Blocks will be produced at a minimum duration defined
        /// by `SLOT_DURATION`, but some slots will not be allocated to any
        /// authority and hence no block will be produced. We expect to have this
        /// block time on average following the defined slot duration and the value
        /// of `c` configured for BABE (where `1 - c` represents the probability of
        /// a slot being empty).
        /// This value is only used indirectly to define the unit constants below
        /// that are expressed in blocks. The rest of the code should use
        /// `SLOT_DURATION` instead (like the Timestamp pallet for calculating the
        /// minimum period).
        ///
        /// If using BABE with secondary slots (default) then all of the slots will
        /// always be assigned, in which case `MILLISECS_PER_BLOCK` and
        /// `SLOT_DURATION` should have the same value.
        ///
        /// <https://research.web3.foundation/en/latest/polkadot/block-production/Babe.html#-6.-practical-results>
        pub const MILLISECS_PER_BLOCK: Moment = 3000;
        pub const SECS_PER_BLOCK: Moment = MILLISECS_PER_BLOCK / 1000;
        pub const SLOT_DURATION: Moment = MILLISECS_PER_BLOCK;
        pub const PRIMARY_PROBABILITY: (u64, u64) = (1, 4);
        pub const EPOCH_DURATION_IN_BLOCKS: BlockNumber = 10 * MINUTES;
        pub const EPOCH_DURATION_IN_SLOTS: u64 = {
            const SLOT_FILL_RATE: f64 = MILLISECS_PER_BLOCK as f64 / SLOT_DURATION as f64;
            (EPOCH_DURATION_IN_BLOCKS as f64 * SLOT_FILL_RATE) as u64
        };
        pub const MINUTES: BlockNumber = 60 / (SECS_PER_BLOCK as BlockNumber);
        pub const HOURS: BlockNumber = MINUTES * 60;
        pub const DAYS: BlockNumber = HOURS * 24;
    }
}

use constants::{time::*, currency::*};
use sp_runtime::generic::Era;

// Make the WASM binary available.
#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

/// Wasm binary unwrapped. If built with `SKIP_WASM_BUILD`, the function panics.
#[cfg(feature = "std")]
pub fn wasm_binary_unwrap() -> &'static [u8] {
    WASM_BINARY.expect("Development wasm binary is not available. This means the client is \
						built with `SKIP_WASM_BUILD` flag and it is only usable for \
						production chains. Please rebuild with the flag disabled.")
}

/// Runtime version.
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_name: { ::sp_runtime::RuntimeString::Borrowed("node") },
    impl_name: { ::sp_runtime::RuntimeString::Borrowed("substrate-node") },
    authoring_version: 10,
    spec_version: 265,
    impl_version: 1,
    apis: RUNTIME_API_VERSIONS,
    transaction_version: 2,
};
/// The BABE epoch configuration at genesis.
pub const BABE_GENESIS_EPOCH_CONFIG: sp_consensus_babe::BabeEpochConfiguration =
    sp_consensus_babe::BabeEpochConfiguration {
        c: PRIMARY_PROBABILITY,
        allowed_slots: sp_consensus_babe::AllowedSlots::PrimaryAndSecondaryPlainSlots,
    };
// pub const SLOT_DURATION: u64 = MILLISECS_PER_BLOCK;
/// Native version.
#[cfg(any(feature = "std", test))]
pub fn native_version() -> NativeVersion {
    NativeVersion {
        runtime_version: VERSION,
        can_author_with: Default::default(),
    }
}
/// We assume that ~10% of the block weight is consumed by `on_initialize` handlers.
/// This is used to limit the maximal weight of a single extrinsic.
const AVERAGE_ON_INITIALIZE_RATIO: Perbill = Perbill::from_percent(10);
/// We allow `Normal` extrinsics to fill up the block up to 75%, the rest can be used
/// by  Operational  extrinsics.
const NORMAL_DISPATCH_RATIO: Perbill = Perbill::from_percent(75);
/// We allow for 2 seconds of compute with a 6 second average block time.
const MAXIMUM_BLOCK_WEIGHT: Weight = 2 * WEIGHT_PER_SECOND;
pub struct BlockHashCount;
impl BlockHashCount {
    /// Returns the value of this parameter type.
    pub const fn get() -> BlockNumber {
        2400
    }
}
impl<I: From<BlockNumber>> ::frame_support::traits::Get<I> for BlockHashCount {
    fn get() -> I {
        I::from(2400)
    }
}
pub struct Version;
impl Version {
    /// Returns the value of this parameter type.
    pub const fn get() -> RuntimeVersion {
        VERSION
    }
}
impl<I: From<RuntimeVersion>> ::frame_support::traits::Get<I> for Version {
    fn get() -> I {
        I::from(VERSION)
    }
}
pub struct RuntimeBlockLength;
impl RuntimeBlockLength {
    /// Returns the value of this parameter type.
    pub fn get() -> BlockLength {
        BlockLength::max_with_normal_ratio(5 * 1024 * 1024, NORMAL_DISPATCH_RATIO)
    }
}
impl<I: From<BlockLength>> ::frame_support::traits::Get<I> for RuntimeBlockLength {
    fn get() -> I {
        I::from(BlockLength::max_with_normal_ratio(
            5 * 1024 * 1024,
            NORMAL_DISPATCH_RATIO,
        ))
    }
}
pub struct RuntimeBlockWeights;
impl RuntimeBlockWeights {
    /// Returns the value of this parameter type.
    pub fn get() -> BlockWeights {
        BlockWeights::builder()
            .base_block(BlockExecutionWeight::get())
            .for_class(DispatchClass::all(), |weights| {
                weights.base_extrinsic = ExtrinsicBaseWeight::get();
            })
            .for_class(DispatchClass::Normal, |weights| {
                weights.max_total = Some(NORMAL_DISPATCH_RATIO * MAXIMUM_BLOCK_WEIGHT);
            })
            .for_class(DispatchClass::Operational, |weights| {
                weights.max_total = Some(MAXIMUM_BLOCK_WEIGHT);
                weights.reserved =
                    Some(MAXIMUM_BLOCK_WEIGHT - NORMAL_DISPATCH_RATIO * MAXIMUM_BLOCK_WEIGHT);
            })
            .avg_block_initialization(AVERAGE_ON_INITIALIZE_RATIO)
            .build_or_panic()
    }
}
impl<I: From<BlockWeights>> ::frame_support::traits::Get<I> for RuntimeBlockWeights {
    fn get() -> I {
        I::from(
            BlockWeights::builder()
                .base_block(BlockExecutionWeight::get())
                .for_class(DispatchClass::all(), |weights| {
                    weights.base_extrinsic = ExtrinsicBaseWeight::get();
                })
                .for_class(DispatchClass::Normal, |weights| {
                    weights.max_total = Some(NORMAL_DISPATCH_RATIO * MAXIMUM_BLOCK_WEIGHT);
                })
                .for_class(DispatchClass::Operational, |weights| {
                    weights.max_total = Some(MAXIMUM_BLOCK_WEIGHT);
                    weights.reserved =
                        Some(MAXIMUM_BLOCK_WEIGHT - NORMAL_DISPATCH_RATIO * MAXIMUM_BLOCK_WEIGHT);
                })
                .avg_block_initialization(AVERAGE_ON_INITIALIZE_RATIO)
                .build_or_panic(),
        )
    }
}
pub struct SS58Prefix;
impl SS58Prefix {
    /// Returns the value of this parameter type.
    pub const fn get() -> u8 {
        42
    }
}
impl<I: From<u8>> ::frame_support::traits::Get<I> for SS58Prefix {
    fn get() -> I {
        I::from(42)
    }
}
#[allow(unknown_lints, eq_op)]
const _: [(); 0 - !{
    const ASSERT: bool =
        NORMAL_DISPATCH_RATIO.deconstruct() >= AVERAGE_ON_INITIALIZE_RATIO.deconstruct();
    ASSERT
} as usize] = [];
impl frame_system::Config for Runtime {
    type BaseCallFilter = ();
    type BlockWeights = RuntimeBlockWeights;
    type BlockLength = RuntimeBlockLength;
    type DbWeight = RocksDbWeight;
    type Origin = Origin;
    type Call = Call;
    type Index = Index;
    type BlockNumber = BlockNumber;
    type Hash = Hash;
    type Hashing = BlakeTwo256;
    type AccountId = AccountId;
    type Lookup = AccountIdLookup<AccountId, ()>;
    type Header = generic::Header<BlockNumber, BlakeTwo256>;
    type Event = Event;
    type BlockHashCount = BlockHashCount;
    type Version = Version;
    type PalletInfo = PalletInfo;
    type AccountData = pallet_balances::AccountData<Balance>;
    type OnNewAccount = ();
    type OnKilledAccount = ();
    type SystemWeightInfo = frame_system::weights::SubstrateWeight<Runtime>;
    type SS58Prefix = SS58Prefix;
    type OnSetCode = ();
}
impl pallet_utility::Config for Runtime {
    type Event = Event;
    type Call = Call;
    type WeightInfo = pallet_utility::weights::SubstrateWeight<Runtime>;
}
pub struct BondingDuration;
impl BondingDuration {
    /// Returns the value of this parameter type.
    pub const fn get() -> pallet_staking::EraIndex {
        24 * 28
    }
}
impl<I: From<pallet_staking::EraIndex>> ::frame_support::traits::Get<I> for BondingDuration {
    fn get() -> I {
        I::from(24 * 28)
    }
}
pub struct SessionsPerEra;
impl SessionsPerEra {
    /// Returns the value of this parameter type.
    pub const fn get() -> sp_staking::SessionIndex {
        6
    }
}
impl<I: From<sp_staking::SessionIndex>> ::frame_support::traits::Get<I> for SessionsPerEra {
    fn get() -> I {
        I::from(6)
    }
}
pub struct EpochDuration;
impl EpochDuration {
    /// Returns the value of this parameter type.
    pub const fn get() -> u64 {
        EPOCH_DURATION_IN_SLOTS
    }
}
impl<I: From<u64>> ::frame_support::traits::Get<I> for EpochDuration {
    fn get() -> I {
        I::from(EPOCH_DURATION_IN_SLOTS)
    }
}
pub struct ExpectedBlockTime;
impl ExpectedBlockTime {
    /// Returns the value of this parameter type.
    pub const fn get() -> Moment {
        MILLISECS_PER_BLOCK
    }
}
impl<I: From<Moment>> ::frame_support::traits::Get<I> for ExpectedBlockTime {
    fn get() -> I {
        I::from(MILLISECS_PER_BLOCK)
    }
}
pub struct ReportLongevity;
impl ReportLongevity {
    /// Returns the value of this parameter type.
    pub const fn get() -> u64 {
        BondingDuration::get() as u64 * SessionsPerEra::get() as u64 * EpochDuration::get()
    }
}
impl<I: From<u64>> ::frame_support::traits::Get<I> for ReportLongevity {
    fn get() -> I {
        I::from(BondingDuration::get() as u64 * SessionsPerEra::get() as u64 * EpochDuration::get())
    }
}
impl pallet_babe::Config for Runtime {
    type EpochDuration = EpochDuration;
    type ExpectedBlockTime = ExpectedBlockTime;
    type EpochChangeTrigger = pallet_babe::ExternalTrigger;
    type KeyOwnerProofSystem = ();
    type KeyOwnerProof = <Self::KeyOwnerProofSystem as KeyOwnerProofSystem<(
        KeyTypeId,
        pallet_babe::AuthorityId,
    )>>::Proof;
    type KeyOwnerIdentification = <Self::KeyOwnerProofSystem as KeyOwnerProofSystem<(
        KeyTypeId,
        pallet_babe::AuthorityId,
    )>>::IdentificationTuple;
    type HandleEquivocation = ();
    type WeightInfo = ();
}
pub struct ExistentialDeposit;
impl ExistentialDeposit {
    /// Returns the value of this parameter type.
    pub const fn get() -> Balance {
        1 * DOLLARS
    }
}
impl<I: From<Balance>> ::frame_support::traits::Get<I> for ExistentialDeposit {
    fn get() -> I {
        I::from(1 * DOLLARS)
    }
}
pub struct MaxLocks;
impl MaxLocks {
    /// Returns the value of this parameter type.
    pub const fn get() -> u32 {
        50
    }
}
impl<I: From<u32>> ::frame_support::traits::Get<I> for MaxLocks {
    fn get() -> I {
        I::from(50)
    }
}
impl pallet_balances::Config for Runtime {
    type MaxLocks = MaxLocks;
    type Balance = Balance;
    type DustRemoval = ();
    type Event = Event;
    type ExistentialDeposit = ExistentialDeposit;
    type AccountStore = frame_system::Pallet<Runtime>;
    type WeightInfo = pallet_balances::weights::SubstrateWeight<Runtime>;
}
pub struct MinimumPeriod;
impl MinimumPeriod {
    /// Returns the value of this parameter type.
    pub const fn get() -> Moment {
        SLOT_DURATION / 2
    }
}
impl<I: From<Moment>> ::frame_support::traits::Get<I> for MinimumPeriod {
    fn get() -> I {
        I::from(SLOT_DURATION / 2)
    }
}
impl pallet_timestamp::Config for Runtime {
    type Moment = Moment;
    type OnTimestampSet = Babe;
    type MinimumPeriod = MinimumPeriod;
    type WeightInfo = pallet_timestamp::weights::SubstrateWeight<Runtime>;
}
pub struct SessionKeys {}

#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::default::Default for SessionKeys {
    #[inline]
    fn default() -> SessionKeys {
        SessionKeys {}
    }
}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::clone::Clone for SessionKeys {
    #[inline]
    fn clone(&self) -> SessionKeys {
        match *self {
            SessionKeys {} => SessionKeys {},
        }
    }
}
impl ::core::marker::StructuralPartialEq for SessionKeys {}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::cmp::PartialEq for SessionKeys {
    #[inline]
    fn eq(&self, other: &SessionKeys) -> bool {
        match *other {
            SessionKeys {} => match *self {
                SessionKeys {} => true,
            },
        }
    }
}
impl ::core::marker::StructuralEq for SessionKeys {}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::cmp::Eq for SessionKeys {
    #[inline]
    #[doc(hidden)]
    fn assert_receiver_is_total_eq(&self) -> () {
        {}
    }
}
const _: () = {
    #[allow(unknown_lints)]
    #[allow(rust_2018_idioms)]
    extern crate codec as _parity_scale_codec;
    impl _parity_scale_codec::Encode for SessionKeys {
        fn encode_to<__CodecOutputEdqy: _parity_scale_codec::Output + ?Sized>(
            &self,
            __codec_dest_edqy: &mut __CodecOutputEdqy,
        ) {
        }
    }
    impl _parity_scale_codec::EncodeLike for SessionKeys {}
};
const _: () = {
    #[allow(unknown_lints)]
    #[allow(rust_2018_idioms)]
    extern crate codec as _parity_scale_codec;
    impl _parity_scale_codec::Decode for SessionKeys {
        fn decode<__CodecInputEdqy: _parity_scale_codec::Input>(
            __codec_input_edqy: &mut __CodecInputEdqy,
        ) -> core::result::Result<Self, _parity_scale_codec::Error> {
            Ok(SessionKeys {})
        }
    }
};
impl core::fmt::Debug for SessionKeys {
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
        fmt.debug_tuple("SessionKeys").finish()
    }
}
impl SessionKeys {
    /// Generate a set of keys with optionally using the given seed.
    ///
    /// The generated key pairs are stored in the keystore.
    ///
    /// Returns the concatenated SCALE encoded public keys.
    pub fn generate(
        seed: Option<::sp_runtime::sp_std::vec::Vec<u8>>,
    ) -> ::sp_runtime::sp_std::vec::Vec<u8> {
        let keys = Self {};
        ::sp_runtime::codec::Encode::encode(&keys)
    }
    /// Converts `Self` into a `Vec` of `(raw public key, KeyTypeId)`.
    pub fn into_raw_public_keys(
        self,
    ) -> ::sp_runtime::sp_std::vec::Vec<(::sp_runtime::sp_std::vec::Vec<u8>, ::sp_runtime::KeyTypeId)>
    {
        let mut keys = Vec::new();
        keys
    }
    /// Decode `Self` from the given `encoded` slice and convert `Self` into the raw public
    /// keys (see [`Self::into_raw_public_keys`]).
    ///
    /// Returns `None` when the decoding failed, otherwise `Some(_)`.
    pub fn decode_into_raw_public_keys(
        encoded: &[u8],
    ) -> Option<
        ::sp_runtime::sp_std::vec::Vec<(
            ::sp_runtime::sp_std::vec::Vec<u8>,
            ::sp_runtime::KeyTypeId,
        )>,
    > {
        <Self as ::sp_runtime::codec::Decode>::decode(&mut &encoded[..])
            .ok()
            .map(|s| s.into_raw_public_keys())
    }
}
impl ::sp_runtime::traits::OpaqueKeys for SessionKeys {
    type KeyTypeIdProviders = ();
    fn key_ids() -> &'static [::sp_runtime::KeyTypeId] {
        &[]
    }
    fn get_raw(&self, i: ::sp_runtime::KeyTypeId) -> &[u8] {
        match i {
            _ => &[],
        }
    }
}
const REWARD_CURVE: PiecewiseLinear<'static> = {
    extern crate sp_runtime as _sp_runtime;
    _sp_runtime::curve::PiecewiseLinear::<'static> {
        points: &[
            (
                _sp_runtime::Perbill::from_parts(0u32),
                _sp_runtime::Perbill::from_parts(25000000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(500000000u32),
                _sp_runtime::Perbill::from_parts(100000000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(514743000u32),
                _sp_runtime::Perbill::from_parts(86136000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(529486000u32),
                _sp_runtime::Perbill::from_parts(74835000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(544229000u32),
                _sp_runtime::Perbill::from_parts(65623000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(558972000u32),
                _sp_runtime::Perbill::from_parts(58114000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(573715000u32),
                _sp_runtime::Perbill::from_parts(51993000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(588456000u32),
                _sp_runtime::Perbill::from_parts(47004000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(603197000u32),
                _sp_runtime::Perbill::from_parts(42937000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(617937000u32),
                _sp_runtime::Perbill::from_parts(39622000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(632675000u32),
                _sp_runtime::Perbill::from_parts(36920000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(647415000u32),
                _sp_runtime::Perbill::from_parts(34717000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(662156000u32),
                _sp_runtime::Perbill::from_parts(32921000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(676897000u32),
                _sp_runtime::Perbill::from_parts(31457000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(691632000u32),
                _sp_runtime::Perbill::from_parts(30264000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(706375000u32),
                _sp_runtime::Perbill::from_parts(29291000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(721114000u32),
                _sp_runtime::Perbill::from_parts(28498000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(735842000u32),
                _sp_runtime::Perbill::from_parts(27852000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(750579000u32),
                _sp_runtime::Perbill::from_parts(27325000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(765292000u32),
                _sp_runtime::Perbill::from_parts(26896000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(780013000u32),
                _sp_runtime::Perbill::from_parts(26546000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(794712000u32),
                _sp_runtime::Perbill::from_parts(26261000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(809448000u32),
                _sp_runtime::Perbill::from_parts(26028000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(824189000u32),
                _sp_runtime::Perbill::from_parts(25838000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(838837000u32),
                _sp_runtime::Perbill::from_parts(25684000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(853524000u32),
                _sp_runtime::Perbill::from_parts(25558000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(868243000u32),
                _sp_runtime::Perbill::from_parts(25455000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(882966000u32),
                _sp_runtime::Perbill::from_parts(25371000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(897571000u32),
                _sp_runtime::Perbill::from_parts(25303000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(912311000u32),
                _sp_runtime::Perbill::from_parts(25247000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(926819000u32),
                _sp_runtime::Perbill::from_parts(25202000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(941413000u32),
                _sp_runtime::Perbill::from_parts(25165000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(955889000u32),
                _sp_runtime::Perbill::from_parts(25135000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(970009000u32),
                _sp_runtime::Perbill::from_parts(25111000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(984340000u32),
                _sp_runtime::Perbill::from_parts(25091000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(998289000u32),
                _sp_runtime::Perbill::from_parts(25075000u32),
            ),
            (
                _sp_runtime::Perbill::from_parts(1000000000u32),
                _sp_runtime::Perbill::from_parts(25073000u32),
            ),
        ],
        maximum: _sp_runtime::Perbill::from_parts(100000000u32),
    }
};
impl pallet_sudo::Config for Runtime {
    type Event = Event;
    type Call = Call;
}
impl pallet_grandpa::Config for Runtime {
    type Event = Event;
    type Call = Call;
    type KeyOwnerProofSystem = ();
    type KeyOwnerProof =
        <Self::KeyOwnerProofSystem as KeyOwnerProofSystem<(KeyTypeId, GrandpaId)>>::Proof;
    type KeyOwnerIdentification = <Self::KeyOwnerProofSystem as KeyOwnerProofSystem<(
        KeyTypeId,
        GrandpaId,
    )>>::IdentificationTuple;
    type HandleEquivocation = ();
    type WeightInfo = ();
}
#[doc(hidden)]
mod sp_api_hidden_includes_construct_runtime {
    pub extern crate frame_support as hidden_include;
}
const _: () = {
    #[allow(unused)]
    type __hidden_use_of_unchecked_extrinsic = UncheckedExtrinsic;
};
pub struct Runtime;
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::clone::Clone for Runtime {
    #[inline]
    fn clone(&self) -> Runtime {
        {
            *self
        }
    }
}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::marker::Copy for Runtime {}
impl ::core::marker::StructuralPartialEq for Runtime {}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::cmp::PartialEq for Runtime {
    #[inline]
    fn eq(&self, other: &Runtime) -> bool {
        match *other {
            Runtime => match *self {
                Runtime => true,
            },
        }
    }
}
impl ::core::marker::StructuralEq for Runtime {}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::cmp::Eq for Runtime {
    #[inline]
    #[doc(hidden)]
    fn assert_receiver_is_total_eq(&self) -> () {
        {}
    }
}
impl core::fmt::Debug for Runtime {
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
        fmt.debug_tuple("Runtime").finish()
    }
}
impl self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_runtime :: traits :: GetNodeBlockType for Runtime { type NodeBlock = node_primitives :: Block ; }
impl self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_runtime :: traits :: GetRuntimeBlockType for Runtime { type RuntimeBlock = Block ; }
#[allow(non_camel_case_types)]
pub enum Event {
    #[codec(index = 0u8)]
    frame_system(frame_system::Event<Runtime>),
    #[codec(index = 1u8)]
    pallet_utility(pallet_utility::Event),
    #[codec(index = 4u8)]
    pallet_balances(pallet_balances::Event<Runtime>),
    #[codec(index = 5u8)]
    pallet_grandpa(pallet_grandpa::Event),
    #[codec(index = 6u8)]
    pallet_sudo(pallet_sudo::Event<Runtime>),
}
#[automatically_derived]
#[allow(unused_qualifications)]
#[allow(non_camel_case_types)]
impl ::core::clone::Clone for Event {
    #[inline]
    fn clone(&self) -> Event {
        match (&*self,) {
            (&Event::frame_system(ref __self_0),) => {
                Event::frame_system(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&Event::pallet_utility(ref __self_0),) => {
                Event::pallet_utility(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&Event::pallet_balances(ref __self_0),) => {
                Event::pallet_balances(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&Event::pallet_grandpa(ref __self_0),) => {
                Event::pallet_grandpa(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&Event::pallet_sudo(ref __self_0),) => {
                Event::pallet_sudo(::core::clone::Clone::clone(&(*__self_0)))
            }
        }
    }
}
#[allow(non_camel_case_types)]
impl ::core::marker::StructuralPartialEq for Event {}
#[automatically_derived]
#[allow(unused_qualifications)]
#[allow(non_camel_case_types)]
impl ::core::cmp::PartialEq for Event {
    #[inline]
    fn eq(&self, other: &Event) -> bool {
        {
            let __self_vi = ::core::intrinsics::discriminant_value(&*self);
            let __arg_1_vi = ::core::intrinsics::discriminant_value(&*other);
            if true && __self_vi == __arg_1_vi {
                match (&*self, &*other) {
                    (&Event::frame_system(ref __self_0), &Event::frame_system(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    (
                        &Event::pallet_utility(ref __self_0),
                        &Event::pallet_utility(ref __arg_1_0),
                    ) => (*__self_0) == (*__arg_1_0),
                    (
                        &Event::pallet_balances(ref __self_0),
                        &Event::pallet_balances(ref __arg_1_0),
                    ) => (*__self_0) == (*__arg_1_0),
                    (
                        &Event::pallet_grandpa(ref __self_0),
                        &Event::pallet_grandpa(ref __arg_1_0),
                    ) => (*__self_0) == (*__arg_1_0),
                    (&Event::pallet_sudo(ref __self_0), &Event::pallet_sudo(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    _ => unsafe { ::core::intrinsics::unreachable() },
                }
            } else {
                false
            }
        }
    }
    #[inline]
    fn ne(&self, other: &Event) -> bool {
        {
            let __self_vi = ::core::intrinsics::discriminant_value(&*self);
            let __arg_1_vi = ::core::intrinsics::discriminant_value(&*other);
            if true && __self_vi == __arg_1_vi {
                match (&*self, &*other) {
                    (&Event::frame_system(ref __self_0), &Event::frame_system(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    (
                        &Event::pallet_utility(ref __self_0),
                        &Event::pallet_utility(ref __arg_1_0),
                    ) => (*__self_0) != (*__arg_1_0),
                    (
                        &Event::pallet_balances(ref __self_0),
                        &Event::pallet_balances(ref __arg_1_0),
                    ) => (*__self_0) != (*__arg_1_0),
                    (
                        &Event::pallet_grandpa(ref __self_0),
                        &Event::pallet_grandpa(ref __arg_1_0),
                    ) => (*__self_0) != (*__arg_1_0),
                    (&Event::pallet_sudo(ref __self_0), &Event::pallet_sudo(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    _ => unsafe { ::core::intrinsics::unreachable() },
                }
            } else {
                true
            }
        }
    }
}
#[allow(non_camel_case_types)]
impl ::core::marker::StructuralEq for Event {}
#[automatically_derived]
#[allow(unused_qualifications)]
#[allow(non_camel_case_types)]
impl ::core::cmp::Eq for Event {
    #[inline]
    #[doc(hidden)]
    fn assert_receiver_is_total_eq(&self) -> () {
        {
            let _: ::core::cmp::AssertParamIsEq<frame_system::Event<Runtime>>;
            let _: ::core::cmp::AssertParamIsEq<pallet_utility::Event>;
            let _: ::core::cmp::AssertParamIsEq<pallet_balances::Event<Runtime>>;
            let _: ::core::cmp::AssertParamIsEq<pallet_grandpa::Event>;
            let _: ::core::cmp::AssertParamIsEq<pallet_sudo::Event<Runtime>>;
        }
    }
}
const _: () = {
    #[allow(unknown_lints)]
    #[allow(rust_2018_idioms)]
    extern crate codec as _parity_scale_codec;
    impl _parity_scale_codec::Encode for Event {
        fn encode_to<__CodecOutputEdqy: _parity_scale_codec::Output + ?Sized>(
            &self,
            __codec_dest_edqy: &mut __CodecOutputEdqy,
        ) {
            match *self {
                Event::frame_system(ref aa) => {
                    __codec_dest_edqy.push_byte(0u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Event::pallet_utility(ref aa) => {
                    __codec_dest_edqy.push_byte(1u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Event::pallet_balances(ref aa) => {
                    __codec_dest_edqy.push_byte(4u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Event::pallet_grandpa(ref aa) => {
                    __codec_dest_edqy.push_byte(5u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Event::pallet_sudo(ref aa) => {
                    __codec_dest_edqy.push_byte(6u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                _ => (),
            }
        }
    }
    impl _parity_scale_codec::EncodeLike for Event {}
};
const _: () = {
    #[allow(unknown_lints)]
    #[allow(rust_2018_idioms)]
    extern crate codec as _parity_scale_codec;
    impl _parity_scale_codec::Decode for Event {
        fn decode<__CodecInputEdqy: _parity_scale_codec::Input>(
            __codec_input_edqy: &mut __CodecInputEdqy,
        ) -> core::result::Result<Self, _parity_scale_codec::Error> {
            match __codec_input_edqy
                .read_byte()
                .map_err(|e| e.chain("Could not decode `Event`, failed to read variant byte"))?
            {
                __codec_x_edqy if __codec_x_edqy == 0u8 as u8 => Ok(Event::frame_system({
                    let __codec_res_edqy =
                        <frame_system::Event<Runtime> as _parity_scale_codec::Decode>::decode(
                            __codec_input_edqy,
                        );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Event::frame_system.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 1u8 as u8 => Ok(Event::pallet_utility({
                    let __codec_res_edqy =
                        <pallet_utility::Event as _parity_scale_codec::Decode>::decode(
                            __codec_input_edqy,
                        );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Event::pallet_utility.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 4u8 as u8 => {
                    Ok(Event::pallet_balances({
                        let __codec_res_edqy = < pallet_balances :: Event < Runtime > as _parity_scale_codec :: Decode > :: decode (__codec_input_edqy) ;
                        match __codec_res_edqy {
                            Err(e) => {
                                return Err(e.chain("Could not decode `Event::pallet_balances.0`"))
                            }
                            Ok(__codec_res_edqy) => __codec_res_edqy,
                        }
                    }))
                }
                __codec_x_edqy if __codec_x_edqy == 5u8 as u8 => Ok(Event::pallet_grandpa({
                    let __codec_res_edqy =
                        <pallet_grandpa::Event as _parity_scale_codec::Decode>::decode(
                            __codec_input_edqy,
                        );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Event::pallet_grandpa.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 6u8 as u8 => Ok(Event::pallet_sudo({
                    let __codec_res_edqy =
                        <pallet_sudo::Event<Runtime> as _parity_scale_codec::Decode>::decode(
                            __codec_input_edqy,
                        );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Event::pallet_sudo.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                _ => Err("Could not decode `Event`, variant doesn\'t exist".into()),
            }
        }
    }
};
impl core::fmt::Debug for Event {
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::frame_system(ref a0) => fmt.debug_tuple("Event::frame_system").field(a0).finish(),
            Self::pallet_utility(ref a0) => {
                fmt.debug_tuple("Event::pallet_utility").field(a0).finish()
            }
            Self::pallet_balances(ref a0) => {
                fmt.debug_tuple("Event::pallet_balances").field(a0).finish()
            }
            Self::pallet_grandpa(ref a0) => {
                fmt.debug_tuple("Event::pallet_grandpa").field(a0).finish()
            }
            Self::pallet_sudo(ref a0) => fmt.debug_tuple("Event::pallet_sudo").field(a0).finish(),
            _ => Ok(()),
        }
    }
}
impl From<frame_system::Event<Runtime>> for Event {
    fn from(x: frame_system::Event<Runtime>) -> Self {
        Event::frame_system(x)
    }
}
impl ::frame_support::sp_std::convert::TryInto<frame_system::Event<Runtime>> for Event {
    type Error = ();
    fn try_into(
        self,
    ) -> ::frame_support::sp_std::result::Result<frame_system::Event<Runtime>, Self::Error> {
        match self {
            Self::frame_system(evt) => Ok(evt),
            _ => Err(()),
        }
    }
}
impl From<pallet_utility::Event> for Event {
    fn from(x: pallet_utility::Event) -> Self {
        Event::pallet_utility(x)
    }
}
impl ::frame_support::sp_std::convert::TryInto<pallet_utility::Event> for Event {
    type Error = ();
    fn try_into(
        self,
    ) -> ::frame_support::sp_std::result::Result<pallet_utility::Event, Self::Error> {
        match self {
            Self::pallet_utility(evt) => Ok(evt),
            _ => Err(()),
        }
    }
}
impl From<pallet_balances::Event<Runtime>> for Event {
    fn from(x: pallet_balances::Event<Runtime>) -> Self {
        Event::pallet_balances(x)
    }
}
impl ::frame_support::sp_std::convert::TryInto<pallet_balances::Event<Runtime>> for Event {
    type Error = ();
    fn try_into(
        self,
    ) -> ::frame_support::sp_std::result::Result<pallet_balances::Event<Runtime>, Self::Error> {
        match self {
            Self::pallet_balances(evt) => Ok(evt),
            _ => Err(()),
        }
    }
}
impl From<pallet_grandpa::Event> for Event {
    fn from(x: pallet_grandpa::Event) -> Self {
        Event::pallet_grandpa(x)
    }
}
impl ::frame_support::sp_std::convert::TryInto<pallet_grandpa::Event> for Event {
    type Error = ();
    fn try_into(
        self,
    ) -> ::frame_support::sp_std::result::Result<pallet_grandpa::Event, Self::Error> {
        match self {
            Self::pallet_grandpa(evt) => Ok(evt),
            _ => Err(()),
        }
    }
}
impl From<pallet_sudo::Event<Runtime>> for Event {
    fn from(x: pallet_sudo::Event<Runtime>) -> Self {
        Event::pallet_sudo(x)
    }
}
impl ::frame_support::sp_std::convert::TryInto<pallet_sudo::Event<Runtime>> for Event {
    type Error = ();
    fn try_into(
        self,
    ) -> ::frame_support::sp_std::result::Result<pallet_sudo::Event<Runtime>, Self::Error> {
        match self {
            Self::pallet_sudo(evt) => Ok(evt),
            _ => Err(()),
        }
    }
}
impl Runtime {
    #[allow(dead_code)]
    pub fn outer_event_metadata() -> ::frame_support::event::OuterEventMetadata {
        ::frame_support::event::OuterEventMetadata {
            name: ::frame_support::event::DecodeDifferent::Encode("Event"),
            events: ::frame_support::event::DecodeDifferent::Encode(&[
                (
                    "frame_system",
                    ::frame_support::event::FnEncode(frame_system::Event::<Runtime>::metadata),
                ),
                (
                    "pallet_utility",
                    ::frame_support::event::FnEncode(pallet_utility::Event::metadata),
                ),
                (
                    "pallet_balances",
                    ::frame_support::event::FnEncode(pallet_balances::Event::<Runtime>::metadata),
                ),
                (
                    "pallet_grandpa",
                    ::frame_support::event::FnEncode(pallet_grandpa::Event::metadata),
                ),
                (
                    "pallet_sudo",
                    ::frame_support::event::FnEncode(pallet_sudo::Event::<Runtime>::metadata),
                ),
            ]),
        }
    }
    #[allow(dead_code)]
    pub fn __module_events_frame_system() -> &'static [::frame_support::event::EventMetadata] {
        frame_system::Event::<Runtime>::metadata()
    }
    #[allow(dead_code)]
    pub fn __module_events_pallet_utility() -> &'static [::frame_support::event::EventMetadata] {
        pallet_utility::Event::metadata()
    }
    #[allow(dead_code)]
    pub fn __module_events_pallet_balances() -> &'static [::frame_support::event::EventMetadata] {
        pallet_balances::Event::<Runtime>::metadata()
    }
    #[allow(dead_code)]
    pub fn __module_events_pallet_grandpa() -> &'static [::frame_support::event::EventMetadata] {
        pallet_grandpa::Event::metadata()
    }
    #[allow(dead_code)]
    pub fn __module_events_pallet_sudo() -> &'static [::frame_support::event::EventMetadata] {
        pallet_sudo::Event::<Runtime>::metadata()
    }
}
pub struct Origin {
    caller: OriginCaller,
    filter: ::frame_support::sp_std::rc::Rc<
        Box<dyn Fn(&<Runtime as frame_system::Config>::Call) -> bool>,
    >,
}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::clone::Clone for Origin {
    #[inline]
    fn clone(&self) -> Origin {
        match *self {
            Origin {
                caller: ref __self_0_0,
                filter: ref __self_0_1,
            } => Origin {
                caller: ::core::clone::Clone::clone(&(*__self_0_0)),
                filter: ::core::clone::Clone::clone(&(*__self_0_1)),
            },
        }
    }
}
#[cfg(feature = "std")]
impl ::frame_support::sp_std::fmt::Debug for Origin {
    fn fmt(
        &self,
        fmt: &mut ::frame_support::sp_std::fmt::Formatter,
    ) -> ::frame_support::sp_std::result::Result<(), ::frame_support::sp_std::fmt::Error> {
        fmt.debug_struct("Origin")
            .field("caller", &self.caller)
            .field("filter", &"[function ptr]")
            .finish()
    }
}
impl ::frame_support::traits::OriginTrait for Origin {
    type Call = <Runtime as frame_system::Config>::Call;
    type PalletsOrigin = OriginCaller;
    type AccountId = <Runtime as frame_system::Config>::AccountId;
    fn add_filter(&mut self, filter: impl Fn(&Self::Call) -> bool + 'static) {
        let f = self.filter.clone();
        self.filter =
            ::frame_support::sp_std::rc::Rc::new(Box::new(move |call| f(call) && filter(call)));
    }
    fn reset_filter(&mut self) {
        let filter = < < Runtime as frame_system :: Config > :: BaseCallFilter as :: frame_support :: traits :: Filter < < Runtime as frame_system :: Config > :: Call > > :: filter ;
        self.filter = ::frame_support::sp_std::rc::Rc::new(Box::new(filter));
    }
    fn set_caller_from(&mut self, other: impl Into<Self>) {
        self.caller = other.into().caller
    }
    fn filter_call(&self, call: &Self::Call) -> bool {
        (self.filter)(call)
    }
    fn caller(&self) -> &Self::PalletsOrigin {
        &self.caller
    }
    /// Create with system none origin and `frame-system::Config::BaseCallFilter`.
    fn none() -> Self {
        frame_system::RawOrigin::None.into()
    }
    /// Create with system root origin and no filter.
    fn root() -> Self {
        frame_system::RawOrigin::Root.into()
    }
    /// Create with system signed origin and `frame-system::Config::BaseCallFilter`.
    fn signed(by: <Runtime as frame_system::Config>::AccountId) -> Self {
        frame_system::RawOrigin::Signed(by).into()
    }
}
#[allow(non_camel_case_types)]
pub enum OriginCaller {
    #[codec(index = 0u8)]
    system(frame_system::Origin<Runtime>),
    #[allow(dead_code)]
    Void(::frame_support::Void),
}
#[automatically_derived]
#[allow(unused_qualifications)]
#[allow(non_camel_case_types)]
impl ::core::clone::Clone for OriginCaller {
    #[inline]
    fn clone(&self) -> OriginCaller {
        match (&*self,) {
            (&OriginCaller::system(ref __self_0),) => {
                OriginCaller::system(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&OriginCaller::Void(ref __self_0),) => {
                OriginCaller::Void(::core::clone::Clone::clone(&(*__self_0)))
            }
        }
    }
}
#[allow(non_camel_case_types)]
impl ::core::marker::StructuralPartialEq for OriginCaller {}
#[automatically_derived]
#[allow(unused_qualifications)]
#[allow(non_camel_case_types)]
impl ::core::cmp::PartialEq for OriginCaller {
    #[inline]
    fn eq(&self, other: &OriginCaller) -> bool {
        {
            let __self_vi = ::core::intrinsics::discriminant_value(&*self);
            let __arg_1_vi = ::core::intrinsics::discriminant_value(&*other);
            if true && __self_vi == __arg_1_vi {
                match (&*self, &*other) {
                    (&OriginCaller::system(ref __self_0), &OriginCaller::system(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    (&OriginCaller::Void(ref __self_0), &OriginCaller::Void(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    _ => unsafe { ::core::intrinsics::unreachable() },
                }
            } else {
                false
            }
        }
    }
    #[inline]
    fn ne(&self, other: &OriginCaller) -> bool {
        {
            let __self_vi = ::core::intrinsics::discriminant_value(&*self);
            let __arg_1_vi = ::core::intrinsics::discriminant_value(&*other);
            if true && __self_vi == __arg_1_vi {
                match (&*self, &*other) {
                    (&OriginCaller::system(ref __self_0), &OriginCaller::system(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    (&OriginCaller::Void(ref __self_0), &OriginCaller::Void(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    _ => unsafe { ::core::intrinsics::unreachable() },
                }
            } else {
                true
            }
        }
    }
}
#[allow(non_camel_case_types)]
impl ::core::marker::StructuralEq for OriginCaller {}
#[automatically_derived]
#[allow(unused_qualifications)]
#[allow(non_camel_case_types)]
impl ::core::cmp::Eq for OriginCaller {
    #[inline]
    #[doc(hidden)]
    fn assert_receiver_is_total_eq(&self) -> () {
        {
            let _: ::core::cmp::AssertParamIsEq<frame_system::Origin<Runtime>>;
            let _: ::core::cmp::AssertParamIsEq<::frame_support::Void>;
        }
    }
}
impl core::fmt::Debug for OriginCaller {
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::system(ref a0) => fmt.debug_tuple("OriginCaller::system").field(a0).finish(),
            Self::Void(ref a0) => fmt.debug_tuple("OriginCaller::Void").field(a0).finish(),
            _ => Ok(()),
        }
    }
}
const _: () = {
    #[allow(unknown_lints)]
    #[allow(rust_2018_idioms)]
    extern crate codec as _parity_scale_codec;
    impl _parity_scale_codec::Encode for OriginCaller {
        fn encode_to<__CodecOutputEdqy: _parity_scale_codec::Output + ?Sized>(
            &self,
            __codec_dest_edqy: &mut __CodecOutputEdqy,
        ) {
            match *self {
                OriginCaller::system(ref aa) => {
                    __codec_dest_edqy.push_byte(0u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                OriginCaller::Void(ref aa) => {
                    __codec_dest_edqy.push_byte(1usize as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                _ => (),
            }
        }
    }
    impl _parity_scale_codec::EncodeLike for OriginCaller {}
};
const _: () = {
    #[allow(unknown_lints)]
    #[allow(rust_2018_idioms)]
    extern crate codec as _parity_scale_codec;
    impl _parity_scale_codec::Decode for OriginCaller {
        fn decode<__CodecInputEdqy: _parity_scale_codec::Input>(
            __codec_input_edqy: &mut __CodecInputEdqy,
        ) -> core::result::Result<Self, _parity_scale_codec::Error> {
            match __codec_input_edqy.read_byte().map_err(|e| {
                e.chain("Could not decode `OriginCaller`, failed to read variant byte")
            })? {
                __codec_x_edqy if __codec_x_edqy == 0u8 as u8 => Ok(OriginCaller::system({
                    let __codec_res_edqy =
                        <frame_system::Origin<Runtime> as _parity_scale_codec::Decode>::decode(
                            __codec_input_edqy,
                        );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `OriginCaller::system.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 1usize as u8 => Ok(OriginCaller::Void({
                    let __codec_res_edqy =
                        <::frame_support::Void as _parity_scale_codec::Decode>::decode(
                            __codec_input_edqy,
                        );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `OriginCaller::Void.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                _ => Err("Could not decode `OriginCaller`, variant doesn\'t exist".into()),
            }
        }
    }
};
#[allow(dead_code)]
impl Origin {
    /// Create with system none origin and `frame-system::Config::BaseCallFilter`.
    pub fn none() -> Self {
        <Origin as ::frame_support::traits::OriginTrait>::none()
    }
    /// Create with system root origin and no filter.
    pub fn root() -> Self {
        <Origin as ::frame_support::traits::OriginTrait>::root()
    }
    /// Create with system signed origin and `frame-system::Config::BaseCallFilter`.
    pub fn signed(by: <Runtime as frame_system::Config>::AccountId) -> Self {
        <Origin as ::frame_support::traits::OriginTrait>::signed(by)
    }
}
impl From<frame_system::Origin<Runtime>> for OriginCaller {
    fn from(x: frame_system::Origin<Runtime>) -> Self {
        OriginCaller::system(x)
    }
}
impl From<frame_system::Origin<Runtime>> for Origin {
    /// Convert to runtime origin:
    /// * root origin is built with no filter
    /// * others use `frame-system::Config::BaseCallFilter`
    fn from(x: frame_system::Origin<Runtime>) -> Self {
        let o: OriginCaller = x.into();
        o.into()
    }
}
impl From<OriginCaller> for Origin {
    fn from(x: OriginCaller) -> Self {
        let mut o = Origin {
            caller: x,
            filter: ::frame_support::sp_std::rc::Rc::new(Box::new(|_| true)),
        };
        if !match o.caller {
            OriginCaller::system(frame_system::Origin::<Runtime>::Root) => true,
            _ => false,
        } {
            ::frame_support::traits::OriginTrait::reset_filter(&mut o);
        }
        o
    }
}
impl Into<::frame_support::sp_std::result::Result<frame_system::Origin<Runtime>, Origin>>
    for Origin
{
    /// NOTE: converting to pallet origin loses the origin filter information.
    fn into(self) -> ::frame_support::sp_std::result::Result<frame_system::Origin<Runtime>, Self> {
        if let OriginCaller::system(l) = self.caller {
            Ok(l)
        } else {
            Err(self)
        }
    }
}
impl From<Option<<Runtime as frame_system::Config>::AccountId>> for Origin {
    /// Convert to runtime origin with caller being system signed or none and use filter
    /// `frame-system::Config::BaseCallFilter`.
    fn from(x: Option<<Runtime as frame_system::Config>::AccountId>) -> Self {
        <frame_system::Origin<Runtime>>::from(x).into()
    }
}
pub type System = frame_system::Pallet<Runtime>;
pub type Utility = pallet_utility::Pallet<Runtime>;
pub type Babe = pallet_babe::Pallet<Runtime>;
pub type Timestamp = pallet_timestamp::Pallet<Runtime>;
pub type Balances = pallet_balances::Pallet<Runtime>;
pub type Grandpa = pallet_grandpa::Pallet<Runtime>;
pub type Sudo = pallet_sudo::Pallet<Runtime>;
/// All pallets included in the runtime as a nested tuple of types.
/// Excludes the System pallet.
pub type AllPallets = ((Sudo, (Grandpa, (Balances, (Timestamp, (Babe, (Utility,)))))));
/// All pallets included in the runtime as a nested tuple of types.
pub type AllPalletsWithSystem = ((
    Sudo,
    (
        Grandpa,
        (Balances, (Timestamp, (Babe, (Utility, (System,))))),
    ),
));
/// All modules included in the runtime as a nested tuple of types.
/// Excludes the System pallet.
#[deprecated(note = "use `AllPallets` instead")]
#[allow(dead_code)]
pub type AllModules = ((Sudo, (Grandpa, (Balances, (Timestamp, (Babe, (Utility,)))))));
/// All modules included in the runtime as a nested tuple of types.
#[deprecated(note = "use `AllPalletsWithSystem` instead")]
#[allow(dead_code)]
pub type AllModulesWithSystem = ((
    Sudo,
    (
        Grandpa,
        (Balances, (Timestamp, (Babe, (Utility, (System,))))),
    ),
));
/// Provides an implementation of `PalletInfo` to provide information
/// about the pallet setup in the runtime.
pub struct PalletInfo;
impl self::sp_api_hidden_includes_construct_runtime::hidden_include::traits::PalletInfo
    for PalletInfo
{
    fn index<P: 'static>() -> Option<usize> {
        let type_id =
            self::sp_api_hidden_includes_construct_runtime::hidden_include::sp_std::any::TypeId::of::<
                P,
            >();
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < System > () { return Some (0usize) }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Utility > () { return Some (1usize) }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Babe > () { return Some (2usize) }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Timestamp > () { return Some (3usize) }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Balances > () { return Some (4usize) }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Grandpa > () { return Some (5usize) }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Sudo > () { return Some (6usize) }
        None
    }
    fn name<P: 'static>() -> Option<&'static str> {
        let type_id =
            self::sp_api_hidden_includes_construct_runtime::hidden_include::sp_std::any::TypeId::of::<
                P,
            >();
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < System > () { return Some ("System") }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Utility > () { return Some ("Utility") }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Babe > () { return Some ("Babe") }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Timestamp > () { return Some ("Timestamp") }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Balances > () { return Some ("Balances") }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Grandpa > () { return Some ("Grandpa") }
        if type_id == self :: sp_api_hidden_includes_construct_runtime :: hidden_include :: sp_std :: any :: TypeId :: of :: < Sudo > () { return Some ("Sudo") }
        None
    }
}
pub enum Call {
    #[codec(index = 0u8)]
    System(::frame_support::dispatch::CallableCallFor<System, Runtime>),
    #[codec(index = 1u8)]
    Utility(::frame_support::dispatch::CallableCallFor<Utility, Runtime>),
    #[codec(index = 2u8)]
    Babe(::frame_support::dispatch::CallableCallFor<Babe, Runtime>),
    #[codec(index = 3u8)]
    Timestamp(::frame_support::dispatch::CallableCallFor<Timestamp, Runtime>),
    #[codec(index = 4u8)]
    Balances(::frame_support::dispatch::CallableCallFor<Balances, Runtime>),
    #[codec(index = 5u8)]
    Grandpa(::frame_support::dispatch::CallableCallFor<Grandpa, Runtime>),
    #[codec(index = 6u8)]
    Sudo(::frame_support::dispatch::CallableCallFor<Sudo, Runtime>),
}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::clone::Clone for Call {
    #[inline]
    fn clone(&self) -> Call {
        match (&*self,) {
            (&Call::System(ref __self_0),) => {
                Call::System(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&Call::Utility(ref __self_0),) => {
                Call::Utility(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&Call::Babe(ref __self_0),) => Call::Babe(::core::clone::Clone::clone(&(*__self_0))),
            (&Call::Timestamp(ref __self_0),) => {
                Call::Timestamp(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&Call::Balances(ref __self_0),) => {
                Call::Balances(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&Call::Grandpa(ref __self_0),) => {
                Call::Grandpa(::core::clone::Clone::clone(&(*__self_0)))
            }
            (&Call::Sudo(ref __self_0),) => Call::Sudo(::core::clone::Clone::clone(&(*__self_0))),
        }
    }
}
impl ::core::marker::StructuralPartialEq for Call {}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::cmp::PartialEq for Call {
    #[inline]
    fn eq(&self, other: &Call) -> bool {
        {
            let __self_vi = ::core::intrinsics::discriminant_value(&*self);
            let __arg_1_vi = ::core::intrinsics::discriminant_value(&*other);
            if true && __self_vi == __arg_1_vi {
                match (&*self, &*other) {
                    (&Call::System(ref __self_0), &Call::System(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    (&Call::Utility(ref __self_0), &Call::Utility(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    (&Call::Babe(ref __self_0), &Call::Babe(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    (&Call::Timestamp(ref __self_0), &Call::Timestamp(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    (&Call::Balances(ref __self_0), &Call::Balances(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    (&Call::Grandpa(ref __self_0), &Call::Grandpa(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    (&Call::Sudo(ref __self_0), &Call::Sudo(ref __arg_1_0)) => {
                        (*__self_0) == (*__arg_1_0)
                    }
                    _ => unsafe { ::core::intrinsics::unreachable() },
                }
            } else {
                false
            }
        }
    }
    #[inline]
    fn ne(&self, other: &Call) -> bool {
        {
            let __self_vi = ::core::intrinsics::discriminant_value(&*self);
            let __arg_1_vi = ::core::intrinsics::discriminant_value(&*other);
            if true && __self_vi == __arg_1_vi {
                match (&*self, &*other) {
                    (&Call::System(ref __self_0), &Call::System(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    (&Call::Utility(ref __self_0), &Call::Utility(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    (&Call::Babe(ref __self_0), &Call::Babe(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    (&Call::Timestamp(ref __self_0), &Call::Timestamp(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    (&Call::Balances(ref __self_0), &Call::Balances(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    (&Call::Grandpa(ref __self_0), &Call::Grandpa(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    (&Call::Sudo(ref __self_0), &Call::Sudo(ref __arg_1_0)) => {
                        (*__self_0) != (*__arg_1_0)
                    }
                    _ => unsafe { ::core::intrinsics::unreachable() },
                }
            } else {
                true
            }
        }
    }
}
impl ::core::marker::StructuralEq for Call {}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::cmp::Eq for Call {
    #[inline]
    #[doc(hidden)]
    fn assert_receiver_is_total_eq(&self) -> () {
        {
            let _: ::core::cmp::AssertParamIsEq<
                ::frame_support::dispatch::CallableCallFor<System, Runtime>,
            >;
            let _: ::core::cmp::AssertParamIsEq<
                ::frame_support::dispatch::CallableCallFor<Utility, Runtime>,
            >;
            let _: ::core::cmp::AssertParamIsEq<
                ::frame_support::dispatch::CallableCallFor<Babe, Runtime>,
            >;
            let _: ::core::cmp::AssertParamIsEq<
                ::frame_support::dispatch::CallableCallFor<Timestamp, Runtime>,
            >;
            let _: ::core::cmp::AssertParamIsEq<
                ::frame_support::dispatch::CallableCallFor<Balances, Runtime>,
            >;
            let _: ::core::cmp::AssertParamIsEq<
                ::frame_support::dispatch::CallableCallFor<Grandpa, Runtime>,
            >;
            let _: ::core::cmp::AssertParamIsEq<
                ::frame_support::dispatch::CallableCallFor<Sudo, Runtime>,
            >;
        }
    }
}
const _: () = {
    #[allow(unknown_lints)]
    #[allow(rust_2018_idioms)]
    extern crate codec as _parity_scale_codec;
    impl _parity_scale_codec::Encode for Call {
        fn encode_to<__CodecOutputEdqy: _parity_scale_codec::Output + ?Sized>(
            &self,
            __codec_dest_edqy: &mut __CodecOutputEdqy,
        ) {
            match *self {
                Call::System(ref aa) => {
                    __codec_dest_edqy.push_byte(0u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Call::Utility(ref aa) => {
                    __codec_dest_edqy.push_byte(1u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Call::Babe(ref aa) => {
                    __codec_dest_edqy.push_byte(2u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Call::Timestamp(ref aa) => {
                    __codec_dest_edqy.push_byte(3u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Call::Balances(ref aa) => {
                    __codec_dest_edqy.push_byte(4u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Call::Grandpa(ref aa) => {
                    __codec_dest_edqy.push_byte(5u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                Call::Sudo(ref aa) => {
                    __codec_dest_edqy.push_byte(6u8 as u8);
                    _parity_scale_codec::Encode::encode_to(aa, __codec_dest_edqy);
                }
                _ => (),
            }
        }
    }
    impl _parity_scale_codec::EncodeLike for Call {}
};
const _: () = {
    #[allow(unknown_lints)]
    #[allow(rust_2018_idioms)]
    extern crate codec as _parity_scale_codec;
    impl _parity_scale_codec::Decode for Call {
        fn decode<__CodecInputEdqy: _parity_scale_codec::Input>(
            __codec_input_edqy: &mut __CodecInputEdqy,
        ) -> core::result::Result<Self, _parity_scale_codec::Error> {
            match __codec_input_edqy
                .read_byte()
                .map_err(|e| e.chain("Could not decode `Call`, failed to read variant byte"))?
            {
                __codec_x_edqy if __codec_x_edqy == 0u8 as u8 => Ok(Call::System({
                    let __codec_res_edqy = <::frame_support::dispatch::CallableCallFor<
                        System,
                        Runtime,
                    > as _parity_scale_codec::Decode>::decode(
                        __codec_input_edqy
                    );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Call::System.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 1u8 as u8 => Ok(Call::Utility({
                    let __codec_res_edqy = <::frame_support::dispatch::CallableCallFor<
                        Utility,
                        Runtime,
                    > as _parity_scale_codec::Decode>::decode(
                        __codec_input_edqy
                    );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Call::Utility.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 2u8 as u8 => Ok(Call::Babe({
                    let __codec_res_edqy = <::frame_support::dispatch::CallableCallFor<
                        Babe,
                        Runtime,
                    > as _parity_scale_codec::Decode>::decode(
                        __codec_input_edqy
                    );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Call::Babe.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 3u8 as u8 => Ok(Call::Timestamp({
                    let __codec_res_edqy = <::frame_support::dispatch::CallableCallFor<
                        Timestamp,
                        Runtime,
                    > as _parity_scale_codec::Decode>::decode(
                        __codec_input_edqy
                    );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Call::Timestamp.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 4u8 as u8 => Ok(Call::Balances({
                    let __codec_res_edqy = <::frame_support::dispatch::CallableCallFor<
                        Balances,
                        Runtime,
                    > as _parity_scale_codec::Decode>::decode(
                        __codec_input_edqy
                    );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Call::Balances.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 5u8 as u8 => Ok(Call::Grandpa({
                    let __codec_res_edqy = <::frame_support::dispatch::CallableCallFor<
                        Grandpa,
                        Runtime,
                    > as _parity_scale_codec::Decode>::decode(
                        __codec_input_edqy
                    );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Call::Grandpa.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                __codec_x_edqy if __codec_x_edqy == 6u8 as u8 => Ok(Call::Sudo({
                    let __codec_res_edqy = <::frame_support::dispatch::CallableCallFor<
                        Sudo,
                        Runtime,
                    > as _parity_scale_codec::Decode>::decode(
                        __codec_input_edqy
                    );
                    match __codec_res_edqy {
                        Err(e) => return Err(e.chain("Could not decode `Call::Sudo.0`")),
                        Ok(__codec_res_edqy) => __codec_res_edqy,
                    }
                })),
                _ => Err("Could not decode `Call`, variant doesn\'t exist".into()),
            }
        }
    }
};
impl core::fmt::Debug for Call {
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::System(ref a0) => fmt.debug_tuple("Call::System").field(a0).finish(),
            Self::Utility(ref a0) => fmt.debug_tuple("Call::Utility").field(a0).finish(),
            Self::Babe(ref a0) => fmt.debug_tuple("Call::Babe").field(a0).finish(),
            Self::Timestamp(ref a0) => fmt.debug_tuple("Call::Timestamp").field(a0).finish(),
            Self::Balances(ref a0) => fmt.debug_tuple("Call::Balances").field(a0).finish(),
            Self::Grandpa(ref a0) => fmt.debug_tuple("Call::Grandpa").field(a0).finish(),
            Self::Sudo(ref a0) => fmt.debug_tuple("Call::Sudo").field(a0).finish(),
            _ => Ok(()),
        }
    }
}
impl ::frame_support::dispatch::GetDispatchInfo for Call {
    fn get_dispatch_info(&self) -> ::frame_support::dispatch::DispatchInfo {
        match self {
            Call::System(call) => call.get_dispatch_info(),
            Call::Utility(call) => call.get_dispatch_info(),
            Call::Babe(call) => call.get_dispatch_info(),
            Call::Timestamp(call) => call.get_dispatch_info(),
            Call::Balances(call) => call.get_dispatch_info(),
            Call::Grandpa(call) => call.get_dispatch_info(),
            Call::Sudo(call) => call.get_dispatch_info(),
        }
    }
}
impl ::frame_support::dispatch::GetCallMetadata for Call {
    fn get_call_metadata(&self) -> ::frame_support::dispatch::CallMetadata {
        use ::frame_support::dispatch::GetCallName;
        match self {
            Call::System(call) => {
                let function_name = call.get_call_name();
                let pallet_name = "System";
                ::frame_support::dispatch::CallMetadata {
                    function_name,
                    pallet_name,
                }
            }
            Call::Utility(call) => {
                let function_name = call.get_call_name();
                let pallet_name = "Utility";
                ::frame_support::dispatch::CallMetadata {
                    function_name,
                    pallet_name,
                }
            }
            Call::Babe(call) => {
                let function_name = call.get_call_name();
                let pallet_name = "Babe";
                ::frame_support::dispatch::CallMetadata {
                    function_name,
                    pallet_name,
                }
            }
            Call::Timestamp(call) => {
                let function_name = call.get_call_name();
                let pallet_name = "Timestamp";
                ::frame_support::dispatch::CallMetadata {
                    function_name,
                    pallet_name,
                }
            }
            Call::Balances(call) => {
                let function_name = call.get_call_name();
                let pallet_name = "Balances";
                ::frame_support::dispatch::CallMetadata {
                    function_name,
                    pallet_name,
                }
            }
            Call::Grandpa(call) => {
                let function_name = call.get_call_name();
                let pallet_name = "Grandpa";
                ::frame_support::dispatch::CallMetadata {
                    function_name,
                    pallet_name,
                }
            }
            Call::Sudo(call) => {
                let function_name = call.get_call_name();
                let pallet_name = "Sudo";
                ::frame_support::dispatch::CallMetadata {
                    function_name,
                    pallet_name,
                }
            }
        }
    }
    fn get_module_names() -> &'static [&'static str] {
        &[
            "System",
            "Utility",
            "Babe",
            "Timestamp",
            "Balances",
            "Grandpa",
            "Sudo",
        ]
    }
    fn get_call_names(module: &str) -> &'static [&'static str] {
        use ::frame_support::dispatch::{Callable, GetCallName};
        match module {
            "System" => <<System as Callable<Runtime>>::Call as GetCallName>::get_call_names(),
            "Utility" => <<Utility as Callable<Runtime>>::Call as GetCallName>::get_call_names(),
            "Babe" => <<Babe as Callable<Runtime>>::Call as GetCallName>::get_call_names(),
            "Timestamp" => {
                <<Timestamp as Callable<Runtime>>::Call as GetCallName>::get_call_names()
            }
            "Balances" => <<Balances as Callable<Runtime>>::Call as GetCallName>::get_call_names(),
            "Grandpa" => <<Grandpa as Callable<Runtime>>::Call as GetCallName>::get_call_names(),
            "Sudo" => <<Sudo as Callable<Runtime>>::Call as GetCallName>::get_call_names(),
            _ => ::core::panicking::panic("internal error: entered unreachable code"),
        }
    }
}
impl ::frame_support::dispatch::Dispatchable for Call {
    type Origin = Origin;
    type Config = Call;
    type Info = ::frame_support::weights::DispatchInfo;
    type PostInfo = ::frame_support::weights::PostDispatchInfo;
    fn dispatch(self, origin: Origin) -> ::frame_support::dispatch::DispatchResultWithPostInfo {
        if !<Self::Origin as ::frame_support::traits::OriginTrait>::filter_call(&origin, &self) {
            return ::frame_support::sp_std::result::Result::Err(
                ::frame_support::dispatch::DispatchError::BadOrigin.into(),
            );
        }
        ::frame_support::traits::UnfilteredDispatchable::dispatch_bypass_filter(self, origin)
    }
}
impl ::frame_support::traits::UnfilteredDispatchable for Call {
    type Origin = Origin;
    fn dispatch_bypass_filter(
        self,
        origin: Origin,
    ) -> ::frame_support::dispatch::DispatchResultWithPostInfo {
        match self {
            Call::System(call) => {
                ::frame_support::traits::UnfilteredDispatchable::dispatch_bypass_filter(
                    call, origin,
                )
            }
            Call::Utility(call) => {
                ::frame_support::traits::UnfilteredDispatchable::dispatch_bypass_filter(
                    call, origin,
                )
            }
            Call::Babe(call) => {
                ::frame_support::traits::UnfilteredDispatchable::dispatch_bypass_filter(
                    call, origin,
                )
            }
            Call::Timestamp(call) => {
                ::frame_support::traits::UnfilteredDispatchable::dispatch_bypass_filter(
                    call, origin,
                )
            }
            Call::Balances(call) => {
                ::frame_support::traits::UnfilteredDispatchable::dispatch_bypass_filter(
                    call, origin,
                )
            }
            Call::Grandpa(call) => {
                ::frame_support::traits::UnfilteredDispatchable::dispatch_bypass_filter(
                    call, origin,
                )
            }
            Call::Sudo(call) => {
                ::frame_support::traits::UnfilteredDispatchable::dispatch_bypass_filter(
                    call, origin,
                )
            }
        }
    }
}
impl ::frame_support::traits::IsSubType<::frame_support::dispatch::CallableCallFor<System, Runtime>>
    for Call
{
    #[allow(unreachable_patterns)]
    fn is_sub_type(&self) -> Option<&::frame_support::dispatch::CallableCallFor<System, Runtime>> {
        match *self {
            Call::System(ref r) => Some(r),
            _ => None,
        }
    }
}
impl From<::frame_support::dispatch::CallableCallFor<System, Runtime>> for Call {
    fn from(call: ::frame_support::dispatch::CallableCallFor<System, Runtime>) -> Self {
        Call::System(call)
    }
}
impl
    ::frame_support::traits::IsSubType<::frame_support::dispatch::CallableCallFor<Utility, Runtime>>
    for Call
{
    #[allow(unreachable_patterns)]
    fn is_sub_type(&self) -> Option<&::frame_support::dispatch::CallableCallFor<Utility, Runtime>> {
        match *self {
            Call::Utility(ref r) => Some(r),
            _ => None,
        }
    }
}
impl From<::frame_support::dispatch::CallableCallFor<Utility, Runtime>> for Call {
    fn from(call: ::frame_support::dispatch::CallableCallFor<Utility, Runtime>) -> Self {
        Call::Utility(call)
    }
}
impl ::frame_support::traits::IsSubType<::frame_support::dispatch::CallableCallFor<Babe, Runtime>>
    for Call
{
    #[allow(unreachable_patterns)]
    fn is_sub_type(&self) -> Option<&::frame_support::dispatch::CallableCallFor<Babe, Runtime>> {
        match *self {
            Call::Babe(ref r) => Some(r),
            _ => None,
        }
    }
}
impl From<::frame_support::dispatch::CallableCallFor<Babe, Runtime>> for Call {
    fn from(call: ::frame_support::dispatch::CallableCallFor<Babe, Runtime>) -> Self {
        Call::Babe(call)
    }
}
impl
    ::frame_support::traits::IsSubType<
        ::frame_support::dispatch::CallableCallFor<Timestamp, Runtime>,
    > for Call
{
    #[allow(unreachable_patterns)]
    fn is_sub_type(
        &self,
    ) -> Option<&::frame_support::dispatch::CallableCallFor<Timestamp, Runtime>> {
        match *self {
            Call::Timestamp(ref r) => Some(r),
            _ => None,
        }
    }
}
impl From<::frame_support::dispatch::CallableCallFor<Timestamp, Runtime>> for Call {
    fn from(call: ::frame_support::dispatch::CallableCallFor<Timestamp, Runtime>) -> Self {
        Call::Timestamp(call)
    }
}
impl
    ::frame_support::traits::IsSubType<
        ::frame_support::dispatch::CallableCallFor<Balances, Runtime>,
    > for Call
{
    #[allow(unreachable_patterns)]
    fn is_sub_type(
        &self,
    ) -> Option<&::frame_support::dispatch::CallableCallFor<Balances, Runtime>> {
        match *self {
            Call::Balances(ref r) => Some(r),
            _ => None,
        }
    }
}
impl From<::frame_support::dispatch::CallableCallFor<Balances, Runtime>> for Call {
    fn from(call: ::frame_support::dispatch::CallableCallFor<Balances, Runtime>) -> Self {
        Call::Balances(call)
    }
}
impl
    ::frame_support::traits::IsSubType<::frame_support::dispatch::CallableCallFor<Grandpa, Runtime>>
    for Call
{
    #[allow(unreachable_patterns)]
    fn is_sub_type(&self) -> Option<&::frame_support::dispatch::CallableCallFor<Grandpa, Runtime>> {
        match *self {
            Call::Grandpa(ref r) => Some(r),
            _ => None,
        }
    }
}
impl From<::frame_support::dispatch::CallableCallFor<Grandpa, Runtime>> for Call {
    fn from(call: ::frame_support::dispatch::CallableCallFor<Grandpa, Runtime>) -> Self {
        Call::Grandpa(call)
    }
}
impl ::frame_support::traits::IsSubType<::frame_support::dispatch::CallableCallFor<Sudo, Runtime>>
    for Call
{
    #[allow(unreachable_patterns)]
    fn is_sub_type(&self) -> Option<&::frame_support::dispatch::CallableCallFor<Sudo, Runtime>> {
        match *self {
            Call::Sudo(ref r) => Some(r),
            _ => None,
        }
    }
}
impl From<::frame_support::dispatch::CallableCallFor<Sudo, Runtime>> for Call {
    fn from(call: ::frame_support::dispatch::CallableCallFor<Sudo, Runtime>) -> Self {
        Call::Sudo(call)
    }
}
impl Runtime {
    pub fn metadata() -> ::frame_support::metadata::RuntimeMetadataPrefixed {
        :: frame_support :: metadata :: RuntimeMetadataLastVersion { modules : :: frame_support :: metadata :: DecodeDifferent :: Encode (& [:: frame_support :: metadata :: ModuleMetadata { name : :: frame_support :: metadata :: DecodeDifferent :: Encode ("System") , index : 0u8 , storage : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (frame_system :: Pallet :: < Runtime > :: storage_metadata))) , calls : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (frame_system :: Pallet :: < Runtime > :: call_functions))) , event : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (Runtime :: __module_events_frame_system))) , constants : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (frame_system :: Pallet :: < Runtime > :: module_constants_metadata)) , errors : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (< frame_system :: Pallet < Runtime > as :: frame_support :: metadata :: ModuleErrorMetadata > :: metadata)) , } , :: frame_support :: metadata :: ModuleMetadata { name : :: frame_support :: metadata :: DecodeDifferent :: Encode ("Utility") , index : 1u8 , storage : None , calls : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_utility :: Pallet :: < Runtime > :: call_functions))) , event : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (Runtime :: __module_events_pallet_utility))) , constants : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_utility :: Pallet :: < Runtime > :: module_constants_metadata)) , errors : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (< pallet_utility :: Pallet < Runtime > as :: frame_support :: metadata :: ModuleErrorMetadata > :: metadata)) , } , :: frame_support :: metadata :: ModuleMetadata { name : :: frame_support :: metadata :: DecodeDifferent :: Encode ("Babe") , index : 2u8 , storage : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_babe :: Pallet :: < Runtime > :: storage_metadata))) , calls : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_babe :: Pallet :: < Runtime > :: call_functions))) , event : None , constants : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_babe :: Pallet :: < Runtime > :: module_constants_metadata)) , errors : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (< pallet_babe :: Pallet < Runtime > as :: frame_support :: metadata :: ModuleErrorMetadata > :: metadata)) , } , :: frame_support :: metadata :: ModuleMetadata { name : :: frame_support :: metadata :: DecodeDifferent :: Encode ("Timestamp") , index : 3u8 , storage : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_timestamp :: Pallet :: < Runtime > :: storage_metadata))) , calls : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_timestamp :: Pallet :: < Runtime > :: call_functions))) , event : None , constants : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_timestamp :: Pallet :: < Runtime > :: module_constants_metadata)) , errors : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (< pallet_timestamp :: Pallet < Runtime > as :: frame_support :: metadata :: ModuleErrorMetadata > :: metadata)) , } , :: frame_support :: metadata :: ModuleMetadata { name : :: frame_support :: metadata :: DecodeDifferent :: Encode ("Balances") , index : 4u8 , storage : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_balances :: Pallet :: < Runtime > :: storage_metadata))) , calls : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_balances :: Pallet :: < Runtime > :: call_functions))) , event : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (Runtime :: __module_events_pallet_balances))) , constants : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_balances :: Pallet :: < Runtime > :: module_constants_metadata)) , errors : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (< pallet_balances :: Pallet < Runtime > as :: frame_support :: metadata :: ModuleErrorMetadata > :: metadata)) , } , :: frame_support :: metadata :: ModuleMetadata { name : :: frame_support :: metadata :: DecodeDifferent :: Encode ("Grandpa") , index : 5u8 , storage : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_grandpa :: Pallet :: < Runtime > :: storage_metadata))) , calls : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_grandpa :: Pallet :: < Runtime > :: call_functions))) , event : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (Runtime :: __module_events_pallet_grandpa))) , constants : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_grandpa :: Pallet :: < Runtime > :: module_constants_metadata)) , errors : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (< pallet_grandpa :: Pallet < Runtime > as :: frame_support :: metadata :: ModuleErrorMetadata > :: metadata)) , } , :: frame_support :: metadata :: ModuleMetadata { name : :: frame_support :: metadata :: DecodeDifferent :: Encode ("Sudo") , index : 6u8 , storage : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_sudo :: Pallet :: < Runtime > :: storage_metadata))) , calls : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_sudo :: Pallet :: < Runtime > :: call_functions))) , event : Some (:: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (Runtime :: __module_events_pallet_sudo))) , constants : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (pallet_sudo :: Pallet :: < Runtime > :: module_constants_metadata)) , errors : :: frame_support :: metadata :: DecodeDifferent :: Encode (:: frame_support :: metadata :: FnEncode (< pallet_sudo :: Pallet < Runtime > as :: frame_support :: metadata :: ModuleErrorMetadata > :: metadata)) , }]) , extrinsic : :: frame_support :: metadata :: ExtrinsicMetadata { version : < UncheckedExtrinsic as :: frame_support :: sp_runtime :: traits :: ExtrinsicMetadata > :: VERSION , signed_extensions : < < UncheckedExtrinsic as :: frame_support :: sp_runtime :: traits :: ExtrinsicMetadata > :: SignedExtensions as :: frame_support :: sp_runtime :: traits :: SignedExtension > :: identifier () . into_iter () . map (:: frame_support :: metadata :: DecodeDifferent :: Encode) . collect () , } , } . into ()
    }
}
#[cfg(any(feature = "std", test))]
pub type SystemConfig = frame_system::GenesisConfig;
#[cfg(any(feature = "std", test))]
pub type BabeConfig = pallet_babe::GenesisConfig;
#[cfg(any(feature = "std", test))]
pub type BalancesConfig = pallet_balances::GenesisConfig<Runtime>;
#[cfg(any(feature = "std", test))]
pub type GrandpaConfig = pallet_grandpa::GenesisConfig;
#[cfg(any(feature = "std", test))]
pub type SudoConfig = pallet_sudo::GenesisConfig<Runtime>;
#[cfg(any(feature = "std", test))]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct GenesisConfig {
    pub frame_system: SystemConfig,
    pub pallet_babe: BabeConfig,
    pub pallet_balances: BalancesConfig,
    pub pallet_grandpa: GrandpaConfig,
    pub pallet_sudo: SudoConfig,
}

#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::default::Default for GenesisConfig {
    #[inline]
    fn default() -> GenesisConfig {
        GenesisConfig {
            frame_system: ::core::default::Default::default(),
            pallet_babe: ::core::default::Default::default(),
            pallet_balances: ::core::default::Default::default(),
            pallet_grandpa: ::core::default::Default::default(),
            pallet_sudo: ::core::default::Default::default(),
        }
    }
}
#[cfg(any(feature = "std", test))]
impl ::frame_support::sp_runtime::BuildStorage for GenesisConfig {
    fn assimilate_storage(
        &self,
        storage: &mut ::frame_support::sp_runtime::Storage,
    ) -> std::result::Result<(), String> {
        ::frame_support::sp_runtime::BuildModuleGenesisStorage::<
            Runtime,
            frame_system::__InherentHiddenInstance,
        >::build_module_genesis_storage(&self.frame_system, storage)?;
        ::frame_support::sp_runtime::BuildModuleGenesisStorage::<
            Runtime,
            pallet_babe::__InherentHiddenInstance,
        >::build_module_genesis_storage(&self.pallet_babe, storage)?;
        ::frame_support::sp_runtime::BuildModuleGenesisStorage::<
            Runtime,
            pallet_balances::__InherentHiddenInstance,
        >::build_module_genesis_storage(&self.pallet_balances, storage)?;
        ::frame_support::sp_runtime::BuildModuleGenesisStorage::<
            Runtime,
            pallet_grandpa::__InherentHiddenInstance,
        >::build_module_genesis_storage(&self.pallet_grandpa, storage)?;
        ::frame_support::sp_runtime::BuildModuleGenesisStorage::<
            Runtime,
            pallet_sudo::__InherentHiddenInstance,
        >::build_module_genesis_storage(&self.pallet_sudo, storage)?;
        ::frame_support::BasicExternalities::execute_with_storage(storage, || {
            <AllPalletsWithSystem as ::frame_support::traits::OnGenesis>::on_genesis();
        });
        Ok(())
    }
}
trait InherentDataExt {
    fn create_extrinsics(
        &self,
    ) -> ::frame_support::inherent::Vec<<Block as ::frame_support::inherent::BlockT>::Extrinsic>;
    fn check_extrinsics(&self, block: &Block) -> ::frame_support::inherent::CheckInherentsResult;
}
impl InherentDataExt for ::frame_support::inherent::InherentData {
    fn create_extrinsics(
        &self,
    ) -> ::frame_support::inherent::Vec<<Block as ::frame_support::inherent::BlockT>::Extrinsic>
    {
        use ::frame_support::inherent::{ProvideInherent, Extrinsic};
        let mut inherents = Vec::new();
        if let Some(inherent) = Timestamp::create_inherent(self) {
            inherents.push(UncheckedExtrinsic::new(inherent.into(), None).expect(
                "Runtime UncheckedExtrinsic is not Opaque, so it has to return `Some`; qed",
            ));
        }
        inherents
    }
    fn check_extrinsics(&self, block: &Block) -> ::frame_support::inherent::CheckInherentsResult {
        use ::frame_support::inherent::{ProvideInherent, IsFatalError};
        use ::frame_support::traits::IsSubType;
        use ::frame_support::sp_runtime::traits::Block as _;
        let mut result = ::frame_support::inherent::CheckInherentsResult::new();
        for xt in block.extrinsics() {
            if ::frame_support::inherent::Extrinsic::is_signed(xt).unwrap_or(false) {
                break;
            }
            {
                if let Some(call) = IsSubType::<_>::is_sub_type(&xt.function) {
                    if let Err(e) = Timestamp::check_inherent(call, self) {
                        result
                            .put_error(Timestamp::INHERENT_IDENTIFIER, &e)
                            .expect("There is only one fatal error; qed");
                        if e.is_fatal_error() {
                            return result;
                        }
                    }
                }
            }
        }
        match Timestamp::is_inherent_required(self) {
            Ok(Some(e)) => {
                let found = block.extrinsics().iter().any(|xt| {
                    if ::frame_support::inherent::Extrinsic::is_signed(xt).unwrap_or(false) {
                        return false;
                    }
                    let call: Option<&<Timestamp as ProvideInherent>::Call> =
                        xt.function.is_sub_type();
                    call.is_some()
                });
                if !found {
                    result
                        .put_error(Timestamp::INHERENT_IDENTIFIER, &e)
                        .expect("There is only one fatal error; qed");
                    if e.is_fatal_error() {
                        return result;
                    }
                }
            }
            Ok(None) => (),
            Err(e) => {
                result
                    .put_error(Timestamp::INHERENT_IDENTIFIER, &e)
                    .expect("There is only one fatal error; qed");
                if e.is_fatal_error() {
                    return result;
                }
            }
        }
        result
    }
}
impl ::frame_support::unsigned::ValidateUnsigned for Runtime {
    type Call = Call;
    fn pre_dispatch(
        call: &Self::Call,
    ) -> Result<(), ::frame_support::unsigned::TransactionValidityError> {
        #[allow(unreachable_patterns)]
        match call {
            Call::Babe(inner_call) => Babe::pre_dispatch(inner_call),
            Call::Grandpa(inner_call) => Grandpa::pre_dispatch(inner_call),
            _ => Ok(()),
        }
    }
    fn validate_unsigned(
        #[allow(unused_variables)] source: ::frame_support::unsigned::TransactionSource,
        call: &Self::Call,
    ) -> ::frame_support::unsigned::TransactionValidity {
        #[allow(unreachable_patterns)]
        match call {
            Call::Babe(inner_call) => Babe::validate_unsigned(source, inner_call),
            Call::Grandpa(inner_call) => Grandpa::validate_unsigned(source, inner_call),
            _ => ::frame_support::unsigned::UnknownTransaction::NoUnsignedValidator.into(),
        }
    }
}
/// The address format for describing accounts.
pub type Address = sp_runtime::MultiAddress<AccountId, ()>;
/// Block header type as expected by this runtime.
pub type Header = generic::Header<BlockNumber, BlakeTwo256>;
/// Block type as expected by this runtime.
pub type Block = generic::Block<Header, UncheckedExtrinsic>;
/// A Block signed with a Justification
pub type SignedBlock = generic::SignedBlock<Block>;
/// BlockId type as expected by this runtime.
pub type BlockId = generic::BlockId<Block>;
/// The SignedExtension to the basic transaction logic.
///
/// When you change this, you **MUST** modify [`sign`] in `bin/node/testing/src/keyring.rs`!
///
/// [`sign`]: <../../testing/src/keyring.rs.html>
pub type SignedExtra = (
    frame_system::CheckSpecVersion<Runtime>,
    frame_system::CheckTxVersion<Runtime>,
    frame_system::CheckGenesis<Runtime>,
    frame_system::CheckEra<Runtime>,
    frame_system::CheckNonce<Runtime>,
    frame_system::CheckWeight<Runtime>,
);
/// Unchecked extrinsic type as expected by this runtime.
pub type UncheckedExtrinsic = generic::UncheckedExtrinsic<Address, Call, Signature, SignedExtra>;
/// The payload being signed in transactions.
pub type SignedPayload = generic::SignedPayload<Call, SignedExtra>;
/// Extrinsic type that has already been checked.
pub type CheckedExtrinsic = generic::CheckedExtrinsic<AccountId, Call, SignedExtra>;
/// Executive: handles dispatch to the various modules.
pub type Executive = frame_executive::Executive<
    Runtime,
    Block,
    frame_system::ChainContext<Runtime>,
    Runtime,
    AllPallets,
    (),
>;
#[doc(hidden)]
mod sp_api_hidden_includes_IMPL_RUNTIME_APIS {
    pub extern crate sp_api as sp_api;
}
pub struct RuntimeApi {}
/// Implements all runtime apis for the client side.
#[cfg(any(feature = "std", test))]
pub struct RuntimeApiImpl<
    Block: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT,
    C: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<Block> + 'static,
> where
    C::StateBackend: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<Block>,
    >,
{
    call: &'static C,
    commit_on_success: std::cell::RefCell<bool>,
    initialized_block: std::cell::RefCell<
        Option<self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<Block>>,
    >,
    changes: std::cell::RefCell<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::OverlayedChanges,
    >,
    storage_transaction_cache: std::cell::RefCell<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StorageTransactionCache<
            Block,
            C::StateBackend,
        >,
    >,
    recorder: Option<self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ProofRecorder<Block>>,
}
#[cfg(any(feature = "std", test))]
unsafe impl<
        Block: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT,
        C: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<Block>,
    > Send for RuntimeApiImpl<Block, C>
where
    C::StateBackend: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<Block>,
    >,
{
}
#[cfg(any(feature = "std", test))]
unsafe impl<
        Block: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT,
        C: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<Block>,
    > Sync for RuntimeApiImpl<Block, C>
where
    C::StateBackend: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<Block>,
    >,
{
}
#[cfg(any(feature = "std", test))]
impl<
        Block: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT,
        C: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<Block>,
    > self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiExt<Block>
    for RuntimeApiImpl<Block, C>
where
    C::StateBackend: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<Block>,
    >,
{
    type StateBackend = C::StateBackend;
    fn execute_in_transaction<
        F: FnOnce(
            &Self,
        )
            -> self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::TransactionOutcome<R>,
        R,
    >(
        &self,
        call: F,
    ) -> R
    where
        Self: Sized,
    {
        self.changes.borrow_mut().start_transaction();
        *self.commit_on_success.borrow_mut() = false;
        let res = call(self);
        *self.commit_on_success.borrow_mut() = true;
        self.commit_or_rollback(match res {
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::TransactionOutcome::Commit(
                _,
            ) => true,
            _ => false,
        });
        res.into_inner()
    }
    fn has_api<A: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::RuntimeApiInfo + ?Sized>(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<Block>,
    ) -> std::result::Result<bool, self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError>
    where
        Self: Sized,
    {
        self.call
            .runtime_version_at(at)
            .map(|v| v.has_api_with(&A::ID, |v| v == A::VERSION))
    }
    fn has_api_with<
        A: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::RuntimeApiInfo + ?Sized,
        P: Fn(u32) -> bool,
    >(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<Block>,
        pred: P,
    ) -> std::result::Result<bool, self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError>
    where
        Self: Sized,
    {
        self.call
            .runtime_version_at(at)
            .map(|v| v.has_api_with(&A::ID, pred))
    }
    fn record_proof(&mut self) {
        self.recorder = Some(Default::default());
    }
    fn extract_proof(
        &mut self,
    ) -> Option<self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StorageProof> {
        self.recorder.take().map(|recorder| {
            let trie_nodes = recorder
                .read()
                .iter()
                .filter_map(|(_k, v)| v.as_ref().map(|v| v.to_vec()))
                .collect();
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StorageProof::new(trie_nodes)
        })
    }
    fn into_storage_changes(
        &self,
        backend: &Self::StateBackend,
        changes_trie_state: Option<
            &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ChangesTrieState<
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<Block>,
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NumberFor<Block>,
            >,
        >,
        parent_hash: Block::Hash,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StorageChanges<
            C::StateBackend,
            Block,
        >,
        String,
    >
    where
        Self: Sized,
    {
        self.initialized_block.borrow_mut().take();
        self.changes
            .replace(Default::default())
            .into_storage_changes(
                backend,
                changes_trie_state,
                parent_hash,
                self.storage_transaction_cache.replace(Default::default()),
            )
    }
}
#[cfg(any(feature = "std", test))]
impl<Block: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT, C>
    self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ConstructRuntimeApi<Block, C>
    for RuntimeApi
where
    C: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<Block> + 'static,
    C::StateBackend: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<Block>,
    >,
{
    type RuntimeApi = RuntimeApiImpl<Block, C>;
    fn construct_runtime_api<'a>(
        call: &'a C,
    ) -> self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiRef<'a, Self::RuntimeApi> {
        RuntimeApiImpl {
            call: unsafe { std::mem::transmute(call) },
            commit_on_success: true.into(),
            initialized_block: None.into(),
            changes: Default::default(),
            recorder: Default::default(),
            storage_transaction_cache: Default::default(),
        }
        .into()
    }
}
#[cfg(any(feature = "std", test))]
impl<
        Block: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT,
        C: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<Block>,
    > RuntimeApiImpl<Block, C>
where
    C::StateBackend: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<Block>,
    >,
{
    fn call_api_at<
        R: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode
            + self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Decode
            + PartialEq,
        F: FnOnce(
            &C,
            &Self,
            &std::cell::RefCell<
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::OverlayedChanges,
            >,
            &std::cell::RefCell<
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StorageTransactionCache<
                    Block,
                    C::StateBackend,
                >,
            >,
            &std::cell::RefCell<
                Option<self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<Block>>,
            >,
            &Option<self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ProofRecorder<Block>>,
        ) -> std::result::Result<
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<R>,
            E,
        >,
        E,
    >(
        &self,
        call_api_at: F,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<R>,
        E,
    > {
        if *self.commit_on_success.borrow() {
            self.changes.borrow_mut().start_transaction();
        }
        let res = call_api_at(
            &self.call,
            self,
            &self.changes,
            &self.storage_transaction_cache,
            &self.initialized_block,
            &self.recorder,
        );
        self.commit_or_rollback(res.is_ok());
        res
    }
    fn commit_or_rollback(&self, commit: bool) {
        let proof = "\
					We only close a transaction when we opened one ourself.
					Other parts of the runtime that make use of transactions (state-machine)
					also balance their transactions. The runtime cannot close client initiated
					transactions. qed";
        if *self.commit_on_success.borrow() {
            if commit {
                self.changes.borrow_mut().commit_transaction().expect(proof);
            } else {
                self.changes
                    .borrow_mut()
                    .rollback_transaction()
                    .expect(proof);
            }
        }
    }
}
impl sp_api::runtime_decl_for_Core::Core<Block> for Runtime {
    fn version() -> RuntimeVersion {
        VERSION
    }
    fn execute_block(block: Block) {
        Executive::execute_block(block);
    }
    fn initialize_block(header: &<Block as BlockT>::Header) {
        Executive::initialize_block(header)
    }
}
impl sp_api::runtime_decl_for_Metadata::Metadata<Block> for Runtime {
    fn metadata() -> OpaqueMetadata {
        Runtime::metadata().into()
    }
}
impl sp_block_builder::runtime_decl_for_BlockBuilder::BlockBuilder<Block> for Runtime {
    fn apply_extrinsic(extrinsic: <Block as BlockT>::Extrinsic) -> ApplyExtrinsicResult {
        Executive::apply_extrinsic(extrinsic)
    }
    fn finalize_block() -> <Block as BlockT>::Header {
        Executive::finalize_block()
    }
    fn inherent_extrinsics(data: InherentData) -> Vec<<Block as BlockT>::Extrinsic> {
        data.create_extrinsics()
    }
    fn check_inherents(block: Block, data: InherentData) -> CheckInherentsResult {
        data.check_extrinsics(&block)
    }
    fn random_seed() -> <Block as BlockT>::Hash {
        pallet_babe::RandomnessFromOneEpochAgo::<Runtime>::random_seed().0
    }
}
impl sp_transaction_pool :: runtime_api :: runtime_decl_for_TaggedTransactionQueue :: TaggedTransactionQueue < Block > for Runtime { fn validate_transaction (source : TransactionSource , tx : < Block as BlockT > :: Extrinsic) -> TransactionValidity { Executive :: validate_transaction (source , tx) } }
impl fg_primitives::runtime_decl_for_GrandpaApi::GrandpaApi<Block> for Runtime {
    fn grandpa_authorities() -> GrandpaAuthorityList {
        Grandpa::grandpa_authorities()
    }
    fn submit_report_equivocation_unsigned_extrinsic(
        equivocation_proof: fg_primitives::EquivocationProof<
            <Block as BlockT>::Hash,
            NumberFor<Block>,
        >,
        key_owner_proof: fg_primitives::OpaqueKeyOwnershipProof,
    ) -> Option<()> {
        let key_owner_proof = key_owner_proof.decode()?;
        Grandpa::submit_unsigned_equivocation_report(equivocation_proof, key_owner_proof)
    }
    fn generate_key_ownership_proof(
        _set_id: fg_primitives::SetId,
        authority_id: GrandpaId,
    ) -> Option<fg_primitives::OpaqueKeyOwnershipProof> {
        use codec::Encode;
        None
    }
}
impl sp_consensus_babe::runtime_decl_for_BabeApi::BabeApi<Block> for Runtime {
    fn configuration() -> sp_consensus_babe::BabeGenesisConfiguration {
        sp_consensus_babe::BabeGenesisConfiguration {
            slot_duration: Babe::slot_duration(),
            epoch_length: EpochDuration::get(),
            c: BABE_GENESIS_EPOCH_CONFIG.c,
            genesis_authorities: Babe::authorities(),
            randomness: Babe::randomness(),
            allowed_slots: BABE_GENESIS_EPOCH_CONFIG.allowed_slots,
        }
    }
    fn current_epoch_start() -> sp_consensus_babe::Slot {
        Babe::current_epoch_start()
    }
    fn current_epoch() -> sp_consensus_babe::Epoch {
        Babe::current_epoch()
    }
    fn next_epoch() -> sp_consensus_babe::Epoch {
        Babe::next_epoch()
    }
    fn generate_key_ownership_proof(
        _slot: sp_consensus_babe::Slot,
        authority_id: sp_consensus_babe::AuthorityId,
    ) -> Option<sp_consensus_babe::OpaqueKeyOwnershipProof> {
        None
    }
    fn submit_report_equivocation_unsigned_extrinsic(
        equivocation_proof: sp_consensus_babe::EquivocationProof<<Block as BlockT>::Header>,
        key_owner_proof: sp_consensus_babe::OpaqueKeyOwnershipProof,
    ) -> Option<()> {
        let key_owner_proof = key_owner_proof.decode()?;
        Babe::submit_unsigned_equivocation_report(equivocation_proof, key_owner_proof)
    }
}
impl sp_session::runtime_decl_for_SessionKeys::SessionKeys<Block> for Runtime {
    fn generate_session_keys(seed: Option<Vec<u8>>) -> Vec<u8> {
        SessionKeys::generate(seed)
    }
    fn decode_session_keys(encoded: Vec<u8>) -> Option<Vec<(Vec<u8>, KeyTypeId)>> {
        SessionKeys::decode_into_raw_public_keys(&encoded)
    }
}
#[cfg(any(feature = "std", test))]
impl<
        __SR_API_BLOCK__: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT
            + std::panic::UnwindSafe
            + std::panic::RefUnwindSafe,
        RuntimeApiImplCall: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<__SR_API_BLOCK__>
            + 'static,
    > sp_api::Core<__SR_API_BLOCK__> for RuntimeApiImpl<__SR_API_BLOCK__, RuntimeApiImplCall>
where
    RuntimeApiImplCall::StateBackend:
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<__SR_API_BLOCK__>,
        >,
    RuntimeVersion: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    __SR_API_BLOCK__: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    <__SR_API_BLOCK__ as BlockT>::Header: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    __SR_API_BLOCK__::Header: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
{
    fn Core_version_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<()>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<RuntimeVersion>,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self.call_api_at(
            |call_runtime_at,
             core_api,
             changes,
             storage_transaction_cache,
             initialized_block,
             recorder| {
                sp_api::runtime_decl_for_Core::version_call_api_at(
                    call_runtime_at,
                    core_api,
                    at,
                    params_encoded,
                    changes,
                    storage_transaction_cache,
                    initialized_block,
                    params.map(|p| {
                        sp_api::runtime_decl_for_Core::version_native_call_generator::<
                            Runtime,
                            __SR_API_BLOCK__,
                            Block,
                        >()
                    }),
                    context,
                    recorder,
                )
            },
        )
    }
    fn Core_execute_block_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(__SR_API_BLOCK__)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<()>,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self.call_api_at(
            |call_runtime_at,
             core_api,
             changes,
             storage_transaction_cache,
             initialized_block,
             recorder| {
                sp_api::runtime_decl_for_Core::execute_block_call_api_at(
                    call_runtime_at,
                    core_api,
                    at,
                    params_encoded,
                    changes,
                    storage_transaction_cache,
                    initialized_block,
                    params.map(|p| {
                        sp_api::runtime_decl_for_Core::execute_block_native_call_generator::<
                            Runtime,
                            __SR_API_BLOCK__,
                            Block,
                        >(p)
                    }),
                    context,
                    recorder,
                )
            },
        )
    }
    fn Core_initialize_block_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(&<__SR_API_BLOCK__ as BlockT>::Header)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<()>,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self.call_api_at(
            |call_runtime_at,
             core_api,
             changes,
             storage_transaction_cache,
             initialized_block,
             recorder| {
                sp_api::runtime_decl_for_Core::initialize_block_call_api_at(
                    call_runtime_at,
                    core_api,
                    at,
                    params_encoded,
                    changes,
                    storage_transaction_cache,
                    initialized_block,
                    params.map(|p| {
                        sp_api::runtime_decl_for_Core::initialize_block_native_call_generator::<
                            Runtime,
                            __SR_API_BLOCK__,
                            Block,
                        >(p)
                    }),
                    context,
                    recorder,
                )
            },
        )
    }
}
#[cfg(any(feature = "std", test))]
impl<
        __SR_API_BLOCK__: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT
            + std::panic::UnwindSafe
            + std::panic::RefUnwindSafe,
        RuntimeApiImplCall: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<__SR_API_BLOCK__>
            + 'static,
    > sp_api::Metadata<__SR_API_BLOCK__> for RuntimeApiImpl<__SR_API_BLOCK__, RuntimeApiImplCall>
where
    RuntimeApiImplCall::StateBackend:
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<__SR_API_BLOCK__>,
        >,
    OpaqueMetadata: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    __SR_API_BLOCK__::Header: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
{
    fn Metadata_metadata_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<()>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<OpaqueMetadata>,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self.call_api_at(
            |call_runtime_at,
             core_api,
             changes,
             storage_transaction_cache,
             initialized_block,
             recorder| {
                sp_api::runtime_decl_for_Metadata::metadata_call_api_at(
                    call_runtime_at,
                    core_api,
                    at,
                    params_encoded,
                    changes,
                    storage_transaction_cache,
                    initialized_block,
                    params.map(|p| {
                        sp_api::runtime_decl_for_Metadata::metadata_native_call_generator::<
                            Runtime,
                            __SR_API_BLOCK__,
                            Block,
                        >()
                    }),
                    context,
                    recorder,
                )
            },
        )
    }
}
#[cfg(any(feature = "std", test))]
impl<
        __SR_API_BLOCK__: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT
            + std::panic::UnwindSafe
            + std::panic::RefUnwindSafe,
        RuntimeApiImplCall: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<__SR_API_BLOCK__>
            + 'static,
    > sp_block_builder::BlockBuilder<__SR_API_BLOCK__>
    for RuntimeApiImpl<__SR_API_BLOCK__, RuntimeApiImplCall>
where
    RuntimeApiImplCall::StateBackend:
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<__SR_API_BLOCK__>,
        >,
    <__SR_API_BLOCK__ as BlockT>::Extrinsic: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    ApplyExtrinsicResult: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    <__SR_API_BLOCK__ as BlockT>::Header: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    InherentData: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    Vec<<__SR_API_BLOCK__ as BlockT>::Extrinsic>:
        std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    __SR_API_BLOCK__: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    InherentData: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    CheckInherentsResult: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    <__SR_API_BLOCK__ as BlockT>::Hash: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    __SR_API_BLOCK__::Header: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
{
    fn BlockBuilder_apply_extrinsic_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(<__SR_API_BLOCK__ as BlockT>::Extrinsic)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            ApplyExtrinsicResult,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_block_builder :: runtime_decl_for_BlockBuilder :: apply_extrinsic_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_block_builder :: runtime_decl_for_BlockBuilder :: apply_extrinsic_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p) }) , context , recorder) })
    }
    fn BlockBuilder_finalize_block_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<()>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            <__SR_API_BLOCK__ as BlockT>::Header,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_block_builder :: runtime_decl_for_BlockBuilder :: finalize_block_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_block_builder :: runtime_decl_for_BlockBuilder :: finalize_block_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > () }) , context , recorder) })
    }
    fn BlockBuilder_inherent_extrinsics_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(InherentData)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            Vec<<__SR_API_BLOCK__ as BlockT>::Extrinsic>,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_block_builder :: runtime_decl_for_BlockBuilder :: inherent_extrinsics_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_block_builder :: runtime_decl_for_BlockBuilder :: inherent_extrinsics_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p) }) , context , recorder) })
    }
    fn BlockBuilder_check_inherents_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(__SR_API_BLOCK__, InherentData)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            CheckInherentsResult,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_block_builder :: runtime_decl_for_BlockBuilder :: check_inherents_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_block_builder :: runtime_decl_for_BlockBuilder :: check_inherents_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p . 0 , p . 1) }) , context , recorder) })
    }
    fn BlockBuilder_random_seed_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<()>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            <__SR_API_BLOCK__ as BlockT>::Hash,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_block_builder :: runtime_decl_for_BlockBuilder :: random_seed_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_block_builder :: runtime_decl_for_BlockBuilder :: random_seed_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > () }) , context , recorder) })
    }
}
#[cfg(any(feature = "std", test))]
impl<
        __SR_API_BLOCK__: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT
            + std::panic::UnwindSafe
            + std::panic::RefUnwindSafe,
        RuntimeApiImplCall: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<__SR_API_BLOCK__>
            + 'static,
    > sp_transaction_pool::runtime_api::TaggedTransactionQueue<__SR_API_BLOCK__>
    for RuntimeApiImpl<__SR_API_BLOCK__, RuntimeApiImplCall>
where
    RuntimeApiImplCall::StateBackend:
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<__SR_API_BLOCK__>,
        >,
    TransactionSource: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    <__SR_API_BLOCK__ as BlockT>::Extrinsic: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    TransactionValidity: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    __SR_API_BLOCK__::Header: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
{
    fn TaggedTransactionQueue_validate_transaction_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(TransactionSource, <__SR_API_BLOCK__ as BlockT>::Extrinsic)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            TransactionValidity,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_transaction_pool :: runtime_api :: runtime_decl_for_TaggedTransactionQueue :: validate_transaction_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_transaction_pool :: runtime_api :: runtime_decl_for_TaggedTransactionQueue :: validate_transaction_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p . 0 , p . 1) }) , context , recorder) })
    }
}
#[cfg(any(feature = "std", test))]
impl<
        __SR_API_BLOCK__: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT
            + std::panic::UnwindSafe
            + std::panic::RefUnwindSafe,
        RuntimeApiImplCall: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<__SR_API_BLOCK__>
            + 'static,
    > fg_primitives::GrandpaApi<__SR_API_BLOCK__>
    for RuntimeApiImpl<__SR_API_BLOCK__, RuntimeApiImplCall>
where
    RuntimeApiImplCall::StateBackend:
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<__SR_API_BLOCK__>,
        >,
    GrandpaAuthorityList: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    fg_primitives::EquivocationProof<
        <__SR_API_BLOCK__ as BlockT>::Hash,
        NumberFor<__SR_API_BLOCK__>,
    >: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    fg_primitives::OpaqueKeyOwnershipProof: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    Option<()>: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    fg_primitives::SetId: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    GrandpaId: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    Option<fg_primitives::OpaqueKeyOwnershipProof>:
        std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    __SR_API_BLOCK__::Header: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
{
    fn GrandpaApi_grandpa_authorities_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<()>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            GrandpaAuthorityList,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { fg_primitives :: runtime_decl_for_GrandpaApi :: grandpa_authorities_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { fg_primitives :: runtime_decl_for_GrandpaApi :: grandpa_authorities_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > () }) , context , recorder) })
    }
    fn GrandpaApi_submit_report_equivocation_unsigned_extrinsic_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(
            fg_primitives::EquivocationProof<
                <__SR_API_BLOCK__ as BlockT>::Hash,
                NumberFor<__SR_API_BLOCK__>,
            >,
            fg_primitives::OpaqueKeyOwnershipProof,
        )>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<Option<()>>,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { fg_primitives :: runtime_decl_for_GrandpaApi :: submit_report_equivocation_unsigned_extrinsic_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { fg_primitives :: runtime_decl_for_GrandpaApi :: submit_report_equivocation_unsigned_extrinsic_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p . 0 , p . 1) }) , context , recorder) })
    }
    fn GrandpaApi_generate_key_ownership_proof_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(fg_primitives::SetId, GrandpaId)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            Option<fg_primitives::OpaqueKeyOwnershipProof>,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { fg_primitives :: runtime_decl_for_GrandpaApi :: generate_key_ownership_proof_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { fg_primitives :: runtime_decl_for_GrandpaApi :: generate_key_ownership_proof_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p . 0 , p . 1) }) , context , recorder) })
    }
}
#[cfg(any(feature = "std", test))]
impl<
        __SR_API_BLOCK__: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT
            + std::panic::UnwindSafe
            + std::panic::RefUnwindSafe,
        RuntimeApiImplCall: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<__SR_API_BLOCK__>
            + 'static,
    > sp_consensus_babe::BabeApi<__SR_API_BLOCK__>
    for RuntimeApiImpl<__SR_API_BLOCK__, RuntimeApiImplCall>
where
    RuntimeApiImplCall::StateBackend:
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<__SR_API_BLOCK__>,
        >,
    sp_consensus_babe::BabeGenesisConfiguration: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    sp_consensus_babe::Slot: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    sp_consensus_babe::Epoch: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    sp_consensus_babe::Epoch: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    sp_consensus_babe::Slot: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    sp_consensus_babe::AuthorityId: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    Option<sp_consensus_babe::OpaqueKeyOwnershipProof>:
        std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    sp_consensus_babe::EquivocationProof<<__SR_API_BLOCK__ as BlockT>::Header>:
        std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    sp_consensus_babe::OpaqueKeyOwnershipProof: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    Option<()>: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    __SR_API_BLOCK__::Header: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
{
    fn BabeApi_configuration_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<()>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            sp_consensus_babe::BabeGenesisConfiguration,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_consensus_babe :: runtime_decl_for_BabeApi :: configuration_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_consensus_babe :: runtime_decl_for_BabeApi :: configuration_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > () }) , context , recorder) })
    }
    fn BabeApi_current_epoch_start_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<()>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            sp_consensus_babe::Slot,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_consensus_babe :: runtime_decl_for_BabeApi :: current_epoch_start_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_consensus_babe :: runtime_decl_for_BabeApi :: current_epoch_start_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > () }) , context , recorder) })
    }
    fn BabeApi_current_epoch_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<()>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            sp_consensus_babe::Epoch,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_consensus_babe :: runtime_decl_for_BabeApi :: current_epoch_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_consensus_babe :: runtime_decl_for_BabeApi :: current_epoch_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > () }) , context , recorder) })
    }
    fn BabeApi_next_epoch_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<()>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            sp_consensus_babe::Epoch,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_consensus_babe :: runtime_decl_for_BabeApi :: next_epoch_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_consensus_babe :: runtime_decl_for_BabeApi :: next_epoch_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > () }) , context , recorder) })
    }
    fn BabeApi_generate_key_ownership_proof_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(sp_consensus_babe::Slot, sp_consensus_babe::AuthorityId)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            Option<sp_consensus_babe::OpaqueKeyOwnershipProof>,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_consensus_babe :: runtime_decl_for_BabeApi :: generate_key_ownership_proof_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_consensus_babe :: runtime_decl_for_BabeApi :: generate_key_ownership_proof_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p . 0 , p . 1) }) , context , recorder) })
    }
    fn BabeApi_submit_report_equivocation_unsigned_extrinsic_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(
            sp_consensus_babe::EquivocationProof<<__SR_API_BLOCK__ as BlockT>::Header>,
            sp_consensus_babe::OpaqueKeyOwnershipProof,
        )>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<Option<()>>,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_consensus_babe :: runtime_decl_for_BabeApi :: submit_report_equivocation_unsigned_extrinsic_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_consensus_babe :: runtime_decl_for_BabeApi :: submit_report_equivocation_unsigned_extrinsic_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p . 0 , p . 1) }) , context , recorder) })
    }
}
#[cfg(any(feature = "std", test))]
impl<
        __SR_API_BLOCK__: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockT
            + std::panic::UnwindSafe
            + std::panic::RefUnwindSafe,
        RuntimeApiImplCall: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::CallApiAt<__SR_API_BLOCK__>
            + 'static,
    > sp_session::SessionKeys<__SR_API_BLOCK__>
    for RuntimeApiImpl<__SR_API_BLOCK__, RuntimeApiImplCall>
where
    RuntimeApiImplCall::StateBackend:
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::StateBackend<
            self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::HashFor<__SR_API_BLOCK__>,
        >,
    Option<Vec<u8>>: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    Vec<u8>: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    Vec<u8>: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    Option<Vec<(Vec<u8>, KeyTypeId)>>: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    __SR_API_BLOCK__::Header: std::panic::UnwindSafe + std::panic::RefUnwindSafe,
{
    fn SessionKeys_generate_session_keys_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(Option<Vec<u8>>)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<Vec<u8>>,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_session :: runtime_decl_for_SessionKeys :: generate_session_keys_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_session :: runtime_decl_for_SessionKeys :: generate_session_keys_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p) }) , context , recorder) })
    }
    fn SessionKeys_decode_session_keys_runtime_api_impl(
        &self,
        at: &self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::BlockId<__SR_API_BLOCK__>,
        context: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ExecutionContext,
        params: Option<(Vec<u8>)>,
        params_encoded: Vec<u8>,
    ) -> std::result::Result<
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::NativeOrEncoded<
            Option<Vec<(Vec<u8>, KeyTypeId)>>,
        >,
        self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApiError,
    > {
        self . call_api_at (| call_runtime_at , core_api , changes , storage_transaction_cache , initialized_block , recorder | { sp_session :: runtime_decl_for_SessionKeys :: decode_session_keys_call_api_at (call_runtime_at , core_api , at , params_encoded , changes , storage_transaction_cache , initialized_block , params . map (| p | { sp_session :: runtime_decl_for_SessionKeys :: decode_session_keys_native_call_generator :: < Runtime , __SR_API_BLOCK__ , Block > (p) }) , context , recorder) })
    }
}
const RUNTIME_API_VERSIONS: self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::ApisVec =
    ::sp_version::sp_std::borrow::Cow::Borrowed(&[
        (
            sp_api::runtime_decl_for_Core::ID,
            sp_api::runtime_decl_for_Core::VERSION,
        ),
        (
            sp_api::runtime_decl_for_Metadata::ID,
            sp_api::runtime_decl_for_Metadata::VERSION,
        ),
        (
            sp_block_builder::runtime_decl_for_BlockBuilder::ID,
            sp_block_builder::runtime_decl_for_BlockBuilder::VERSION,
        ),
        (
            sp_transaction_pool::runtime_api::runtime_decl_for_TaggedTransactionQueue::ID,
            sp_transaction_pool::runtime_api::runtime_decl_for_TaggedTransactionQueue::VERSION,
        ),
        (
            fg_primitives::runtime_decl_for_GrandpaApi::ID,
            fg_primitives::runtime_decl_for_GrandpaApi::VERSION,
        ),
        (
            sp_consensus_babe::runtime_decl_for_BabeApi::ID,
            sp_consensus_babe::runtime_decl_for_BabeApi::VERSION,
        ),
        (
            sp_session::runtime_decl_for_SessionKeys::ID,
            sp_session::runtime_decl_for_SessionKeys::VERSION,
        ),
    ]);
pub mod api {
    use super::*;
    #[cfg(feature = "std")]
    pub fn dispatch(method: &str, mut __sp_api__input_data: &[u8]) -> Option<Vec<u8>> {
        match method {
            "Core_version" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let () : () = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "version" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    #[allow(deprecated)]
                    <Runtime as sp_api::runtime_decl_for_Core::Core<Block>>::version()
                }),
            ),
            "Core_execute_block" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (block) : (Block) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "execute_block" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    #[allow(deprecated)]
                    <Runtime as sp_api::runtime_decl_for_Core::Core<Block>>::execute_block(block)
                }),
            ),
            "Core_initialize_block" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (header) : (< Block as BlockT > :: Header) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "initialize_block" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    #[allow(deprecated)]
                    <Runtime as sp_api::runtime_decl_for_Core::Core<Block>>::initialize_block(
                        &header,
                    )
                }),
            ),
            "Metadata_metadata" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let () : () = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "metadata" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    #[allow(deprecated)]
                    <Runtime as sp_api::runtime_decl_for_Metadata::Metadata<Block>>::metadata()
                }),
            ),
            "BlockBuilder_apply_extrinsic" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (extrinsic) : (< Block as BlockT > :: Extrinsic) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "apply_extrinsic" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    #[allow(deprecated)]
                    <Runtime as sp_block_builder::runtime_decl_for_BlockBuilder::BlockBuilder<
                        Block,
                    >>::apply_extrinsic(extrinsic)
                }),
            ),
            "BlockBuilder_finalize_block" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let () : () = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "finalize_block" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    #[allow(deprecated)]
                    <Runtime as sp_block_builder::runtime_decl_for_BlockBuilder::BlockBuilder<
                        Block,
                    >>::finalize_block()
                }),
            ),
            "BlockBuilder_inherent_extrinsics" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (data) : (InherentData) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "inherent_extrinsics" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    #[allow(deprecated)]
                    <Runtime as sp_block_builder::runtime_decl_for_BlockBuilder::BlockBuilder<
                        Block,
                    >>::inherent_extrinsics(data)
                }),
            ),
            "BlockBuilder_check_inherents" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (block , data) : (Block , InherentData) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "check_inherents" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    #[allow(deprecated)]
                    <Runtime as sp_block_builder::runtime_decl_for_BlockBuilder::BlockBuilder<
                        Block,
                    >>::check_inherents(block, data)
                }),
            ),
            "BlockBuilder_random_seed" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let () : () = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "random_seed" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    #[allow(deprecated)]
                    <Runtime as sp_block_builder::runtime_decl_for_BlockBuilder::BlockBuilder<
                        Block,
                    >>::random_seed()
                }),
            ),
            "TaggedTransactionQueue_validate_transaction" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (source , tx) : (TransactionSource , < Block as BlockT > :: Extrinsic) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "validate_transaction" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as sp_transaction_pool :: runtime_api :: runtime_decl_for_TaggedTransactionQueue :: TaggedTransactionQueue < Block > > :: validate_transaction (source , tx)
                }),
            ),
            "GrandpaApi_grandpa_authorities" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let () : () = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "grandpa_authorities" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as fg_primitives :: runtime_decl_for_GrandpaApi :: GrandpaApi < Block > > :: grandpa_authorities ()
                }),
            ),
            "GrandpaApi_submit_report_equivocation_unsigned_extrinsic" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (equivocation_proof , key_owner_proof) : (fg_primitives :: EquivocationProof < < Block as BlockT > :: Hash , NumberFor < Block > > , fg_primitives :: OpaqueKeyOwnershipProof) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "submit_report_equivocation_unsigned_extrinsic" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as fg_primitives :: runtime_decl_for_GrandpaApi :: GrandpaApi < Block > > :: submit_report_equivocation_unsigned_extrinsic (equivocation_proof , key_owner_proof)
                }),
            ),
            "GrandpaApi_generate_key_ownership_proof" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (_set_id , authority_id) : (fg_primitives :: SetId , GrandpaId) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "generate_key_ownership_proof" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as fg_primitives :: runtime_decl_for_GrandpaApi :: GrandpaApi < Block > > :: generate_key_ownership_proof (_set_id , authority_id)
                }),
            ),
            "BabeApi_configuration" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let () : () = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "configuration" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as sp_consensus_babe :: runtime_decl_for_BabeApi :: BabeApi < Block > > :: configuration ()
                }),
            ),
            "BabeApi_current_epoch_start" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let () : () = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "current_epoch_start" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as sp_consensus_babe :: runtime_decl_for_BabeApi :: BabeApi < Block > > :: current_epoch_start ()
                }),
            ),
            "BabeApi_current_epoch" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let () : () = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "current_epoch" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as sp_consensus_babe :: runtime_decl_for_BabeApi :: BabeApi < Block > > :: current_epoch ()
                }),
            ),
            "BabeApi_next_epoch" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let () : () = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "next_epoch" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as sp_consensus_babe :: runtime_decl_for_BabeApi :: BabeApi < Block > > :: next_epoch ()
                }),
            ),
            "BabeApi_generate_key_ownership_proof" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (_slot , authority_id) : (sp_consensus_babe :: Slot , sp_consensus_babe :: AuthorityId) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "generate_key_ownership_proof" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as sp_consensus_babe :: runtime_decl_for_BabeApi :: BabeApi < Block > > :: generate_key_ownership_proof (_slot , authority_id)
                }),
            ),
            "BabeApi_submit_report_equivocation_unsigned_extrinsic" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (equivocation_proof , key_owner_proof) : (sp_consensus_babe :: EquivocationProof < < Block as BlockT > :: Header > , sp_consensus_babe :: OpaqueKeyOwnershipProof) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "submit_report_equivocation_unsigned_extrinsic" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as sp_consensus_babe :: runtime_decl_for_BabeApi :: BabeApi < Block > > :: submit_report_equivocation_unsigned_extrinsic (equivocation_proof , key_owner_proof)
                }),
            ),
            "SessionKeys_generate_session_keys" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (seed) : (Option < Vec < u8 > >) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "generate_session_keys" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as sp_session :: runtime_decl_for_SessionKeys :: SessionKeys < Block > > :: generate_session_keys (seed)
                }),
            ),
            "SessionKeys_decode_session_keys" => Some(
                self::sp_api_hidden_includes_IMPL_RUNTIME_APIS::sp_api::Encode::encode(&{
                    let (encoded) : (Vec < u8 >) = match self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: DecodeLimit :: decode_all_with_depth_limit (self :: sp_api_hidden_includes_IMPL_RUNTIME_APIS :: sp_api :: MAX_EXTRINSIC_DEPTH , & __sp_api__input_data) { Ok (res) => res , Err (e) => { :: std :: rt :: begin_panic_fmt (& :: core :: fmt :: Arguments :: new_v1 (& ["Bad input data provided to " , ": "] , & match (& "decode_session_keys" , & e) { (arg0 , arg1) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt) , :: core :: fmt :: ArgumentV1 :: new (arg1 , :: core :: fmt :: Display :: fmt)] , })) } } ;
                    # [allow (deprecated)] < Runtime as sp_session :: runtime_decl_for_SessionKeys :: SessionKeys < Block > > :: decode_session_keys (encoded)
                }),
            ),
            _ => None,
        }
    }
}
