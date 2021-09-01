use crate::{Config, Pallet};
use frame_support::dispatch::Dispatchable;

use codec::{Decode, Encode};
use sp_runtime::{traits::{SignedExtension, DispatchInfoOf}, transaction_validity::{TransactionValidity, TransactionValidityError}};
use sp_runtime::transaction_validity::InvalidTransaction;
use frame_support::pallet_prelude::ValidTransaction;

#[derive(Encode, Decode, Clone, Eq, PartialEq)]
pub struct AccountExtension<T: Config + Send + Sync>(sp_std::marker::PhantomData<T>);

impl<T: Config + Send + Sync> sp_std::fmt::Debug for AccountExtension<T> {
    #[cfg(feature = "std")]
    fn fmt(&self, f: &mut sp_std::fmt::Formatter) -> sp_std::fmt::Result {
        write!(f, "CheckAccount")
    }

    #[cfg(not(feature = "std"))]
    fn fmt(&self, _: &mut sp_std::fmt::Formatter) -> sp_std::fmt::Result {
        Ok(())
    }
}

impl<T: Config + Send + Sync> AccountExtension<T> {
    /// Create new `SignedExtension` to check runtime version.
    pub fn new() -> Self {
        Self(sp_std::marker::PhantomData)
    }
}

impl<T: Config + Send + Sync> SignedExtension for AccountExtension<T> {
    type AccountId = T::AccountId;
    type Call = T::Call;
    type AdditionalSigned = ();
    type Pre = ();
    const IDENTIFIER: &'static str = "CheckAccount";

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        Ok(())
    }

    fn validate(
        &self,
        who: &Self::AccountId,
        _call: &Self::Call,
        _info: &DispatchInfoOf<Self::Call>,
        _len: usize,
    ) -> TransactionValidity {
        // return InvalidTransaction::Stale.into();
        Ok(ValidTransaction {
            ..Default::default()
        })
    }
}
