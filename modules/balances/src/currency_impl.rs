#![cfg_attr(not(feature = "std"), no_std)]

use crate::{Config, Pallet, TotalIssuance, Error, Event, AccountData, Reasons};
use sp_std::prelude::*;
use sp_std::{cmp, result, mem, fmt::Debug, ops::BitOr};
use codec::{Codec, Encode, Decode, MaxEncodedLen};
use frame_support::{
    // debug,
    ensure, WeakBoundedVec, traits::{
    Currency, OnUnbalanced, TryDrop, StoredMap,
    WithdrawReasons, LockIdentifier, LockableCurrency, ExistenceRequirement,
    Imbalance, SignedImbalance, ReservableCurrency, Get, ExistenceRequirement::{AllowDeath, KeepAlive},
    NamedReservableCurrency,
    tokens::{fungible, DepositConsequence, WithdrawConsequence, BalanceStatus as Status},
}, BoundedVec};
#[cfg(feature = "std")]
use frame_support::traits::GenesisBuild;
use sp_runtime::{
    RuntimeDebug, DispatchResult, DispatchError, ArithmeticError,
    traits::{
        Zero, AtLeast32BitUnsigned, StaticLookup, CheckedAdd, CheckedSub,
        MaybeSerializeDeserialize, Saturating, Bounded, StoredMapError,
    },
};
use frame_system as system;
use sp_std::marker::PhantomData;
use module_common::currency::VerticCurrency;
use sp_runtime::print;
use sp_std::convert::TryInto;

pub struct PalletImpl<T: Config<I>, I: 'static = ()>(PhantomData<(T, I)>);

impl<T: Config<I>, I: 'static> VerticCurrency<T::AccountId> for PalletImpl<T, I> where
    T::Balance: MaybeSerializeDeserialize + Debug
{
    type Balance = T::Balance;

    fn total_balance(who: &T::AccountId) -> Self::Balance {
        Pallet::<T, I>::account(who).total()
    }

    fn total_issuance() -> Self::Balance {
        <TotalIssuance<T, I>>::get()
    }

    fn minimum_balance() -> Self::Balance {
        T::ExistentialDeposit::get()
    }

    fn free_balance(who: &T::AccountId) -> Self::Balance {
        Pallet::<T, I>::account(who).free
    }

    fn ensure_can_withdraw(
        who: &T::AccountId,
        amount: T::Balance,
        reasons: WithdrawReasons,
        new_balance: T::Balance,
    ) -> DispatchResult {
        if amount.is_zero() { return Ok(()) }
        let min_balance = Pallet::<T, I>::account(who).frozen(reasons.into());
        ensure!(new_balance >= min_balance, Error::<T, I>::LiquidityRestrictions);
        Ok(())
    }

    fn transfer(
        transactor: &T::AccountId,
        dest: &T::AccountId,
        value: Self::Balance,
        existence_requirement: ExistenceRequirement,
    ) -> DispatchResult {
        if value.is_zero() || transactor == dest { return Ok(()) }

        Pallet::<T, I>::try_mutate_account(
            dest,
            |to_account, _| -> DispatchResult {
                Pallet::<T, I>::try_mutate_account(
                    transactor,
                    |from_account, _| -> DispatchResult {
                        let ed = T::ExistentialDeposit::get();

                        from_account.free = from_account.free.checked_sub(&value)
                            .ok_or(Error::<T, I>::InsufficientBalance)?;
                        let from_total = from_account.total();
                        log::info!("pallet from:{:?}, ed:{:?}", from_total, ed);

                        ensure!(from_account.total() >= ed, Error::<T, I>::ExistentialDeposit);

                        // NOTE: total stake being stored in the same type means that this could never overflow
                        // but better to be safe than sorry.
                        to_account.free = to_account.free.checked_add(&value).ok_or(ArithmeticError::Overflow)?;
                        ensure!(to_account.total() >= ed, Error::<T, I>::ExistentialDeposit);

                        Self::ensure_can_withdraw(
                            transactor,
                            value,
                            WithdrawReasons::TRANSFER,
                            from_account.free,
                        ).map_err(|_| Error::<T, I>::LiquidityRestrictions)?;

                        let allow_death = existence_requirement == ExistenceRequirement::AllowDeath;
                        let allow_death = allow_death && !system::Pallet::<T>::is_provider_required(transactor);
                        ensure!(allow_death || from_account.total() >= ed, Error::<T, I>::KeepAlive);

                        Ok(())
                    }
                )
            }
        )?;

        // Emit transfer event.
        Pallet::<T, I>::deposit_event(Event::Transfer(transactor.clone(), dest.clone(), value));

        Ok(())
    }
}

impl<T: Config<I>, I: 'static> Pallet<T, I> {
    /// Get the free balance of an account.
    pub fn free_balance(who: impl sp_std::borrow::Borrow<T::AccountId>) -> T::Balance {
        Self::account(who.borrow()).free
    }

    /// Get the balance of an account that can be used for transfers, reservations, or any other
    /// non-locking, non-transaction-fee activity. Will be at most `free_balance`.
    pub fn usable_balance(who: impl sp_std::borrow::Borrow<T::AccountId>) -> T::Balance {
        Self::account(who.borrow()).usable(Reasons::Misc)
    }

    /// Get the balance of an account that can be used for paying transaction fees (not tipping,
    /// or any other kind of fees, though). Will be at most `free_balance`.
    pub fn usable_balance_for_fees(who: impl sp_std::borrow::Borrow<T::AccountId>) -> T::Balance {
        Self::account(who.borrow()).usable(Reasons::Fee)
    }

    /// Get the reserved balance of an account.
    pub fn reserved_balance(who: impl sp_std::borrow::Borrow<T::AccountId>) -> T::Balance {
        Self::account(who.borrow()).reserved
    }

    /// Get both the free and reserved balances of an account.
    pub fn account(who: &T::AccountId) -> AccountData<T::Balance> {
        T::AccountStore::get(&who)
    }

    fn post_mutation(
        _who: &T::AccountId,
        new: AccountData<T::Balance>,
    ) -> Option<AccountData<T::Balance>> {
        let total = new.total();
        if total < T::ExistentialDeposit::get() {
            None
        } else {
            Some(new)
        }
    }

    pub fn mutate_account<R>(
        who: &T::AccountId,
        f: impl FnOnce(&mut AccountData<T::Balance>) -> R,
    ) -> Result<R, StoredMapError> {
        Self::try_mutate_account(who, |a, _| -> Result<R, StoredMapError> { Ok(f(a)) })
    }

    pub fn try_mutate_account<R, E: From<StoredMapError>>(
        who: &T::AccountId,
        f: impl FnOnce(&mut AccountData<T::Balance>, bool) -> Result<R, E>,
    ) -> Result<R, E> {
        let result = T::AccountStore::try_mutate_exists(who, |maybe_account| {
            let is_new = maybe_account.is_none();
            let mut account = maybe_account.take().unwrap_or_default();
            f(&mut account, is_new).map(move |result| {
                let maybe_endowed = if is_new { Some(account.free) } else { None };
                // let maybe_account_maybe_dust = Self::post_mutation(who, account);
                let total = account.total();
                let maybe_account1 = if total < T::ExistentialDeposit::get() {
                    None
                } else {
                    Some(account)
                };
                *maybe_account = maybe_account1;
                (maybe_endowed, result)
            })
        });
        result.map(|(maybe_endowed, result)| {
            if let Some(endowed) = maybe_endowed {
                Self::deposit_event(Event::Endowed(who.clone(), endowed));
            }
            result
        })
    }
}