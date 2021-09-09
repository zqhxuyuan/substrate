use crate::{Config, Pallet};
use frame_support::dispatch::Dispatchable;

use codec::{Decode, Encode};
use sp_runtime::{traits::{SignedExtension, DispatchInfoOf}, transaction_validity::{TransactionValidity, TransactionValidityError}};
use sp_runtime::transaction_validity::InvalidTransaction;
use frame_support::pallet_prelude::ValidTransaction;

#[derive(Encode, Decode, Clone, Eq, PartialEq)]
pub struct AccountExtension<T: Config + Send + Sync>(
    // T::AccountId
    Option<u32>,
    sp_std::marker::PhantomData<T>
);

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
        Self(
            None, Default::default()
        )
    }
}

impl<T: Config + Send + Sync> SignedExtension for AccountExtension<T> {
    type AccountId = T::AccountId;
    type Call = T::Call;
    type AdditionalSigned = Option<u32>;
    type Pre = ();
    const IDENTIFIER: &'static str = "CheckAccount";

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        Ok(self.0)
    }

    fn validate(
        &self,
        who: &Self::AccountId,
        _call: &Self::Call,
        _info: &DispatchInfoOf<Self::Call>,
        _len: usize,
    ) -> TransactionValidity {
        let account = &self.0;
        log::info!("account from extension:{:?}", account);
        // return InvalidTransaction::Custom(1).into();
        Ok(ValidTransaction {
            ..Default::default()
        })
    }
}
