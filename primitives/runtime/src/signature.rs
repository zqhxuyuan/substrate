use codec::{Decode, Encode};
#[cfg(feature = "std")]
#[doc(hidden)]
pub use serde;
pub use sp_core::RuntimeDebug;

use sp_core::{crypto::{self, Public}, ecdsa, ecdsa2, ed25519, hash::{H256, H512}, sr25519, H160};
use sp_std::{convert::TryFrom, prelude::*};
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
use sha3::{Keccak256, Digest as ShaDigest};

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