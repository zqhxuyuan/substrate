use crate::{Config, Pallet, *};
use codec::{Codec, Decode, Encode, MaxEncodedLen, EncodeLike};

use sp_runtime::{traits::StaticLookup, DispatchResult, RuntimeDebug, DispatchError, AccountId32};
use sp_std::prelude::*;

use frame_support::{traits::{Get, UnfilteredDispatchable}, weights::GetDispatchInfo, BoundedVec, CloneNoBound, PartialEqNoBound, RuntimeDebugNoBound, WeakBoundedVec};

use frame_support::pallet_prelude::*;
use frame_system::pallet_prelude::*;
use sp_std::convert::TryFrom;

impl<T: Config> Pallet<T> {
    pub fn _create_account(
        _: OriginFor<T>,
        account_id: AccountId32, // account name
        pub_key_account: AccountId32,
        active_pub_key_account: Option<AccountId32>,
    ) -> DispatchResult {
        // let account_ids = vec![pub_key_account];
        // let account_vec = BoundedVec::<_, T::MaxOthers>::try_from(account_ids)
        //     .map_err(|_| DispatchError::CannotLookup)?;
        // OwnerAccountIdMap::<T>::insert(account_id.clone(), account_vec);
        // if let Some(active_pub_key_account) = active_pub_key_account {
        //     let account_ids = vec![active_pub_key_account];
        //     let account_vec = BoundedVec::<_, T::MaxOthers>::try_from(account_ids)
        //         .map_err(|_| DispatchError::CannotLookup)?;
        //     ActiveAccountIdMap::<T>::insert(account_id, account_vec);
        // };
        OwnerAccountIdMap::<T>::insert(pub_key_account, account_id.clone());
        if let Some(active_pub_key_account) = active_pub_key_account {
            OwnerAccountIdMap::<T>::insert(active_pub_key_account, account_id);
        }
        Ok(())
    }

