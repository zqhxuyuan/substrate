use crate::{Config, Pallet, TotalIssuance, Error, Event, AccountData, Reasons};
use sp_std::prelude::*;
use sp_std::{cmp, result, mem, fmt::Debug, ops::BitOr};
use codec::{Codec, Encode, Decode, MaxEncodedLen};
use frame_support::{ensure, WeakBoundedVec, traits::{
    Currency, tokens::currency::VerticCurrency, OnUnbalanced, TryDrop, StoredMap,
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
                        from_account.free = from_account.free.checked_sub(&value)
                            .ok_or(Error::<T, I>::InsufficientBalance)?;

                        // NOTE: total stake being stored in the same type means that this could never overflow
                        // but better to be safe than sorry.
                        to_account.free = to_account.free.checked_add(&value).ok_or(ArithmeticError::Overflow)?;

                        let ed = T::ExistentialDeposit::get();
                        ensure!(to_account.total() >= ed, Error::<T, I>::ExistentialDeposit);

                        Pallet::<T, I>::ensure_can_withdraw(
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

    /// Handles any steps needed after mutating an account.
    ///
    /// This includes DustRemoval unbalancing, in the case than the `new` account's total balance
    /// is non-zero but below ED.
    ///
    /// Returns two values:
    /// - `Some` containing the the `new` account, iff the account has sufficient balance.
    /// - `Some` containing the dust to be dropped, iff some dust should be dropped.
    fn post_mutation(
        _who: &T::AccountId,
        new: AccountData<T::Balance>,
        // ) -> (Option<AccountData<T::Balance>>, Option<NegativeImbalance<T, I>>) {
    ) -> Option<AccountData<T::Balance>> {
        let total = new.total();
        if total < T::ExistentialDeposit::get() {
            // if total.is_zero() {
            // 	(None, None)
            // } else {
            // 	(None, Some(NegativeImbalance::new(total)))
            // }
            None
        } else {
            // (Some(new), None)
            Some(new)
        }
    }

    // fn deposit_consequence(
    // 	_who: &T::AccountId,
    // 	amount: T::Balance,
    // 	account: &AccountData<T::Balance>,
    // ) -> DepositConsequence {
    // 	if amount.is_zero() { return DepositConsequence::Success }

    // 	if TotalIssuance::<T, I>::get().checked_add(&amount).is_none() {
    // 		return DepositConsequence::Overflow
    // 	}

    // 	let new_total_balance = match account.total().checked_add(&amount) {
    // 		Some(x) => x,
    // 		None => return DepositConsequence::Overflow,
    // 	};

    // 	if new_total_balance < T::ExistentialDeposit::get() {
    // 		return DepositConsequence::BelowMinimum
    // 	}

    // 	// NOTE: We assume that we are a provider, so don't need to do any checks in the
    // 	// case of account creation.

    // 	DepositConsequence::Success
    // }

    // fn withdraw_consequence(
    // 	who: &T::AccountId,
    // 	amount: T::Balance,
    // 	account: &AccountData<T::Balance>,
    // ) -> WithdrawConsequence<T::Balance> {
    // 	if amount.is_zero() { return WithdrawConsequence::Success }

    // 	if TotalIssuance::<T, I>::get().checked_sub(&amount).is_none() {
    // 		return WithdrawConsequence::Underflow
    // 	}

    // 	let new_total_balance = match account.total().checked_sub(&amount) {
    // 		Some(x) => x,
    // 		None => return WithdrawConsequence::NoFunds,
    // 	};

    // 	// Provider restriction - total account balance cannot be reduced to zero if it cannot
    // 	// sustain the loss of a provider reference.
    // 	// NOTE: This assumes that the pallet is a provider (which is true). Is this ever changes,
    // 	// then this will need to adapt accordingly.
    // 	let ed = T::ExistentialDeposit::get();
    // 	let success = if new_total_balance < ed {
    // 		if frame_system::Pallet::<T>::can_dec_provider(who) {
    // 			WithdrawConsequence::ReducedToZero(new_total_balance)
    // 		} else {
    // 			return WithdrawConsequence::WouldDie
    // 		}
    // 	} else {
    // 		WithdrawConsequence::Success
    // 	};

    // 	// Enough free funds to have them be reduced.
    // 	let new_free_balance = match account.free.checked_sub(&amount) {
    // 		Some(b) => b,
    // 		None => return WithdrawConsequence::NoFunds,
    // 	};

    // 	// Eventual free funds must be no less than the frozen balance.
    // 	let min_balance = account.frozen(Reasons::All);
    // 	if new_free_balance < min_balance {
    // 		return WithdrawConsequence::Frozen
    // 	}

    // 	success
    // }

    /// Mutate an account to some new value, or delete it entirely with `None`. Will enforce
    /// `ExistentialDeposit` law, annulling the account as needed.
    ///
    /// NOTE: Doesn't do any preparatory work for creating a new account, so should only be used
    /// when it is known that the account already exists.
    ///
    /// NOTE: LOW-LEVEL: This will not attempt to maintain total issuance. It is expected that
    /// the caller will do this.
    pub fn mutate_account<R>(
        who: &T::AccountId,
        f: impl FnOnce(&mut AccountData<T::Balance>) -> R,
    ) -> Result<R, StoredMapError> {
        Self::try_mutate_account(who, |a, _| -> Result<R, StoredMapError> { Ok(f(a)) })
    }

    /// Mutate an account to some new value, or delete it entirely with `None`. Will enforce
    /// `ExistentialDeposit` law, annulling the account as needed. This will do nothing if the
    /// result of `f` is an `Err`.
    ///
    /// NOTE: Doesn't do any preparatory work for creating a new account, so should only be used
    /// when it is known that the account already exists.
    ///
    /// NOTE: LOW-LEVEL: This will not attempt to maintain total issuance. It is expected that
    /// the caller will do this.
    pub fn try_mutate_account<R, E: From<StoredMapError>>(
        who: &T::AccountId,
        f: impl FnOnce(&mut AccountData<T::Balance>, bool) -> Result<R, E>,
    ) -> Result<R, E> {
        // Self::try_mutate_account_with_dust(who, f)
        // 	.map(|(result, dust_cleaner)| {
        // 		drop(dust_cleaner);
        // 		result
        // 	})

        let result = T::AccountStore::try_mutate_exists(who, |maybe_account| {
            let is_new = maybe_account.is_none();
            let mut account = maybe_account.take().unwrap_or_default();
            f(&mut account, is_new).map(move |result| {
                let maybe_endowed = if is_new { Some(account.free) } else { None };
                let maybe_account_maybe_dust = Self::post_mutation(who, account);
                *maybe_account = maybe_account_maybe_dust;
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

    // fn try_mutate_account_with_dust<R, E: From<StoredMapError>>(
    // 	who: &T::AccountId,
    // 	f: impl FnOnce(&mut AccountData<T::Balance>, bool) -> Result<R, E>,
    // ) -> Result<(R, DustCleaner<T, I>), E> {
    // 	let result = T::AccountStore::try_mutate_exists(who, |maybe_account| {
    // 		let is_new = maybe_account.is_none();
    // 		let mut account = maybe_account.take().unwrap_or_default();
    // 		f(&mut account, is_new).map(move |result| {
    // 			let maybe_endowed = if is_new { Some(account.free) } else { None };
    // 			let maybe_account_maybe_dust = Self::post_mutation(who, account);
    // 			*maybe_account = maybe_account_maybe_dust.0;
    // 			(maybe_endowed, maybe_account_maybe_dust.1, result)
    // 		})
    // 	});
    // 	result.map(|(maybe_endowed, maybe_dust, result)| {
    // 		if let Some(endowed) = maybe_endowed {
    // 			Self::deposit_event(Event::Endowed(who.clone(), endowed));
    // 		}
    // 		let dust_cleaner = DustCleaner(maybe_dust.map(|dust| (who.clone(), dust)));
    // 		(result, dust_cleaner)
    // 	})
    // }

    // /// Update the account entry for `who`, given the locks.
    // fn update_locks(who: &T::AccountId, locks: &[BalanceLock<T::Balance>]) {
    // 	let bounded_locks = WeakBoundedVec::<_, T::MaxLocks>::force_from(
    // 		locks.to_vec(),
    // 		Some("Balances Update Locks"),
    // 	);

    // 	if locks.len() as u32 > T::MaxLocks::get() {
    // 		log::warn!(
    // 			target: "runtime::balances",
    // 			"Warning: A user has more currency locks than expected. \
    // 			A runtime configuration adjustment may be needed."
    // 		);
    // 	}
    // 	// No way this can fail since we do not alter the existential balances.
    // 	let res = Self::mutate_account(who, |b| {
    // 		b.misc_frozen = Zero::zero();
    // 		b.fee_frozen = Zero::zero();
    // 		for l in locks.iter() {
    // 			if l.reasons == Reasons::All || l.reasons == Reasons::Misc {
    // 				b.misc_frozen = b.misc_frozen.max(l.amount);
    // 			}
    // 			if l.reasons == Reasons::All || l.reasons == Reasons::Fee {
    // 				b.fee_frozen = b.fee_frozen.max(l.amount);
    // 			}
    // 		}
    // 	});
    // 	debug_assert!(res.is_ok());

    // 	let _: Result<(), Error<T, I>> = VerticAccount::<T, I>::try_mutate_exists(who, |account| {
    // 		if account.is_none() {
    // 			*account = Some(VerticAccountData{
    // 				free: Zero::zero(),
    // 				reserved: Zero::zero(),
    // 				misc_frozen: Zero::zero(),
    // 				fee_frozen: Zero::zero(),
    // 				locks: bounded_locks.clone()
    // 			});
    // 		} else {
    // 			let old = account.take().unwrap();
    // 			*account = Some(VerticAccountData{
    // 				free: old.free,
    // 				reserved: old.reserved,
    // 				misc_frozen: old.misc_frozen,
    // 				fee_frozen: old.fee_frozen,
    // 				locks: bounded_locks.clone()
    // 			});
    // 		}
    // 		Ok(())
    // 	});

    // 	let existed = Locks::<T, I>::contains_key(who);
    // 	if locks.is_empty() {
    // 		Locks::<T, I>::remove(who);
    // 		if existed {
    // 			// TODO: use Locks::<T, I>::hashed_key
    // 			// https://github.com/paritytech/substrate/issues/4969
    // 			system::Pallet::<T>::dec_consumers(who);
    // 		}
    // 	} else {
    // 		Locks::<T, I>::insert(who, bounded_locks);
    // 		if !existed {
    // 			if system::Pallet::<T>::inc_consumers(who).is_err() {
    // 				// No providers for the locks. This is impossible under normal circumstances
    // 				// since the funds that are under the lock will themselves be stored in the
    // 				// account and therefore will need a reference.
    // 				log::warn!(
    // 					target: "runtime::balances",
    // 					"Warning: Attempt to introduce lock consumer reference, yet no providers. \
    // 					This is unexpected but should be safe."
    // 				);
    // 			}
    // 		}
    // 	}
    // }

    // fn do_transfer_reserved(
    // 	slashed: &T::AccountId,
    // 	beneficiary: &T::AccountId,
    // 	value: T::Balance,
    // 	best_effort: bool,
    // 	status: Status,
    // ) -> Result<T::Balance, DispatchError> {
    // 	if value.is_zero() { return Ok(Zero::zero()) }

    // 	if slashed == beneficiary {
    // 		return match status {
    // 			Status::Free => Ok(Self::unreserve(slashed, value)),
    // 			Status::Reserved => Ok(value.saturating_sub(Self::reserved_balance(slashed))),
    // 		};
    // 	}

    // 	let ((actual, _maybe_one_dust), _maybe_other_dust) = Self::try_mutate_account_with_dust(
    // 		beneficiary,
    // 		|to_account, is_new| -> Result<(T::Balance, DustCleaner<T, I>), DispatchError> {
    // 			ensure!(!is_new, Error::<T, I>::DeadAccount);
    // 			Self::try_mutate_account_with_dust(
    // 				slashed,
    // 				|from_account, _| -> Result<T::Balance, DispatchError> {
    // 					let actual = cmp::min(from_account.reserved, value);
    // 					ensure!(best_effort || actual == value, Error::<T, I>::InsufficientBalance);
    // 					match status {
    // 						Status::Free => to_account.free = to_account.free
    // 							.checked_add(&actual)
    // 							.ok_or(ArithmeticError::Overflow)?,
    // 						Status::Reserved => to_account.reserved = to_account.reserved
    // 							.checked_add(&actual)
    // 							.ok_or(ArithmeticError::Overflow)?,
    // 					}
    // 					from_account.reserved -= actual;
    // 					Ok(actual)
    // 				}
    // 			)
    // 		}
    // 	)?;

    // 	Self::deposit_event(Event::ReserveRepatriated(slashed.clone(), beneficiary.clone(), actual, status));
    // 	Ok(actual)
    // }
}