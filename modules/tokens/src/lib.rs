#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::unused_unit)]

// pub use crate::imbalances::{NegativeImbalance, PositiveImbalance};
use codec::{Codec, Encode, Decode, MaxEncodedLen};

use frame_support::{
	ensure, log,
	pallet_prelude::*,
	traits::{
		tokens::{fungible, fungibles, DepositConsequence, WithdrawConsequence},
		BalanceStatus as Status, Currency as PalletCurrency, ExistenceRequirement, Get, Imbalance,
		LockableCurrency as PalletLockableCurrency, ReservableCurrency as PalletReservableCurrency,
		SignedImbalance, WithdrawReasons,
	},
	transactional, BoundedVec,
};
use frame_system::{ensure_signed, pallet_prelude::*};
use orml_traits::{
	arithmetic::{self, Signed},
	currency::TransferAll,
	BalanceStatus, GetByKey, LockIdentifier, BasicCurrency, BasicCurrencyExtended, BasicLockableCurrency,
	BasicReservableCurrency, OnDust,
};
use sp_runtime::{
	traits::{
		AtLeast32BitUnsigned, Bounded, CheckedAdd, CheckedSub, MaybeSerializeDeserialize, Member, Saturating,
		StaticLookup, Zero,
	},
	ArithmeticError, DispatchError, DispatchResult, RuntimeDebug,
};
use sp_std::{
	convert::{Infallible, TryFrom, TryInto},
	marker,
	prelude::*,
	vec::Vec,
};

// mod imbalances;
// mod mock;
// mod tests;
mod weights;
mod model;
// mod dust;

pub use weights::WeightInfo;

pub use module::*;
pub use crate::model::*;

#[frame_support::pallet]
pub mod module {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		/// The balance type
		type Balance: Parameter
			+ Member
			+ AtLeast32BitUnsigned
			+ Default
			+ Copy
			+ MaybeSerializeDeserialize
			+ MaxEncodedLen;

		/// The amount type, should be signed version of `Balance`
		type Amount: Signed
			+ TryInto<Self::Balance>
			+ TryFrom<Self::Balance>
			+ Parameter
			+ Member
			+ arithmetic::SimpleArithmetic
			+ Default
			+ Copy
			+ MaybeSerializeDeserialize;

		/// Weight information for extrinsics in this module.
		type WeightInfo: WeightInfo;

		/// The minimum amount required to keep an account.
		type ExistentialDeposits: Get<Self::Balance>;

		/// Handler to burn or transfer account's dust
		// type OnDust: OnDust<Self::AccountId, Self::CurrencyId, Self::Balance>;

		type MaxLocks: Get<u32>;
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The balance is too low
		BalanceTooLow,
		/// Cannot convert Amount into Balance type
		AmountIntoBalanceFailed,
		/// Failed because liquidity restrictions due to locking
		LiquidityRestrictions,
		/// Failed because the maximum locks was exceeded
		MaxLocksExceeded,
		/// Transfer/payment would kill account
		KeepAlive,
		/// Value too low to create account due to existential deposit
		ExistentialDeposit,
		/// Beneficiary account must pre-exist
		DeadAccount,
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(crate) fn deposit_event)]
	#[pallet::metadata(T::AccountId = "AccountId", T::Balance = "Balance")]
	pub enum Event<T: Config> {
		/// An account was created with some free balance. \[account, free_balance\]
		Endowed(T::AccountId, T::Balance),
		/// An account was removed whose balance was non-zero but below
		/// ExistentialDeposit, resulting in an outright loss. \[account, balance\]
		DustLost(T::AccountId, T::Balance),
		/// Transfer succeeded. \[ from, to, value\]
		Transfer(T::AccountId, T::AccountId, T::Balance),
		/// Some balance was reserved (moved from free to reserved). \[ who, value\]
		Reserved(T::AccountId, T::Balance),
		/// Some balance was unreserved (moved from reserved to free). \[ who, value\]
		Unreserved(T::AccountId, T::Balance),
	}

	/// The total issuance of a token type.
	#[pallet::storage]
	#[pallet::getter(fn total_issuance)]
	pub type TotalIssuance<T: Config> = StorageValue<_, T::Balance, ValueQuery>;

	/// The balance of a token type under an account.
	#[pallet::storage]
	#[pallet::getter(fn accounts)]
	pub type Accounts<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		AccountData<T::Balance>,
		ValueQuery,
	>;

	/// Any liquidity locks of a token type under an account.
	/// NOTE: Should only be accessed when setting, changing and freeing a lock.
	#[pallet::storage]
	#[pallet::getter(fn locks)]
	pub type Locks<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		BoundedVec<BalanceLock<T::Balance>, T::MaxLocks>,
		ValueQuery,
	>;

	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		pub balances: Vec<(T::AccountId, T::Balance)>,
	}

	#[cfg(feature = "std")]
	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			GenesisConfig { balances: vec![] }
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig<T> {
		fn build(&self) {
			let total = self.balances
				.iter()
				.fold(Zero::zero(), |acc: T::Balance, &(_, n)| acc + n);
			<TotalIssuance<T>>::put(total);
		}
	}

	#[pallet::pallet]
	pub struct Pallet<T>(PhantomData<(T)>);

	#[pallet::hooks]
	impl<T: Config> Hooks<T::BlockNumber> for Pallet<T> {}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Transfer some balance to another account.
		#[pallet::weight(T::WeightInfo::transfer())]
		pub fn transfer(
			origin: OriginFor<T>,
			dest: <T::Lookup as StaticLookup>::Source,
			#[pallet::compact] amount: T::Balance,
		) -> DispatchResult {
			let from = ensure_signed(origin)?;
			let to = T::Lookup::lookup(dest)?;
			<Self as BasicCurrency<_>>::transfer(&from, &to, amount)?;

			Self::deposit_event(Event::Transfer(from, to, amount));
			Ok(())
		}

		/// Transfer all remaining balance to the given account.
		#[pallet::weight(T::WeightInfo::transfer_all())]
		pub fn transfer_all(
			origin: OriginFor<T>,
			dest: <T::Lookup as StaticLookup>::Source,
		) -> DispatchResult {
			let from = ensure_signed(origin)?;
			let to = T::Lookup::lookup(dest)?;
			// todo: use reducible_balance from fungible::Inspect trait
			let balance = <Self as BasicCurrency<T::AccountId>>::free_balance(&from);
			<Self as BasicCurrency<T::AccountId>>::transfer(&from, &to, balance)?;

			Self::deposit_event(Event::Transfer(from, to, balance));
			Ok(())
		}
	}
}

