#![cfg_attr(not(feature = "std"), no_std)]

#[macro_use]
mod tests;
mod tests_local;
mod tests_composite;
mod tests_reentrancy;
mod benchmarking;
pub mod weights;
mod currency_impl;
pub mod model;

use sp_std::prelude::*;
use sp_std::{cmp, result, mem, fmt::Debug, ops::BitOr};
use codec::{Codec, Encode, Decode, MaxEncodedLen};
use frame_support::{ensure, WeakBoundedVec, traits::{
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
pub use weights::WeightInfo;
pub use model::*;

pub use pallet::*;
use sp_std::convert::TryInto;

#[frame_support::pallet]
pub mod pallet {
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use super::*;
	use crate::currency_impl::PalletImpl;
	use module_common::currency::VerticCurrency;

	#[pallet::config]
	pub trait Config<I: 'static = ()>: frame_system::Config {
		/// The balance of an account.
		type Balance: Parameter + Member + AtLeast32BitUnsigned + Codec + Default + Copy +
		MaybeSerializeDeserialize + Debug + MaxEncodedLen;

		/// The overarching event type.
		type Event: From<Event<Self, I>> + IsType<<Self as frame_system::Config>::Event>;

		/// The minimum amount required to keep an account open.
		#[pallet::constant]
		type ExistentialDeposit: Get<Self::Balance>;

		/// The means of storing the balances of an account.
		type AccountStore: StoredMap<Self::AccountId, AccountData<Self::Balance>>;

		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	#[pallet::generate_storage_info]
	pub struct Pallet<T, I=()>(PhantomData<(T, I)>);

	#[pallet::call]
	impl<T: Config<I>, I: 'static> Pallet<T, I> {
		#[pallet::weight(T::WeightInfo::transfer())]
		pub fn transfer(
			origin: OriginFor<T>,
			dest: <T::Lookup as StaticLookup>::Source,
			#[pallet::compact] value: T::Balance,
		) -> DispatchResultWithPostInfo {
			let transactor = ensure_signed(origin)?;
			let dest = T::Lookup::lookup(dest)?;
			<PalletImpl::<T, I> as VerticCurrency<_>>::transfer(&transactor, &dest, value, ExistenceRequirement::AllowDeath)?;
			Ok(().into())
		}

		#[pallet::weight(T::WeightInfo::transfer_keep_alive())]
		pub fn transfer_keep_alive(
			origin: OriginFor<T>,
			dest: <T::Lookup as StaticLookup>::Source,
			#[pallet::compact] value: T::Balance,
		) -> DispatchResultWithPostInfo {
			let transactor = ensure_signed(origin)?;
			let dest = T::Lookup::lookup(dest)?;
			<PalletImpl::<T, I> as VerticCurrency<_>>::transfer(&transactor, &dest, value, KeepAlive)?;
			Ok(().into())
		}

		#[pallet::weight(
		T::WeightInfo::set_balance_creating() // Creates a new account.
		.max(T::WeightInfo::set_balance_killing()) // Kills an existing account.
		)]
		pub fn set_balance(
			origin: OriginFor<T>,
			who: <T::Lookup as StaticLookup>::Source,
			#[pallet::compact] new_free: T::Balance,
			#[pallet::compact] new_reserved: T::Balance,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;
			let who = T::Lookup::lookup(who)?;
			let existential_deposit = T::ExistentialDeposit::get();

			let wipeout = new_free + new_reserved < existential_deposit;
			let new_free = if wipeout { Zero::zero() } else { new_free };
			let new_reserved = if wipeout { Zero::zero() } else { new_reserved };

			let (free, reserved) = Self::mutate_account(&who, |account| {
				account.free = new_free;
				account.reserved = new_reserved;

				(account.free, account.reserved)
			})?;
			Self::deposit_event(Event::BalanceSet(who, free, reserved));
			Ok(().into())
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	#[pallet::metadata(T::AccountId = "AccountId", T::Balance = "Balance")]
	pub enum Event<T: Config<I>, I: 'static = ()> {
		/// An account was created with some free balance. \[account, free_balance\]
		Endowed(T::AccountId, T::Balance),
		/// An account was removed whose balance was non-zero but below ExistentialDeposit,
		/// resulting in an outright loss. \[account, balance\]
		DustLost(T::AccountId, T::Balance),
		/// Transfer succeeded. \[from, to, value\]
		Transfer(T::AccountId, T::AccountId, T::Balance),
		/// A balance was set by root. \[who, free, reserved\]
		BalanceSet(T::AccountId, T::Balance, T::Balance),
		/// Some amount was deposited (e.g. for transaction fees). \[who, deposit\]
		Deposit(T::AccountId, T::Balance),
		/// Some balance was reserved (moved from free to reserved). \[who, value\]
		Reserved(T::AccountId, T::Balance),
		/// Some balance was unreserved (moved from reserved to free). \[who, value\]
		Unreserved(T::AccountId, T::Balance),
		/// Some balance was moved from the reserve of the first account to the second account.
		/// Final argument indicates the destination balance type.
		/// \[from, to, balance, destination_status\]
		ReserveRepatriated(T::AccountId, T::AccountId, T::Balance, Status),
	}

	/// Old name generated by `decl_event`.
	#[deprecated(note = "use `Event` instead")]
	pub type RawEvent<T, I = ()> = Event<T, I>;

	#[pallet::error]
	pub enum Error<T, I = ()> {
		/// Vesting balance too high to send value
		VestingBalance,
		/// Account liquidity restrictions prevent withdrawal
		LiquidityRestrictions,
		/// Balance too low to send value
		InsufficientBalance,
		/// Value too low to create account due to existential deposit
		ExistentialDeposit,
		/// Transfer/payment would kill account
		KeepAlive,
		/// A vesting schedule already exists for this account
		ExistingVestingSchedule,
		/// Beneficiary account must pre-exist
		DeadAccount,
		/// Number of named reserves exceed MaxReserves
		TooManyReserves,
	}

	/// The total units issued in the system.
	#[pallet::storage]
	#[pallet::getter(fn total_issuance)]
	pub type TotalIssuance<T: Config<I>, I: 'static = ()> = StorageValue<_, T::Balance, ValueQuery>;

	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config<I>, I: 'static = ()> {
		pub balances: Vec<(T::AccountId, T::Balance)>,
	}

	#[cfg(feature = "std")]
	impl<T: Config<I>, I: 'static> Default for GenesisConfig<T, I> {
		fn default() -> Self {
			Self {
				balances: Default::default(),
			}
		}
	}

	#[pallet::genesis_build]
	impl<T: Config<I>, I: 'static> GenesisBuild<T, I> for GenesisConfig<T, I> {
		fn build(&self) {
			let total = self.balances
				.iter()
				.fold(Zero::zero(), |acc: T::Balance, &(_, n)| acc + n);
			<TotalIssuance<T, I>>::put(total);

			for (_, balance) in &self.balances {
				assert!(
					*balance >= <T as Config<I>>::ExistentialDeposit::get(),
					"the balance of any account should always be at least the existential deposit.",
				)
			}

			// ensure no duplicates exist.
			let endowed_accounts = self.balances.iter().map(|(x, _)| x).cloned().collect::<std::collections::BTreeSet<_>>();

			assert!(endowed_accounts.len() == self.balances.len(), "duplicate balances in genesis.");

			for &(ref who, free) in self.balances.iter() {
				assert!(T::AccountStore::insert(who, AccountData { free, ..Default::default() }).is_ok());
			}
		}
	}
}

#[cfg(feature = "std")]
impl<T: Config<I>, I: 'static> GenesisConfig<T, I> {
	/// Direct implementation of `GenesisBuild::build_storage`.
	///
	/// Kept in order not to break dependency.
	pub fn build_storage(&self) -> Result<sp_runtime::Storage, String> {
		<Self as GenesisBuild<T, I>>::build_storage(self)
	}

	/// Direct implementation of `GenesisBuild::assimilate_storage`.
	///
	/// Kept in order not to break dependency.
	pub fn assimilate_storage(
		&self,
		storage: &mut sp_runtime::Storage
	) -> Result<(), String> {
		<Self as GenesisBuild<T, I>>::assimilate_storage(self, storage)
	}
}