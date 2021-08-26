#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Codec, Decode, Encode, MaxEncodedLen, EncodeLike};

use sp_runtime::{traits::StaticLookup, DispatchResult, RuntimeDebug};
use sp_std::prelude::*;

use frame_support::{traits::UnfilteredDispatchable, weights::GetDispatchInfo};
use sp_std::{
	boxed::Box,
};
// #[cfg(test)]
// mod mock;
// #[cfg(test)]
// mod tests;

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use super::{DispatchResult, *};
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use frame_support::WeakBoundedVec;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// The overarching event type.
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		/// A sudo-able call.
		type Call: Parameter + UnfilteredDispatchable<Origin = Self::Origin> + GetDispatchInfo;

		#[pallet::constant]
		type MaxPermission: Get<u32>;

		#[pallet::constant]
		type MaxAuth: Get<u32>;

		// type VarNameType: Parameter + Member + MaxEncodedLen + Ord + Copy;

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
		pub fn create_permission(
			origin: OriginFor<T>,
			perm_name: PermType,
			parent_perm_name: Option<PermType>,
			perm: PermissionData
			// perm_name: BoundedVec<u8, T::KeyLimit>,
			// parent_perm_name: Option<BoundedVec<u8, T::KeyLimit>>,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;
			log::info!("perm_name:{:?}", perm_name);

			// AuthThreshold::<T>::insert((sender, perm_name, parent_perm_name), 1);
			// PermissionMap::<T>::insert((&sender, &perm_name, &parent_perm_name), &perm);

			Ok(Pays::No.into())
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

	/// The `AccountId` of the sudo key.
	#[pallet::storage]
	#[pallet::getter(fn key)]
	pub(super) type Key<T: Config> = StorageValue<_, T::AccountId, ValueQuery>;

	#[pallet::storage]
	pub(super) type PermissionMap<T: Config> = StorageNMap<
		_,
		(
			NMapKey<Twox64Concat, T::AccountId>,
			NMapKey<Identity, PermType>, // permission name
			NMapKey<Identity, PermType>, // parent permission name
		),
		WeakBoundedVec<PermissionData, T::MaxPermission>,
		OptionQuery
	>;

	#[pallet::storage]
	pub(super) type AuthThreshold<T: Config> = StorageNMap<
		_,
		(
			NMapKey<Twox64Concat, T::AccountId>,
			NMapKey<Identity, BoundedVec<u8, T::KeyLimit>>, // permission name
			NMapKey<Identity, Option<BoundedVec<u8, T::KeyLimit>>>, // parent permission name
		),
		u32,
		OptionQuery
	>;

	#[pallet::storage]
	pub(super) type AuthMap<T: Config> = StorageNMap<
		_,
		(
			NMapKey<Twox64Concat, T::AccountId>,
			NMapKey<Identity, PermType>, // permission name
			NMapKey<Identity, PermType>, // parent permission name
		),
		WeakBoundedVec<AuthData, T::MaxAuth>,
		OptionQuery
	>;

	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		/// The `AccountId` of the sudo key.
		pub key: T::AccountId,
	}

	#[cfg(feature = "std")]
	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			Self { key: Default::default() }
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig<T> {
		fn build(&self) {
			<Key<T>>::put(&self.key);
		}
	}
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum PermType {
	Owner,
	Active,
	Others([u8; 10]),
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub struct PermissionData {
	pub effect: Effect,
	// module name: self or other module/contract
	pub module: PermModule,
	// method name: if module is All, the method is None
	// if one module has many method, this will related to multi PermissionData record
	pub method: Option<[u8; 10]>,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum Effect {
	Positive,
	Negative,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum PermModule {
	ALL,
	SELF,
	Module([u8; 10])
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub struct AuthData {
	pub account_key: AccountOrKey,
	pub weight: u8
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, MaxEncodedLen)]
pub enum AccountOrKey {
	Account([u8; 10]),
	Key([u8; 10])
}

// impl codec::EncodeLike for Option<PermType> {}
// impl codec::Decode for Option<PermType> {
// 	fn decode<I: codec::Input>(input: &mut I) -> sp_std::result::Result<Self, codec::Error> {
// 		unimplemented!()
// 	}
// }
// impl codec::Encode for Option<PermType> {
// 	fn encode(&self) -> Vec<u8> {
// 		unimplemented!()
// 	}
// }