impl<T: Config> Pallet<T> {
	pub(crate) fn deposit_consequence(
		_who: &T::AccountId,
		amount: T::Balance,
		account: &AccountData<T::Balance>,
	) -> DepositConsequence {
		if amount.is_zero() {
			return DepositConsequence::Success;
		}

		if TotalIssuance::<T>::get().checked_add(&amount).is_none() {
			return DepositConsequence::Overflow;
		}

		let new_total_balance = match account.total().checked_add(&amount) {
			Some(x) => x,
			None => return DepositConsequence::Overflow,
		};

		if new_total_balance < T::ExistentialDeposits::get() {
			return DepositConsequence::BelowMinimum;
		}

		// NOTE: We assume that we are a provider, so don't need to do any checks in the
		// case of account creation.

		DepositConsequence::Success
	}

	pub(crate) fn withdraw_consequence(
		who: &T::AccountId,
		amount: T::Balance,
		account: &AccountData<T::Balance>,
	) -> WithdrawConsequence<T::Balance> {
		if amount.is_zero() {
			return WithdrawConsequence::Success;
		}

		if TotalIssuance::<T>::get().checked_sub(&amount).is_none() {
			return WithdrawConsequence::Underflow;
		}

		let new_total_balance = match account.total().checked_sub(&amount) {
			Some(x) => x,
			None => return WithdrawConsequence::NoFunds,
		};

		let ed = T::ExistentialDeposits::get();
		let success = if new_total_balance < ed {
			if frame_system::Pallet::<T>::can_dec_provider(who) {
				WithdrawConsequence::ReducedToZero(new_total_balance)
			} else {
				return WithdrawConsequence::WouldDie;
			}
		} else {
			WithdrawConsequence::Success
		};

		// Enough free funds to have them be reduced.
		let new_free_balance = match account.free.checked_sub(&amount) {
			Some(b) => b,
			None => return WithdrawConsequence::NoFunds,
		};

		// Eventual free funds must be no less than the frozen balance.
		if new_free_balance < account.frozen() {
			return WithdrawConsequence::Frozen;
		}

		success
	}

	pub(crate) fn try_mutate_account<R, E>(
		who: &T::AccountId,
		f: impl FnOnce(&mut AccountData<T::Balance>, bool) -> sp_std::result::Result<R, E>,
	) -> sp_std::result::Result<R, E> {
		Accounts::<T>::try_mutate_exists(who, |maybe_account| {
			let existed = maybe_account.is_some();
			let mut account = maybe_account.take().unwrap_or_default();
			f(&mut account, existed).map(move |result| {
				let maybe_endowed = if !existed { Some(account.free) } else { None };
				let mut maybe_dust: Option<T::Balance> = None;
				let total = account.total();
				*maybe_account = if total.is_zero() {
					None
				} else {
					// if non_zero total is below existential deposit, should handle the dust.
					if total < T::ExistentialDeposits::get() {
						maybe_dust = Some(total);
					}
					Some(account)
				};

				(maybe_endowed, existed, maybe_account.is_some(), maybe_dust, result)
			})
		})
		.map(|(maybe_endowed, existed, exists, maybe_dust, result)| {
			if existed && !exists {
				let _ = frame_system::Pallet::<T>::dec_providers(who);
			} else if !existed && exists {
				// if new, increase account provider
				frame_system::Pallet::<T>::inc_providers(who);
			}

			if let Some(endowed) = maybe_endowed {
				Self::deposit_event(Event::Endowed(who.clone(), endowed));
			}
			// todo: deal with maybe_dust?
			result
		})
	}

