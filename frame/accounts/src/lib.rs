#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Codec, Decode, Encode, MaxEncodedLen, EncodeLike};

use sp_runtime::{traits::StaticLookup, DispatchResult, RuntimeDebug};
use sp_std::prelude::*;

use frame_support::{traits::{Get, UnfilteredDispatchable},
					weights::GetDispatchInfo, BoundedVec, CloneNoBound, PartialEqNoBound, RuntimeDebugNoBound, WeakBoundedVec};

// #[cfg(test)]
// mod mock;
// #[cfg(test)]
// mod tests;

mod types;
mod impls;
mod extension;
pub use types::*;
pub use extension::AccountExtension;

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use super::{DispatchResult, *};
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use frame_support::WeakBoundedVec;
	use frame_support::BoundedVec;
	use sp_std::convert::TryFrom;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// The overarching event type.
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		/// A sudo-able call.
		// type Call: Parameter + UnfilteredDispatchable<Origin = Self::Origin> + GetDispatchInfo;

		#[pallet::constant]
		type MaxPermission: Get<u32>;

		#[pallet::constant]
		type MaxOthers: Get<u32>;

		#[pallet::constant]
		type MaxAuth: Get<u32>;

		#[pallet::constant]
		type KeyLimit: Get<u32>;
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	#[pallet::generate_storage_info]
	pub struct Pallet<T>(PhantomData<T>);

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::weight(0)]
		pub fn create_permission_auth(
			origin: OriginFor<T>,
			perm_name: PermType,
			parent_perm_name: Option<PermType>,
			perms: Vec<OnePermissionData>,
			auths: Vec<OneAuthData>,
			threshold: u32,
		) -> DispatchResult {
			log::info!("perm_name:{:?}, threshold:{}", perm_name, threshold);
			Pallet::<T>::_create_permission_auth(origin, perm_name, parent_perm_name, perms, auths, threshold)
		}

		#[pallet::weight(0)]
		pub fn add_or_update_permission(origin: OriginFor<T>,
										perm_name: PermType,
										perm_module: PermModule,
										perm_effect: Effect,
										perm_method: Option<[u8; 5]>
		) -> DispatchResult {
			Pallet::<T>::_add_or_update_permission(origin, perm_name, perm_module, perm_effect, perm_method)
		}

		#[pallet::weight(0)]
		pub fn add_or_update_auth(origin: OriginFor<T>,
								  perm_name: PermType,
								  account_key: AccountOrKey,
								  weight: u32,
								  threshold: u32,
		) -> DispatchResult {
			Pallet::<T>::_add_or_update_auth(origin, perm_name, account_key, weight, threshold)
		}

		#[pallet::weight(0)]
		pub fn delete_permission(origin: OriginFor<T>,
								 perm_name: PermType,
								 perm_modules: Vec<PermModule>,
		) -> DispatchResult {
			Pallet::<T>::_delete_permission(origin, perm_name, perm_modules)
		}

		#[pallet::weight(0)]
		pub fn delete_auth(origin: OriginFor<T>,
						   perm_name: PermType,
						   account_keys: Vec<AccountOrKey>,
		) -> DispatchResult {
			Pallet::<T>::_delete_auth(origin, perm_name, account_keys)
		}
	}

	#[pallet::storage]
	pub(super) type PermissionMap<T: Config> = StorageMap<
		_,
		Twox64Concat,
		T::AccountId,
		BoundedVec<(PermType, PermissionAndOwnerData<T::MaxPermission, T::MaxAuth>), T::MaxOthers>,
		OptionQuery,
	>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	#[pallet::metadata(T::AccountId = "AccountId")]
	pub enum Event<T: Config> {
		/// A sudo just took place. \[result\]
		Sudid(DispatchResult),
		/// The \[sudoer\] just switched identity; the old key is supplied.
		KeyChanged(T::AccountId),
		/// A sudo just took place. \[result\]
		SudoAsDone(DispatchResult),
	}

	#[pallet::error]
	/// Error for the Sudo pallet
	pub enum Error<T> {
		/// Sender must be the Sudo account
		RequireSudo,
	}

	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		phantom: PhantomData<T>,
	}

	#[cfg(feature = "std")]
	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			GenesisConfig { phantom: Default::default() }
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig<T> {
		fn build(&self) {}
	}
}