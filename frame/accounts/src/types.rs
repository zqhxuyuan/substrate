use codec::{Codec, Decode, Encode, MaxEncodedLen, EncodeLike};

use sp_runtime::{traits::StaticLookup, DispatchResult, RuntimeDebug};
use sp_std::prelude::*;

use frame_support::{traits::{Get, UnfilteredDispatchable}, weights::GetDispatchInfo, BoundedVec, CloneNoBound, PartialEqNoBound, RuntimeDebugNoBound, WeakBoundedVec};

#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum CoinType {
    BTC,
    ETH,
    VTC
}

#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum AddressType {
    BTC([u8; 35]),
    ETH([u8; 20]),
    VTC([u8; 32])
}

#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub struct Account {
    pub coin_type: CoinType,
    pub raw_name: [u8; 32],
    pub address: AddressType,
    pub parent_account: [u8; 32],
}

#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen, Ord, PartialOrd)]
pub enum PermType {
    Owner,
    Active,
    Others([u8; 5]),
}
#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum Effect {
    Positive,
    Negative,
}
#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen, Ord, PartialOrd)]
pub enum PermModule {
    ALL,
    SELF,
    Module([u8; 5])
}
#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum AccountOrKey {
    Account([u8; 5]),
    Key([u8; 5])
}

#[derive(
CloneNoBound, Encode, Decode, Eq, MaxEncodedLen, PartialEqNoBound, RuntimeDebugNoBound
)]
#[codec(mel_bound(MaxPerm: Get<u32>, MaxAuth: Get<u32>))]
#[cfg_attr(test, derive(frame_support::DefaultNoBound))]
pub struct PermissionAndOwnerData<MaxPerm: Get<u32>, MaxAuth: Get<u32>> {
    // permission name
    pub perm_type: PermType,
    // parent permission name
    pub parent_perm_type: Option<PermType>,
    // permission detail
    pub permissions: BoundedVec<OnePermissionData, MaxPerm>,
    // auth threshold weight
    pub threshold: u32,
    // auth detail
    pub auths: BoundedVec<OneAuthData, MaxAuth>,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen, Copy)]
pub struct OnePermissionData {
    pub effect: Effect,
    // module name: self or other module/contract
    pub module: PermModule,
    // method name: if module is All, the method is None
    // if one module has many method, this will related to multi record
    pub method: Option<[u8; 5]>,
}
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen, Copy)]
pub struct OneAuthData {
    pub account_key: AccountOrKey,
    pub weight: u32
}