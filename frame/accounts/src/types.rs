use codec::{Codec, Decode, Encode, MaxEncodedLen, EncodeLike};

use sp_runtime::{traits::StaticLookup, DispatchResult, RuntimeDebug, AccountId32};
use sp_std::prelude::*;

use frame_support::{traits::{Get, UnfilteredDispatchable}, weights::GetDispatchInfo, BoundedVec, CloneNoBound, PartialEqNoBound, RuntimeDebugNoBound, WeakBoundedVec};
use sp_runtime::traits::{Hash, BlakeTwo256};

// The Account frame reference the EOS account model. In VTC, the Account will also encode to [u8; 32].
// for not conflict with other AddressType, the VTC encoded account has prefix "VTC" character
// just like "EOS" character prefix in eos public key.

pub const PREFIX_VTC: [u8; 3] = *b"VTC";

#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum CoinType {
    BTC,
    ETH,
    DOT,
    VTC,
}

#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum AddressType {
    BTC([u8; 35]),
    ETH([u8; 20]),
    DOT([u8; 32]),
    VTC([u8; 32]),
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub struct Account {
    pub coin_type: CoinType,
    pub address: AddressType,
    pub raw_name: [u8; 32],
    pub account_id: AccountId32,
    pub parent_account: AccountId32,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug)]
pub struct VerticAccount {
    pub name: Vec<u8>
}

impl From<VerticAccount> for AccountId32 {
    fn from(x: VerticAccount) -> AccountId32 {
        // let bytes = x.name.as_bytes();
        let bytes = x.name.as_slice();
        let hash = BlakeTwo256::hash_of(&bytes).0;
        let mut vtc_hash = [0u8; 32];
        vtc_hash[0..3].copy_from_slice(&PREFIX_VTC[..]);
        vtc_hash[3..32].copy_from_slice(&hash[3..32]);
        AccountId32::from(vtc_hash)
    }
}



// Permission and auth data of AccountId. the type of AccountId is [u8; 32] used directly from polkadot.
#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen, Ord, PartialOrd)]
pub enum PermType {
    Owner,
    Active,
    // custom permission type
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
// #[cfg_attr(test, derive(frame_support::DefaultNoBound))]
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
    // todo: set method length
    pub method: Option<[u8; 5]>,
}
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen, Copy)]
pub struct OneAuthData {
    pub account_key: AccountOrKey,
    pub weight: u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vertic_to_accountId() {
        let vertic_account = VerticAccount {
            name: "vertic".as_bytes().to_vec()
        };
        // prefix: 565443 == VTC
        // account:56544361ac69da1b70da8b099b05d6399fee2de5e770276995865bc8963ab609 (5E1u1LcV...)
        let accountId: AccountId32 = vertic_account.into();
        println!("account:{:?}", accountId);
    }
}
