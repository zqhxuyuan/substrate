use codec::{Decode, Encode};
#[cfg(feature = "std")]
#[doc(hidden)]
pub use serde;
pub use sp_core::RuntimeDebug;

use sp_core::{crypto::{self, Public}, ecdsa, ecdsa2, ed25519, hash::{H256, H512}, sr25519, H160};
use sp_std::{convert::TryFrom, prelude::*};
use crate::traits::IdentifyAccount;
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
use crate::traits;
use sp_core::crypto::AccountId32;
use sha3::{Keccak256, Digest as ShaDigest};

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