    pub fn _create_permission_auth(
        origin: OriginFor<T>, // public key of role
        account_id: T::AccountId, // account name encoded as account id
        perm_name: PermType,
        parent_perm_name: Option<PermType>,
        perms: Vec<OnePermissionData>,
        auths: Vec<OneAuthData>,
        threshold: u32,
    ) -> DispatchResult {
        // we need to sign public key of role and account name, means set permission for account
        ensure_signed(origin)?;
        let permission_data = BoundedVec::<_, T::MaxPermission>::try_from(perms).map_err(|_| DispatchError::CannotLookup)?;
        let auth_data = BoundedVec::<_, T::MaxAuth>::try_from(auths).map_err(|_| DispatchError::CannotLookup)?;
        let permission_auth_data = PermissionAndOwnerData {
            perm_type: perm_name,
            parent_perm_type: parent_perm_name,
            permissions: permission_data,
            threshold: threshold,
            auths: auth_data
        };
        let exist = PermissionMap::<T>::contains_key(&account_id);
        if exist {
            // todo: only one owner and one active most, use another storage item for check
            let mut permissions_old =
                PermissionMap::<T>::get(&account_id).ok_or(DispatchError::CannotLookup)?;
            let perm_types: Vec<PermType> = permissions_old.iter()
                .map(|(x,_)| *x).collect::<Vec<PermType>>();
            log::info!("exist perm types:{:?}", perm_types);
            let perm_exist = perm_types.contains(&perm_name);
            if perm_exist {
                // todo: error
                log::warn!("perm type exist");
                return Ok(())
            }
            permissions_old.try_push((perm_name, permission_auth_data));
            PermissionMap::<T>::try_mutate(account_id, |permissions|->DispatchResult {
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
            PermissionMap::<T>::insert(account_id, permission_vec);
        }
        Ok(())
    }

    pub fn _add_or_update_permission(origin: OriginFor<T>,
                                     account_id: T::AccountId, // account name encoded as account id
                                     perm_name: PermType,
                                     perm_module: PermModule,
                                     perm_effect: Effect,
                                     perm_method: Option<[u8; 5]>
    ) -> DispatchResult {
        let sender = ensure_signed(origin)?;
        let mut permissions_old =
            PermissionMap::<T>::get(&account_id).ok_or(DispatchError::CannotLookup)?;

        let mut permission_new = BoundedVec::<OnePermissionData, T::MaxPermission>::default();
        let current_permission_data = OnePermissionData {
            effect: perm_effect,
            module: perm_module,
            method: perm_method
        };
        permission_new.try_push(current_permission_data);
        for (perm_type, perm_data) in permissions_old {
            if perm_type.eq(&perm_name) {
                for perm in perm_data.permissions {
                    if !perm.module.eq(&perm_module) {
                        permission_new.try_push(perm);
                    }
                }
                let current_permission_auth_data = PermissionAndOwnerData {
                    perm_type: perm_name,
                    parent_perm_type: perm_data.parent_perm_type,
                    permissions: permission_new,
                    threshold: perm_data.threshold,
                    auths: perm_data.auths
                };
                // todo: use try_mutate?
                PermissionMap::<T>::remove(&account_id);
                let permission_vec = vec![(perm_name, current_permission_auth_data)];
                let permission_vec =
                    BoundedVec::<_, T::MaxOthers>::try_from(permission_vec).map_err(|_| DispatchError::CannotLookup)?;
                PermissionMap::<T>::insert(&account_id, permission_vec);
                return Ok(())
            }
        }
        Ok(())
    }

    pub fn _add_or_update_auth(origin: OriginFor<T>,
                               account_id: T::AccountId,
                               perm_name: PermType,
                               account_key: AccountOrKey,
                               weight: u32,
                               threshold: u32,
    ) -> DispatchResult {
        let sender = ensure_signed(origin)?;
        let mut permissions_old =
            PermissionMap::<T>::get(&account_id).ok_or(DispatchError::CannotLookup)?;

        let mut latest_data = BoundedVec::<OneAuthData, T::MaxAuth>::default();
        let current_data = OneAuthData {
            account_key,
            weight
        };
        latest_data.try_push(current_data);
        for (perm_type, perm_data) in permissions_old {
            if perm_type.eq(&perm_name) {
                for perm in perm_data.auths {
                    if !perm.account_key.eq(&account_key) {
                        latest_data.try_push(perm);
                    }
                }
                let current_permission_auth_data = PermissionAndOwnerData {
                    perm_type: perm_name,
                    parent_perm_type: perm_data.parent_perm_type,
                    permissions: perm_data.permissions,
                    threshold,
                    auths: latest_data
                };
                // todo: use try_mutate?
                PermissionMap::<T>::remove(&account_id);
                let permission_vec = vec![(perm_name, current_permission_auth_data)];
                let permission_vec =
                    BoundedVec::<_, T::MaxOthers>::try_from(permission_vec).map_err(|_| DispatchError::CannotLookup)?;
                PermissionMap::<T>::insert(&account_id, permission_vec);
                return Ok(())
            }
        }
        Ok(())
    }

    pub fn _delete_permission(origin: OriginFor<T>,
                              account_id: T::AccountId,
                              perm_name: PermType,
                              perm_modules: Vec<PermModule>,
    ) -> DispatchResult {
        let sender = ensure_signed(origin)?;
        let mut permissions_old =
            PermissionMap::<T>::get(&account_id).ok_or(DispatchError::CannotLookup)?;

        let mut permission_new = BoundedVec::<OnePermissionData, T::MaxPermission>::default();
        for (perm_type, perm_data) in permissions_old {
            if perm_type.eq(&perm_name) {
                let mut exist = false;
                for perm in perm_data.permissions {
                    for perm_module in &perm_modules {
                        if !perm.module.eq(perm_module) {
                            permission_new.try_push(perm);
                        } else {
                            exist = true;
                        }
                    }
                }
                if !exist {
                    return Ok(())
                }
                let current_permission_auth_data = PermissionAndOwnerData {
                    perm_type: perm_name,
                    parent_perm_type: perm_data.parent_perm_type,
                    permissions: permission_new,
                    threshold: perm_data.threshold,
                    auths: perm_data.auths
                };
                // todo: use try_mutate?
                PermissionMap::<T>::remove(&account_id);
                let permission_vec = vec![(perm_name, current_permission_auth_data)];
                let permission_vec =
                    BoundedVec::<_, T::MaxOthers>::try_from(permission_vec).map_err(|_| DispatchError::CannotLookup)?;
                PermissionMap::<T>::insert(&account_id, permission_vec);
                return Ok(())
            }
        }
        Ok(())
    }

    pub fn _delete_auth(origin: OriginFor<T>,
                        account_id: T::AccountId,
                        perm_name: PermType,
                        account_keys: Vec<AccountOrKey>,
    ) -> DispatchResult {
        let sender = ensure_signed(origin)?;
        let mut permissions_old =
            PermissionMap::<T>::get(&account_id).ok_or(DispatchError::CannotLookup)?;

        let mut data_new = BoundedVec::<OneAuthData, T::MaxAuth>::default();
        for (perm_type, perm_data) in permissions_old {
            if perm_type.eq(&perm_name) {
                let mut exist = false;
                for data in perm_data.auths {
                    for account_key in &account_keys {
                        if !data.account_key.eq(account_key) {
                            data_new.try_push(data);
                        } else {
                            exist = true;
                        }
                    }
                }
                if !exist {
                    return Ok(())
                }
                let current_permission_auth_data = PermissionAndOwnerData {
                    perm_type: perm_name,
                    parent_perm_type: perm_data.parent_perm_type,
                    permissions: perm_data.permissions,
                    threshold: perm_data.threshold,
                    auths: data_new
                };
                // todo: use try_mutate?
                PermissionMap::<T>::remove(&account_id);
                let permission_vec = vec![(perm_name, current_permission_auth_data)];
                let permission_vec =
                    BoundedVec::<_, T::MaxOthers>::try_from(permission_vec).map_err(|_| DispatchError::CannotLookup)?;
                PermissionMap::<T>::insert(&account_id, permission_vec);
                return Ok(())
            }
        }
        Ok(())
    }
}