// This file is part of Substrate.

// Copyright (C) 2020-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! RPC api for babe.

use sc_consensus_babe::{Epoch, authorship, Config};
use futures::{FutureExt as _, TryFutureExt as _};
use jsonrpc_core::{
	Error as RpcError,
	futures::future as rpc_future,
};
use jsonrpc_derive::rpc;
use sc_consensus_epochs::{descendent_query, Epoch as EpochT, SharedEpochChanges};
use sp_consensus_babe::{
	AuthorityId,
	BabeApi as BabeRuntimeApi,
	digests::PreDigest,
};
use serde::{Deserialize, Serialize};
use sp_core::{
	crypto::Public,
};
use sp_application_crypto::AppKey;
use sp_keystore::{SyncCryptoStorePtr, SyncCryptoStore};
use sc_rpc_api::DenyUnsafe;
use sp_api::{ProvideRuntimeApi, BlockId};
use sp_runtime::traits::{Block as BlockT, Header as _};
use sp_consensus::{SelectChain, Error as ConsensusError};
use sp_blockchain::{HeaderBackend, HeaderMetadata, Error as BlockChainError};
use std::{collections::HashMap, sync::Arc};

type FutureResult<T> = Box<dyn rpc_future::Future<Item = T, Error = RpcError> + Send>;

/// Provides rpc methods for interacting with Babe.
#[rpc]
pub trait BabeApi {
	/// Returns data about which slots (primary or secondary) can be claimed in the current epoch
	/// with the keys in the keystore.
	#[rpc(name = "babe_epochAuthorship")]
	fn epoch_authorship(&self) -> FutureResult<HashMap<AuthorityId, EpochAuthorship>>;
}

/// Implements the BabeRpc trait for interacting with Babe.
pub struct BabeRpcHandler<B: BlockT, C, SC> {
	/// shared reference to the client.
	client: Arc<C>,
	/// shared reference to EpochChanges
	shared_epoch_changes: SharedEpochChanges<B, Epoch>,
	/// shared reference to the Keystore
	keystore: SyncCryptoStorePtr,
	/// config (actually holds the slot duration)
	babe_config: Config,
	/// The SelectChain strategy
	select_chain: SC,
	/// Whether to deny unsafe calls
	deny_unsafe: DenyUnsafe,
}

impl<B: BlockT, C, SC> BabeRpcHandler<B, C, SC> {
	/// Creates a new instance of the BabeRpc handler.
	pub fn new(
		client: Arc<C>,
		shared_epoch_changes: SharedEpochChanges<B, Epoch>,
		keystore: SyncCryptoStorePtr,
		babe_config: Config,
		select_chain: SC,
		deny_unsafe: DenyUnsafe,
	) -> Self {
		Self {
			client,
			shared_epoch_changes,
			keystore,
			babe_config,
			select_chain,
			deny_unsafe,
		}
	}
}

impl<B, C, SC> BabeApi for BabeRpcHandler<B, C, SC>
	where
		B: BlockT,
		C: ProvideRuntimeApi<B> + HeaderBackend<B> + HeaderMetadata<B, Error=BlockChainError> + 'static,
		C::Api: BabeRuntimeApi<B>,
		SC: SelectChain<B> + Clone + 'static,
{
	fn epoch_authorship(&self) -> FutureResult<HashMap<AuthorityId, EpochAuthorship>> {
		if let Err(err) = self.deny_unsafe.check_if_safe() {
			return Box::new(rpc_future::err(err.into()));
		}

		let (
			babe_config,
			keystore,
			shared_epoch,
			client,
			select_chain,
		) = (
			self.babe_config.clone(),
			self.keystore.clone(),
			self.shared_epoch_changes.clone(),
			self.client.clone(),
			self.select_chain.clone(),
		);
		let future = async move {
			let header = select_chain.best_chain().map_err(Error::Consensus)?;
			let epoch_start = client.runtime_api()
				.current_epoch_start(&BlockId::Hash(header.hash()))
				.map_err(|err| {
					Error::StringError(format!("{:?}", err))
				})?;
			let epoch = epoch_data(
				&shared_epoch,
				&client,
				&babe_config,
				*epoch_start,
				&select_chain,
			)?;
			let (epoch_start, epoch_end) = (epoch.start_slot(), epoch.end_slot());

			let mut claims: HashMap<AuthorityId, EpochAuthorship> = HashMap::new();

			let keys = {
				epoch.authorities.iter()
					.enumerate()
					.filter_map(|(i, a)| {
						if SyncCryptoStore::has_keys(&*keystore, &[(a.0.to_raw_vec(), AuthorityId::ID)]) {
							Some((a.0.clone(), i))
						} else {
							None
						}
					})
					.collect::<Vec<_>>()
			};

			for slot in *epoch_start..*epoch_end {
				if let Some((claim, key)) =
					authorship::claim_slot_using_keys(slot.into(), &epoch, &keystore, &keys)
				{
					match claim {
						PreDigest::Primary { .. } => {
							claims.entry(key).or_default().primary.push(slot);
						}
						PreDigest::SecondaryPlain { .. } => {
							claims.entry(key).or_default().secondary.push(slot);
						}
						PreDigest::SecondaryVRF { .. } => {
							claims.entry(key).or_default().secondary_vrf.push(slot.into());
						},
					};
				}
			}

			Ok(claims)
		}.boxed();

		Box::new(future.compat())
	}
}

/// Holds information about the `slot`'s that can be claimed by a given key.
#[derive(Default, Debug, Deserialize, Serialize)]
pub struct EpochAuthorship {
	/// the array of primary slots that can be claimed
	primary: Vec<u64>,
	/// the array of secondary slots that can be claimed
	secondary: Vec<u64>,
	/// The array of secondary VRF slots that can be claimed.
	secondary_vrf: Vec<u64>,
}

/// Errors encountered by the RPC
#[derive(Debug, derive_more::Display, derive_more::From)]
pub enum Error {
	/// Consensus error
	Consensus(ConsensusError),
	/// Errors that can be formatted as a String
	StringError(String)
}

impl From<Error> for jsonrpc_core::Error {
	fn from(error: Error) -> Self {
		jsonrpc_core::Error {
			message: format!("{}", error),
			code: jsonrpc_core::ErrorCode::ServerError(1234),
			data: None,
		}
	}
}

/// fetches the epoch data for a given slot.
fn epoch_data<B, C, SC>(
	epoch_changes: &SharedEpochChanges<B, Epoch>,
	client: &Arc<C>,
	babe_config: &Config,
	slot: u64,
	select_chain: &SC,
) -> Result<Epoch, Error>
	where
		B: BlockT,
		C: HeaderBackend<B> + HeaderMetadata<B, Error=BlockChainError> + 'static,
		SC: SelectChain<B>,
{
	let parent = select_chain.best_chain()?;
	epoch_changes.shared_data().epoch_data_for_child_of(
		descendent_query(&**client),
		&parent.hash(),
		parent.number().clone(),
		slot.into(),
		|slot| Epoch::genesis(&babe_config, slot),
	)
		.map_err(|e| Error::Consensus(ConsensusError::ChainLookup(format!("{:?}", e))))?
		.ok_or(Error::Consensus(ConsensusError::InvalidAuthoritiesSet))
}