	pub(crate) fn mutate_account<R>(
		who: &T::AccountId,
		f: impl FnOnce(&mut AccountData<T::Balance>, bool) -> R,
	) -> R {
		Self::try_mutate_account(who, |account, existed| -> Result<R, Infallible> {
			Ok(f(account, existed))
		})
		.expect("Error is infallible; qed")
	}

	/// Set free balance of `who` to a new value.
	///
	/// Note this will not maintain total issuance, and the caller is expected to do it.
	pub(crate) fn set_free_balance( who: &T::AccountId, amount: T::Balance) {
		Self::mutate_account(who, |account, _| {
			account.free = amount;
		});
	}

	/// Set reserved balance of `who` to a new value.
	///
	/// Note this will not maintain total issuance, and the caller is expected to do it.
	pub(crate) fn set_reserved_balance(who: &T::AccountId, amount: T::Balance) {
		Self::mutate_account(who, |account, _| {
			account.reserved = amount;
		});
	}

	/// Update the account entry for `who` under `currency_id`, given the locks.
	pub(crate) fn update_locks(
		who: &T::AccountId,
		locks: &[BalanceLock<T::Balance>],
	) -> DispatchResult {
		// update account data, use the maximum as newly frozen amount
		Self::mutate_account(who, |account, _| {
			account.frozen = Zero::zero();
			for lock in locks.iter() {
				account.frozen = account.frozen.max(lock.amount);
			}
		});

		// update locks
		let existed = <Locks<T>>::contains_key(who);
		if locks.is_empty() {
			<Locks<T>>::remove(who);
			if existed {
				// decrease account ref count when destruct lock
				frame_system::Pallet::<T>::dec_consumers(who);
			}
		} else {
			let bounded_locks: BoundedVec<BalanceLock<T::Balance>, T::MaxLocks> =
				locks.to_vec().try_into().map_err(|_| Error::<T>::MaxLocksExceeded)?;
			<Locks<T>>::insert(who, bounded_locks);
			if !existed {
				// increase account ref count when initialize lock
				if frame_system::Pallet::<T>::inc_consumers(who).is_err() {
					log::warn!(
						"Warning: Attempt to introduce lock consumer reference, yet no providers. \
						This is unexpected but should be safe."
					);
				}
			}
		}

		Ok(())
	}

	/// Transfer some free balance from `from` to `to`.
	/// Ensure from_account allow death or new balance above existential
	/// deposit. Ensure to_account new balance above existential deposit.
	pub(crate) fn do_transfer(
		from: &T::AccountId,
		to: &T::AccountId,
		amount: T::Balance,
		existence_requirement: ExistenceRequirement,
	) -> DispatchResult {
		if amount.is_zero() || from == to {
			return Ok(());
		}

		Self::try_mutate_account(to, |to_account, _existed| -> DispatchResult {
			Self::try_mutate_account(from, |from_account, _existed| -> DispatchResult {
				from_account.free = from_account
					.free
					.checked_sub(&amount)
					.ok_or(Error::<T>::BalanceTooLow)?;
				to_account.free = to_account.free.checked_add(&amount).ok_or(ArithmeticError::Overflow)?;

				let ed = T::ExistentialDeposits::get();
				ensure!(to_account.total() >= ed, Error::<T>::ExistentialDeposit);

				Self::ensure_can_withdraw(from, amount)?;

				let allow_death = existence_requirement == ExistenceRequirement::AllowDeath;
				let allow_death = allow_death && frame_system::Pallet::<T>::can_dec_provider(from);
				ensure!(allow_death || from_account.total() >= ed, Error::<T>::KeepAlive);
				Ok(())
			})?;
			Ok(())
		})
	}
}

impl<T: Config> BasicCurrency<T::AccountId> for Pallet<T> {
	type Balance = T::Balance;

	fn minimum_balance() -> Self::Balance {
		T::ExistentialDeposits::get()
	}

	fn total_issuance() -> Self::Balance {
		<TotalIssuance<T>>::get()
	}

	fn total_balance(who: &T::AccountId) -> Self::Balance {
		Self::accounts(who).total()
	}

	fn free_balance(who: &T::AccountId) -> Self::Balance {
		Self::accounts(who).free
	}

	// Ensure that an account can withdraw from their free balance given any
	// existing withdrawal restrictions like locks and vesting balance.
	// Is a no-op if amount to be withdrawn is zero.
	fn ensure_can_withdraw(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
		if amount.is_zero() {
			return Ok(());
		}

		let new_balance = Self::free_balance(who)
			.checked_sub(&amount)
			.ok_or(Error::<T>::BalanceTooLow)?;
		ensure!(
			new_balance >= Self::accounts(who).frozen(),
			Error::<T>::LiquidityRestrictions
		);
		Ok(())
	}

	/// Transfer some free balance from `from` to `to`.
	fn transfer(
		from: &T::AccountId,
		to: &T::AccountId,
		amount: Self::Balance,
	) -> DispatchResult {
		Self::do_transfer(from, to, amount, ExistenceRequirement::AllowDeath)
	}

