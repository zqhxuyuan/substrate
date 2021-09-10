use crate::{Config, Pallet};
use frame_support::dispatch::Dispatchable;

use codec::{Decode, Encode};
use sp_runtime::{traits::{SignedExtension, DispatchInfoOf}, transaction_validity::{TransactionValidity, TransactionValidityError}, AccountId32};
use sp_runtime::transaction_validity::InvalidTransaction;
use frame_support::pallet_prelude::ValidTransaction;
use frame_support::traits::EnsureOrigin;
use frame_system::RawOrigin;

#[derive(Encode, Decode, Clone, Eq, PartialEq)]
pub struct AccountExtension<T: Config + Send + Sync>(
    // T::AccountId,
    [u8; 32],
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
            [0u8; 32], Default::default()
        )
    }
}

impl<T: Config + Send + Sync> SignedExtension for AccountExtension<T> {
    type AccountId = T::AccountId;
    type Call = T::Call;
    type AdditionalSigned = [u8; 32];
    type Pre = ();
    const IDENTIFIER: &'static str = "CheckAccount";

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        Ok(self.0)
    }

    fn validate(
        &self,
        who: &Self::AccountId, // signer of transaction
        call: &Self::Call, // function in extrinsic: including CallIndex(module+function) and parameters
        _info: &DispatchInfoOf<Self::Call>,
        _len: usize,
    ) -> TransactionValidity {
        let account = &self.0;
        log::info!("account from extension:{:?}", account);
        log::info!("call info:{:?}", call);
        let account_id: AccountId32 = (*account).into();
        log::info!("account32:{:?}", account_id);

        // 1. find the public key belongs to which permission map(owner,active,custom)
        let owner_accounts = crate::OwnerAccountIdMap::<T>::get(who);
        let active_accounts = crate::ActiveAccountIdMap::<T>::get(who);

        // todo: T::AccountId can't just cast to AccountId32 here

        // return InvalidTransaction::Custom(1).into();
        Ok(ValidTransaction {
            ..Default::default()
        })
    }
}

pub struct EnsurePermission<T: Config>(
    // pub T::AccountId,
    sp_std::marker::PhantomData<T>
);

impl<T: Config> EnsureOrigin<T::Origin> for EnsurePermission<T> {
    type Success = T::AccountId;
    fn try_origin(o: T::Origin) -> Result<Self::Success, T::Origin> {

        o.into().and_then(|o| {
            log::info!("raw origin:{:?}", o);
            match o {
                RawOrigin::Signed2(ref who, ref account) => {
                    // let owner_accounts = crate::OwnerAccountIdMap::<T>::get(who);
                    // let active_accounts = crate::ActiveAccountIdMap::<T>::get(who);
                    log::info!("signed_by:{:?}", who);
                    log::info!("accountid:{:?}", account);

                    Ok(who.clone())
                },
                r => Err(T::Origin::from(r)),
            }
        })
    }

    #[cfg(feature = "runtime-benchmarks")]
    fn successful_origin() -> T::Origin {
        T::Origin::from(RawOrigin::Signed(Default::default()))
    }
}