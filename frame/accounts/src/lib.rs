#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Codec, Decode, Encode, MaxEncodedLen, EncodeLike};

use sp_runtime::{traits::StaticLookup, DispatchResult, RuntimeDebug};
use sp_std::prelude::*;

use frame_support::{traits::UnfilteredDispatchable, weights::GetDispatchInfo, BoundedVec, CloneNoBound, PartialEqNoBound, RuntimeDebugNoBound, WeakBoundedVec};
use sp_std::{
	boxed::Box,
};
// #[cfg(test)]
// mod mock;
// #[cfg(test)]
// mod tests;

pub use pallet::*;
use frame_support::traits::Get;

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
		type Call: Parameter + UnfilteredDispatchable<Origin = Self::Origin> + GetDispatchInfo;

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
			let sender = ensure_signed(origin)?;
			log::info!("perm_name:{:?}, threshold:{}", perm_name, threshold);

			let permission_data = BoundedVec::<_, T::MaxPermission>::try_from(perms).map_err(|_| DispatchError::CannotLookup)?;
			let auth_data = BoundedVec::<_, T::MaxAuth>::try_from(auths).map_err(|_| DispatchError::CannotLookup)?;
			let permission_auth_data = PermissionAndOwnerData {
				perm_type: perm_name,
				parent_perm_type: parent_perm_name,
				permissions: permission_data,
				threshold: threshold,
				auths: auth_data
			};
			let exist = PermissionMap::<T>::contains_key(&sender);
			if exist {
				// todo: only one owner and one active most, use another storage item for check
				let mut permissions_old =
					PermissionMap::<T>::get(&sender).ok_or(DispatchError::CannotLookup)?;
				permissions_old.try_push((perm_name, permission_auth_data));
				PermissionMap::<T>::try_mutate(sender, |permissions|->DispatchResult {
					match permissions {
						Some(permissions) => {
							*permissions = permissions_old;
						}
						None => {
							log::error!("try mutate but none");
						}
					}
					Ok(())
				});
			} else {
				let permission_vec = vec![(perm_name, permission_auth_data)];
				let permission_vec =
					BoundedVec::<_, T::MaxOthers>::try_from(permission_vec).map_err(|_| DispatchError::CannotLookup)?;
				PermissionMap::<T>::insert(sender, permission_vec);
			}
			Ok(())
		}

		#[pallet::weight(0)]
		pub fn update_permission(origin: OriginFor<T>,
								 perm_name: PermType,
			perm_model: PermModule,
			perm_effect: Effect,
			perm_method: Option<[u8; 10]>
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			// let (perm_name, mut permissions_old)
			// 	= PermissionMap::<T>::get(&sender).ok_or(DispatchError::CannotLookup)?;
			Ok(())
		}

		#[pallet::weight(0)]
		pub fn add_permission(origin: OriginFor<T>,
							  perm_name: PermType,
							  perm_data: Vec<OnePermissionData>
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			Ok(())
		}

		#[pallet::weight(0)]
		pub fn update_auth(origin: OriginFor<T>,
						   perm_name: PermType,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			Ok(())
		}

		#[pallet::weight(0)]
		pub fn add_auth(origin: OriginFor<T>,
						perm_name: PermType,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			Ok(())
		}
	}

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

	#[pallet::storage]
	pub(super) type PermissionMap<T: Config> = StorageMap<
		_,
		Twox64Concat,
		T::AccountId,
		BoundedVec<(PermType, PermissionAndOwnerData<T::MaxPermission, T::MaxAuth>), T::MaxOthers>,
		OptionQuery,
	>;

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

#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum PermType {
	Owner,
	Active,
	Others([u8; 10]),
}
#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum Effect {
	Positive,
	Negative,
}
#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum PermModule {
	ALL,
	SELF,
	Module([u8; 10])
}
#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum AccountOrKey {
	Account([u8; 10]),
	Key([u8; 10])
}

#[derive(
	CloneNoBound, Encode, Decode, Eq, MaxEncodedLen, PartialEqNoBound, RuntimeDebugNoBound,
)]
#[codec(mel_bound(MaxPerm: Get<u32>, MaxAuth: Get<u32>))]
#[cfg_attr(test, derive(frame_support::DefaultNoBound))]
pub struct PermissionAndOwnerData<MaxPerm: Get<u32>, MaxAuth: Get<u32>> {
	// permission name
	pub perm_type: PermType,
	// parent permission name
	pub parent_perm_type: Option<PermType>,
	// permission detail
	pub permissions: BoundedVec<OnePermissionData, MaxPerm>,
	// auth threshold weight
	pub threshold: u32,
	// auth detail
	pub auths: BoundedVec<OneAuthData, MaxAuth>,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub struct OnePermissionData {
	pub effect: Effect,
	// module name: self or other module/contract
	pub module: PermModule,
	// method name: if module is All, the method is None
	// if one module has many method, this will related to multi record
	pub method: Option<[u8; 10]>,
}
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub struct OneAuthData {
	pub account_key: AccountOrKey,
	pub weight: u32
}