	/// Deposit some `amount` into the free balance of account `who`.
	/// different with pallet_balance which has: deposit_into_existing and deposit_creating
	fn deposit(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
		if amount.is_zero() {
			return Ok(());
		}

		TotalIssuance::<T>::try_mutate(|total_issuance| -> DispatchResult {
			// mutate the total balance
			*total_issuance = total_issuance.checked_add(&amount).ok_or(ArithmeticError::Overflow)?;

			// mutate the account's balance
			Self::set_free_balance(who, Self::free_balance(who) + amount);

			Ok(())
		})
	}

	fn withdraw(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
		if amount.is_zero() {
			return Ok(());
		}
		Self::ensure_can_withdraw(who, amount)?;

		// Cannot underflow because ensure_can_withdraw check
		<TotalIssuance<T>>::mutate(|v| *v -= amount);

		// mutate the account's balance
		Self::set_free_balance(who, Self::free_balance(who) - amount);

		Ok(())
	}
}

impl<T: Config> BasicCurrencyExtended<T::AccountId> for Pallet<T> {
	type Amount = T::Amount;

	fn update_balance(who: &T::AccountId, by_amount: Self::Amount) -> DispatchResult {
		if by_amount.is_zero() {
			return Ok(());
		}

		// Ensure this doesn't overflow. There isn't any traits that exposes
		// `saturating_abs` so we need to do it manually.
		let by_amount_abs = if by_amount == Self::Amount::min_value() {
			Self::Amount::max_value()
		} else {
			by_amount.abs()
		};

		let by_balance =
			TryInto::<Self::Balance>::try_into(by_amount_abs).map_err(|_| Error::<T>::AmountIntoBalanceFailed)?;
		if by_amount.is_positive() {
			Self::deposit(who, by_balance)
		} else {
			Self::withdraw(who, by_balance).map(|_| ())
		}
	}
}

impl<T: Config> BasicLockableCurrency<T::AccountId> for Pallet<T> {
	type Moment = T::BlockNumber;

	// Set a lock on the balance of `who` under `currency_id`.
	// Is a no-op if lock amount is zero.
	fn set_lock(
		lock_id: LockIdentifier,
		who: &T::AccountId,
		amount: Self::Balance,
	) -> DispatchResult {
		if amount.is_zero() {
			return Ok(());
		}
		let mut new_lock = Some(BalanceLock { id: lock_id, amount });
		let mut locks = Self::locks(who)
			.into_iter()
			.filter_map(|lock| {
				if lock.id == lock_id {
					new_lock.take()
				} else {
					Some(lock)
				}
			})
			.collect::<Vec<_>>();
		if let Some(lock) = new_lock {
			locks.push(lock)
		}
		Self::update_locks(who, &locks[..])
	}

	// Extend a lock on the balance of `who` under `currency_id`.
	// Is a no-op if lock amount is zero
	fn extend_lock(
		lock_id: LockIdentifier,
		who: &T::AccountId,
		amount: Self::Balance,
	) -> DispatchResult {
		if amount.is_zero() {
			return Ok(());
		}
		let mut new_lock = Some(BalanceLock { id: lock_id, amount });
		let mut locks = Self::locks(who)
			.into_iter()
			.filter_map(|lock| {
				if lock.id == lock_id {
					new_lock.take().map(|nl| BalanceLock {
						id: lock.id,
						amount: lock.amount.max(nl.amount),
					})
				} else {
					Some(lock)
				}
			})
			.collect::<Vec<_>>();
		if let Some(lock) = new_lock {
			locks.push(lock)
		}
		Self::update_locks(who, &locks[..])
	}

	fn remove_lock(lock_id: LockIdentifier, who: &T::AccountId) -> DispatchResult {
		let mut locks = Self::locks(who);
		locks.retain(|lock| lock.id != lock_id);
		let locks_vec = locks.to_vec();
		Self::update_locks(who, &locks_vec[..])
	}
}

impl<T: Config> BasicReservableCurrency<T::AccountId> for Pallet<T> {
	/// Check if `who` can reserve `value` from their free balance.
	fn can_reserve(who: &T::AccountId, value: Self::Balance) -> bool {
		if value.is_zero() {
			return true;
		}
		Self::ensure_can_withdraw(who, value).is_ok()
	}

	fn reserved_balance(who: &T::AccountId) -> Self::Balance {
		Self::accounts(who).reserved
	}

	/// Move `value` from the free balance from `who` to their reserved balance.
	fn reserve(who: &T::AccountId, value: Self::Balance) -> DispatchResult {
		if value.is_zero() {
			return Ok(());
		}
		Self::ensure_can_withdraw( who, value)?;

		let account = Self::accounts(who);
		Self::set_free_balance(who, account.free - value);
		// Cannot overflow becuase total issuance is using the same balance type and
		// this doesn't increase total issuance
		Self::set_reserved_balance(who, account.reserved + value);

		Self::deposit_event(Event::Reserved(who.clone(), value));
		Ok(())
	}

	/// Unreserve some funds, returning any amount that was unable to be unreserved.
	fn unreserve(who: &T::AccountId, value: Self::Balance) -> Self::Balance {
		if value.is_zero() {
			return value;
		}

		let account = Self::accounts(who);
		let actual = account.reserved.min(value);
		Self::set_reserved_balance(who, account.reserved - actual);
		Self::set_free_balance(who, account.free + actual);

		Self::deposit_event(Event::Unreserved(who.clone(), actual));
		value - actual
	}
}

impl<T: Config> fungible::Inspect<T::AccountId> for Pallet<T> {
	type Balance = T::Balance;

	fn total_issuance() -> Self::Balance {
		Pallet::<T>::total_issuance()
	}
	fn minimum_balance() -> Self::Balance {
		<Self as BasicCurrency<_>>::minimum_balance()
	}
	fn balance(who: &T::AccountId) -> Self::Balance {
		Pallet::<T>::total_balance(who)
	}
	fn reducible_balance(who: &T::AccountId, keep_alive: bool) -> Self::Balance {
		let a = Pallet::<T>::accounts(who);
		// Liquid balance is what is neither reserved nor locked/frozen.
		let liquid = a.free.saturating_sub(a.frozen);
		if frame_system::Pallet::<T>::can_dec_provider(who) && !keep_alive {
			liquid
		} else {
			let must_remain_to_exist = T::ExistentialDeposits::get().saturating_sub(a.total() - liquid);
			liquid.saturating_sub(must_remain_to_exist)
		}
	}
	fn can_deposit(who: &T::AccountId, amount: Self::Balance) -> DepositConsequence {
		Pallet::<T>::deposit_consequence(who, amount, &Pallet::<T>::accounts(who))
	}
	fn can_withdraw(
		who: &T::AccountId,
		amount: Self::Balance,
	) -> WithdrawConsequence<Self::Balance> {
		Pallet::<T>::withdraw_consequence(who, amount, &Pallet::<T>::accounts(who))
	}
}

impl<T: Config> fungible::Mutate<T::AccountId> for Pallet<T> {
	fn mint_into(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
		if amount.is_zero() {
			return Ok(());
		}
		Self::try_mutate_account(who, |account, _existed| -> DispatchResult {
			Self::deposit_consequence(who, amount, &account).into_result()?;
			// deposit_consequence already did overflow checking
			account.free += amount;
			Ok(())
		})?;
		// deposit_consequence already did overflow checking
		<TotalIssuance<T>>::mutate(|t| *t += amount);
		Ok(())
	}

	fn burn_from(
		who: &T::AccountId,
		amount: Self::Balance,
	) -> Result<Self::Balance, DispatchError> {
		if amount.is_zero() {
			return Ok(Self::Balance::zero());
		}
		let actual = Self::try_mutate_account(
			who,
			|account, _existed| -> Result<T::Balance, DispatchError> {
				let extra = Self::withdraw_consequence(who, amount, &account).into_result()?;
				// withdraw_consequence already did underflow checking
				let actual = amount + extra;
				account.free -= actual;
				Ok(actual)
			},
		)?;
		// withdraw_consequence already did underflow checking
		<TotalIssuance<T>>::mutate(|t| *t -= actual);
		Ok(actual)
	}
}

impl<T: Config> fungible::Transfer<T::AccountId> for Pallet<T> {
	fn transfer(
		source: &T::AccountId,
		dest: &T::AccountId,
		amount: T::Balance,
		keep_alive: bool,
	) -> Result<T::Balance, DispatchError> {
		let er = if keep_alive {
			ExistenceRequirement::KeepAlive
		} else {
			ExistenceRequirement::AllowDeath
		};
		Self::do_transfer(source, dest, amount, er).map(|_| amount)
	}
}

impl<T: Config> fungible::Unbalanced<T::AccountId> for Pallet<T> {
	fn set_balance(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
		// Balance is the same type and will not overflow
		Self::mutate_account(who, |account, _| account.free = amount);
		Ok(())
	}

	fn set_total_issuance(amount: Self::Balance) {
		// Balance is the same type and will not overflow
		<TotalIssuance<T>>::mutate(|t| *t = amount);
	}
}

impl<T: Config> fungible::InspectHold<T::AccountId> for Pallet<T> {
	fn balance_on_hold(who: &T::AccountId) -> T::Balance {
		Self::accounts(who).reserved
	}
	fn can_hold(who: &T::AccountId, amount: T::Balance) -> bool {
		let a = Self::accounts(who);
		let min_balance = T::ExistentialDeposits::get().max(a.frozen);
		if a.reserved.checked_add(&amount).is_none() {
			return false;
		}
		// We require it to be min_balance + amount to ensure that the full reserved
		// funds may be slashed without compromising locked funds or destroying the account.
		let required_free = match min_balance.checked_add(&amount) {
			Some(x) => x,
			None => return false,
		};
		a.free >= required_free
	}
}

impl<T: Config> fungible::MutateHold<T::AccountId> for Pallet<T> {
	fn hold(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
		if amount.is_zero() {
			return Ok(());
		}
		ensure!(
			Self::can_reserve(who, amount),
			Error::<T>::BalanceTooLow
		);
		Self::mutate_account(who, |a, _| {
			// `can_reserve` has did underflow checking
			a.free -= amount;
			// Cannot overflow as `amount` is from `a.free`
			a.reserved += amount;
		});
		Ok(())
	}
	fn release(
		who: &T::AccountId,
		amount: Self::Balance,
		best_effort: bool,
	) -> Result<T::Balance, DispatchError> {
		if amount.is_zero() {
			return Ok(amount);
		}
		// Done on a best-effort basis.
		Self::try_mutate_account(who, |a, _existed| {
			let new_free = a.free.saturating_add(amount.min(a.reserved));
			let actual = new_free - a.free;
			// Guaranteed to be <= amount and <= a.reserved
			ensure!(best_effort || actual == amount, Error::<T>::BalanceTooLow);
			a.free = new_free;
			a.reserved = a.reserved.saturating_sub(actual);
			Ok(actual)
		})
	}
	fn transfer_held(
		source: &T::AccountId,
		dest: &T::AccountId,
		amount: Self::Balance,
		_best_effort: bool,
		on_hold: bool,
	) -> Result<Self::Balance, DispatchError> {
		// let status = if on_hold { Status::Reserved } else { Status::Free };
		// Pallet::<T>::repatriate_reserved(source, dest, amount, status)
		unimplemented!()
	}
}

// pub struct CurrencyAdapter<T>(marker::PhantomData<T>);
//
// impl<T> PalletCurrency<T::AccountId> for CurrencyAdapter<T>
// where
// 	T: Config,
// 	GetCurrencyId: Get<T::CurrencyId>,
// {
// 	type Balance = T::Balance;
// 	type PositiveImbalance = PositiveImbalance<T>;
// 	type NegativeImbalance = NegativeImbalance<T>;
//
// 	fn total_balance(who: &T::AccountId) -> Self::Balance {
// 		Pallet::<T>::total_balance(who)
// 	}
//
// 	fn can_slash(who: &T::AccountId, value: Self::Balance) -> bool {
// 		Pallet::<T>::can_slash(who, value)
// 	}
//
// 	fn total_issuance() -> Self::Balance {
// 		Pallet::<T>::total_issuance(GetCurrencyId::get())
// 	}
//
// 	fn minimum_balance() -> Self::Balance {
// 		Pallet::<T>::minimum_balance(GetCurrencyId::get())
// 	}
//
// 	fn burn(mut amount: Self::Balance) -> Self::PositiveImbalance {
// 		if amount.is_zero() {
// 			return PositiveImbalance::zero();
// 		}
// 		<TotalIssuance<T>>::mutate(|issued| {
// 			*issued = issued.checked_sub(&amount).unwrap_or_else(|| {
// 				amount = *issued;
// 				Zero::zero()
// 			});
// 		});
// 		PositiveImbalance::new(amount)
// 	}
//
// 	fn issue(mut amount: Self::Balance) -> Self::NegativeImbalance {
// 		if amount.is_zero() {
// 			return NegativeImbalance::zero();
// 		}
// 		<TotalIssuance<T>>::mutate(|issued| {
// 			*issued = issued.checked_add(&amount).unwrap_or_else(|| {
// 				amount = Self::Balance::max_value() - *issued;
// 				Self::Balance::max_value()
// 			})
// 		});
// 		NegativeImbalance::new(amount)
// 	}
//
// 	fn free_balance(who: &T::AccountId) -> Self::Balance {
// 		Pallet::<T>::free_balance(who)
// 	}
//
// 	fn ensure_can_withdraw(
// 		who: &T::AccountId,
// 		amount: Self::Balance,
// 		_reasons: WithdrawReasons,
// 		_new_balance: Self::Balance,
// 	) -> DispatchResult {
// 		Pallet::<T>::ensure_can_withdraw(who, amount)
// 	}
//
// 	fn transfer(
// 		source: &T::AccountId,
// 		dest: &T::AccountId,
// 		value: Self::Balance,
// 		existence_requirement: ExistenceRequirement,
// 	) -> DispatchResult {
// 		Pallet::<T>::do_transfer(&source, &dest, value, existence_requirement)
// 	}
//
// 	fn slash(who: &<T as Config>::AccountId, value: Self::Balance) -> (Self::NegativeImbalance, Self::Balance) {
// 		unimplemented!()
// 	}
//
// 	/// Deposit some `value` into the free balance of an existing target account
// 	/// `who`.
// 	fn deposit_into_existing(
// 		who: &T::AccountId,
// 		value: Self::Balance,
// 	) -> sp_std::result::Result<Self::PositiveImbalance, DispatchError> {
// 		if value.is_zero() {
// 			return Ok(Self::PositiveImbalance::zero());
// 		}
// 		Pallet::<T>::try_mutate_account(
// 			who,
// 			|account, existed| -> Result<Self::PositiveImbalance, DispatchError> {
// 				ensure!(existed, Error::<T>::DeadAccount);
// 				account.free = account.free.checked_add(&value).ok_or(ArithmeticError::Overflow)?;
// 				Ok(PositiveImbalance::new(value))
// 			},
// 		)
// 	}
//
// 	/// Deposit some `value` into the free balance of `who`, possibly creating a
// 	/// new account.
// 	fn deposit_creating(who: &T::AccountId, value: Self::Balance) -> Self::PositiveImbalance {
// 		if value.is_zero() {
// 			return Self::PositiveImbalance::zero();
// 		}
// 		Pallet::<T>::try_mutate_account(
// 			who,
// 			|account, existed| -> Result<Self::PositiveImbalance, DispatchError> {
// 				let ed = T::ExistentialDeposits::get();
// 				ensure!(value >= ed || existed, Error::<T>::ExistentialDeposit);
// 				account.free = account.free.checked_add(&value).ok_or(ArithmeticError::Overflow)?;
// 				Ok(PositiveImbalance::new(value))
// 			},
// 		)
// 		.unwrap_or_else(|_| Self::PositiveImbalance::zero())
// 	}
//
// 	fn withdraw(
// 		who: &T::AccountId,
// 		value: Self::Balance,
// 		_reasons: WithdrawReasons,
// 		liveness: ExistenceRequirement,
// 	) -> sp_std::result::Result<Self::NegativeImbalance, DispatchError> {
// 		if value.is_zero() {
// 			return Ok(Self::NegativeImbalance::zero());
// 		}
//
// 		Pallet::<T>::try_mutate_account(who, |account, _existed| -> DispatchResult {
// 			account.free = account.free.checked_sub(&value).ok_or(Error::<T>::BalanceTooLow)?;
//
// 			Pallet::<T>::ensure_can_withdraw(who, value)?;
//
// 			let ed = T::ExistentialDeposits::get();
// 			let allow_death = liveness == ExistenceRequirement::AllowDeath;
// 			let allow_death = allow_death && frame_system::Pallet::<T>::can_dec_provider(who);
// 			ensure!(allow_death || account.total() >= ed, Error::<T>::KeepAlive);
//
// 			Ok(())
// 		})?;
//
// 		Ok(Self::NegativeImbalance::new(value))
// 	}
//
// 	fn make_free_balance_be(
// 		who: &T::AccountId,
// 		value: Self::Balance,
// 	) -> SignedImbalance<Self::Balance, Self::PositiveImbalance> {
// 		Pallet::<T>::try_mutate_account(
// 			who,
// 			|account, existed| -> Result<SignedImbalance<Self::Balance, Self::PositiveImbalance>, ()> {
// 				let ed = T::ExistentialDeposits::get();
// 				ensure!(value.saturating_add(account.reserved) >= ed || existed, ());
//
// 				let imbalance = if account.free <= value {
// 					SignedImbalance::Positive(PositiveImbalance::new(value - account.free))
// 				} else {
// 					SignedImbalance::Negative(NegativeImbalance::new(account.free - value))
// 				};
// 				account.free = value;
// 				Ok(imbalance)
// 			},
// 		)
// 		.unwrap_or_else(|_| SignedImbalance::Positive(Self::PositiveImbalance::zero()))
// 	}
// }
//
// impl<T> PalletReservableCurrency<T::AccountId> for CurrencyAdapter<T>
// where
// 	T: Config,
// 	GetCurrencyId: Get<T::CurrencyId>,
// {
// 	fn can_reserve(who: &T::AccountId, value: Self::Balance) -> bool {
// 		Pallet::<T>::can_reserve(who, value)
// 	}
//
// 	fn slash_reserved(who: &T::AccountId, value: Self::Balance) -> (Self::NegativeImbalance, Self::Balance) {
// 		let actual = Pallet::<T>::slash_reserved(who, value);
// 		(Self::NegativeImbalance::zero(), actual)
// 	}
//
// 	fn reserved_balance(who: &T::AccountId) -> Self::Balance {
// 		Pallet::<T>::reserved_balance(who)
// 	}
//
// 	fn reserve(who: &T::AccountId, value: Self::Balance) -> DispatchResult {
// 		Pallet::<T>::reserve(who, value)
// 	}
//
// 	fn unreserve(who: &T::AccountId, value: Self::Balance) -> Self::Balance {
// 		Pallet::<T>::unreserve(who, value)
// 	}
//
// 	fn repatriate_reserved(
// 		slashed: &T::AccountId,
// 		beneficiary: &T::AccountId,
// 		value: Self::Balance,
// 		status: Status,
// 	) -> sp_std::result::Result<Self::Balance, DispatchError> {
// 		Pallet::<T>::repatriate_reserved(slashed, beneficiary, value, status)
// 	}
// }
//
// impl<T> PalletLockableCurrency<T::AccountId> for CurrencyAdapter<T>
// where
// 	T: Config,
// {
// 	type Moment = T::BlockNumber;
// 	type MaxLocks = ();
//
// 	fn set_lock(id: LockIdentifier, who: &T::AccountId, amount: Self::Balance, _reasons: WithdrawReasons) {
// 		let _ = Pallet::<T>::set_lock(id::get(), who, amount);
// 	}
//
// 	fn extend_lock(id: LockIdentifier, who: &T::AccountId, amount: Self::Balance, _reasons: WithdrawReasons) {
// 		let _ = Pallet::<T>::extend_lock(id::get(), who, amount);
// 	}
//
// 	fn remove_lock(id: LockIdentifier, who: &T::AccountId) {
// 		let _ = Pallet::<T>::remove_lock(id::get(), who);
// 	}
// }
//
// impl<T: Config> TransferAll<T::AccountId> for Pallet<T> {
// 	#[transactional]
// 	fn transfer_all(source: &T::AccountId, dest: &T::AccountId) -> DispatchResult {
// 		Accounts::<T>::iter_prefix(source).try_for_each(|( account_data)| -> DispatchResult {
// 			<Self as MultiCurrency<T::AccountId>>::transfer( source, dest, account_data.free)
// 		})
// 	}
// }
//
// impl<T> fungible::Inspect<T::AccountId> for CurrencyAdapter<T>
// where
// 	T: Config,
// {
// 	type Balance = T::Balance;
//
// 	fn total_issuance() -> Self::Balance {
// 		<Pallet<T> as fungibles::Inspect<_>>::total_issuance(GetCurrencyId::get())
// 	}
// 	fn minimum_balance() -> Self::Balance {
// 		<Pallet<T> as fungibles::Inspect<_>>::minimum_balance(GetCurrencyId::get())
// 	}
// 	fn balance(who: &T::AccountId) -> Self::Balance {
// 		<Pallet<T> as fungibles::Inspect<_>>::balance(who)
// 	}
// 	fn reducible_balance(who: &T::AccountId, keep_alive: bool) -> Self::Balance {
// 		<Pallet<T> as fungibles::Inspect<_>>::reducible_balance(who, keep_alive)
// 	}
// 	fn can_deposit(who: &T::AccountId, amount: Self::Balance) -> DepositConsequence {
// 		<Pallet<T> as fungibles::Inspect<_>>::can_deposit(who, amount)
// 	}
// 	fn can_withdraw(who: &T::AccountId, amount: Self::Balance) -> WithdrawConsequence<Self::Balance> {
// 		<Pallet<T> as fungibles::Inspect<_>>::can_withdraw(who, amount)
// 	}
// }
//
// impl<T> fungible::Mutate<T::AccountId> for CurrencyAdapter<T>
// where
// 	T: Config,
// {
// 	fn mint_into(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
// 		<Pallet<T> as fungibles::Mutate<_>>::mint_into(who, amount)
// 	}
// 	fn burn_from(who: &T::AccountId, amount: Self::Balance) -> Result<Self::Balance, DispatchError> {
// 		<Pallet<T> as fungibles::Mutate<_>>::burn_from(who, amount)
// 	}
// }
//
// impl<T> fungible::Transfer<T::AccountId> for CurrencyAdapter<T>
// where
// 	T: Config,
// {
// 	fn transfer(
// 		source: &T::AccountId,
// 		dest: &T::AccountId,
// 		amount: T::Balance,
// 		keep_alive: bool,
// 	) -> Result<T::Balance, DispatchError> {
// 		<Pallet<T> as fungibles::Transfer<_>>::transfer(source, dest, amount, keep_alive)
// 	}
// }
//
// impl<T> fungible::Unbalanced<T::AccountId> for CurrencyAdapter<T>
// where
// 	T: Config,
// {
// 	fn set_balance(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
// 		<Pallet<T> as fungibles::Unbalanced<_>>::set_balance(who, amount)
// 	}
// 	fn set_total_issuance(amount: Self::Balance) {
// 		<Pallet<T> as fungibles::Unbalanced<_>>::set_total_issuance(amount)
// 	}
// }
//
// impl<T> fungible::InspectHold<T::AccountId> for CurrencyAdapter<T>
// where
// 	T: Config,
// {
// 	fn balance_on_hold(who: &T::AccountId) -> T::Balance {
// 		<Pallet<T> as fungibles::InspectHold<_>>::balance_on_hold(who)
// 	}
// 	fn can_hold(who: &T::AccountId, amount: T::Balance) -> bool {
// 		<Pallet<T> as fungibles::InspectHold<_>>::can_hold(who, amount)
// 	}
// }
//
// impl<T> fungible::MutateHold<T::AccountId> for CurrencyAdapter<T>
// where
// 	T: Config,
// {
// 	fn hold(who: &T::AccountId, amount: Self::Balance) -> DispatchResult {
// 		<Pallet<T> as fungibles::MutateHold<_>>::hold(who, amount)
// 	}
// 	fn release(who: &T::AccountId, amount: Self::Balance, best_effort: bool) -> Result<T::Balance, DispatchError> {
// 		<Pallet<T> as fungibles::MutateHold<_>>::release(who, amount, best_effort)
// 	}
// 	fn transfer_held(
// 		source: &T::AccountId,
// 		dest: &T::AccountId,
// 		amount: Self::Balance,
// 		best_effort: bool,
// 		on_hold: bool,
// 	) -> Result<Self::Balance, DispatchError> {
// 		<Pallet<T> as fungibles::MutateHold<_>>::transfer_held(
// 			source,
// 			dest,
// 			amount,
// 			best_effort,
// 			on_hold,
// 		)
// 	}
// }
