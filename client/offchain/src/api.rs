// This file is part of Substrate.

// Copyright (C) 2019-2021 Parity Technologies (UK) Ltd.
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

use std::{
	str::FromStr,
	sync::Arc,
	convert::TryFrom,
	thread::sleep,
	collections::HashSet,
};

use crate::NetworkProvider;
use futures::Future;
use sc_network::{PeerId, Multiaddr};
use codec::{Encode, Decode};
use sp_core::OpaquePeerId;
use sp_core::offchain::{
	self, HttpRequestId, Timestamp, HttpRequestStatus, HttpError,
	OffchainStorage, OpaqueNetworkState, OpaqueMultiaddr, StorageKind,
};
pub use sp_offchain::STORAGE_PREFIX;
pub use http::SharedClient;

#[cfg(not(target_os = "unknown"))]
mod http;

#[cfg(target_os = "unknown")]
use http_dummy as http;
#[cfg(target_os = "unknown")]
mod http_dummy;

mod timestamp;

fn unavailable_yet<R: Default>(name: &str) -> R {
	log::error!(
		target: "sc_offchain",
		"The {:?} API is not available for offchain workers yet. Follow \
		https://github.com/paritytech/substrate/issues/1458 for details", name
	);
	Default::default()
}

const LOCAL_DB: &str = "LOCAL (fork-aware) DB";

/// Offchain DB reference.
#[derive(Debug, Clone)]
pub struct Db<Storage> {
	/// Persistent storage database.
	persistent: Storage,
}

impl<Storage: OffchainStorage> Db<Storage> {
	/// Create new instance of Offchain DB.
	pub fn new(persistent: Storage) -> Self {
		Self { persistent }
	}

	/// Create new instance of Offchain DB, backed by given backend.
	pub fn factory_from_backend<Backend, Block>(backend: &Backend) -> Option<
		Box<dyn sc_client_api::execution_extensions::DbExternalitiesFactory>
	> where
		Backend: sc_client_api::Backend<Block, OffchainStorage = Storage>,
		Block: sp_runtime::traits::Block,
		Storage: 'static,
	{
		sc_client_api::Backend::offchain_storage(backend).map(|db|
			Box::new(Self::new(db)) as _
		)
	}
}

impl<Storage: OffchainStorage> offchain::DbExternalities for Db<Storage> {
	fn local_storage_set(&mut self, kind: StorageKind, key: &[u8], value: &[u8]) {
		log::debug!(
			target: "sc_offchain",
			"{:?}: Write: {:?} <= {:?}", kind, hex::encode(key), hex::encode(value)
		);
		match kind {
			StorageKind::PERSISTENT => self.persistent.set(STORAGE_PREFIX, key, value),
			StorageKind::LOCAL => unavailable_yet(LOCAL_DB),
		}
	}

	fn local_storage_clear(&mut self, kind: StorageKind, key: &[u8]) {
		log::debug!(
			target: "sc_offchain",
			"{:?}: Clear: {:?}", kind, hex::encode(key)
		);
		match kind {
			StorageKind::PERSISTENT => self.persistent.remove(STORAGE_PREFIX, key),
			StorageKind::LOCAL => unavailable_yet(LOCAL_DB),
		}
	}

	fn local_storage_compare_and_set(
		&mut self,
		kind: StorageKind,
		key: &[u8],
		old_value: Option<&[u8]>,
		new_value: &[u8],
	) -> bool {
		log::debug!(
			target: "sc_offchain",
			"{:?}: CAS: {:?} <= {:?} vs {:?}",
			kind,
			hex::encode(key),
			hex::encode(new_value),
			old_value.as_ref().map(hex::encode),
		);
		match kind {
			StorageKind::PERSISTENT => {
				self.persistent.compare_and_set(STORAGE_PREFIX, key, old_value, new_value)
			},
			StorageKind::LOCAL => unavailable_yet(LOCAL_DB),
		}
	}

	fn local_storage_get(&mut self, kind: StorageKind, key: &[u8]) -> Option<Vec<u8>> {
		let result = match kind {
			StorageKind::PERSISTENT => self.persistent.get(STORAGE_PREFIX, key),
			StorageKind::LOCAL => unavailable_yet(LOCAL_DB),
		};
		log::debug!(
			target: "sc_offchain",
			"{:?}: Read: {:?} => {:?}",
			kind,
			hex::encode(key),
			result.as_ref().map(hex::encode)
		);
		result
	}
}

/// Asynchronous offchain API.
///
/// NOTE this is done to prevent recursive calls into the runtime
/// (which are not supported currently).
pub(crate) struct Api {
	/// A provider for substrate networking.
	network_provider: Arc<dyn NetworkProvider + Send + Sync>,
	/// Is this node a potential validator?
	is_validator: bool,
	/// Everything HTTP-related is handled by a different struct.
	http: http::HttpApi,
}

impl offchain::Externalities for Api {
	fn is_validator(&self) -> bool {
		self.is_validator
	}

	fn network_state(&self) -> Result<OpaqueNetworkState, ()> {
		let external_addresses = self.network_provider.external_addresses();

		let state = NetworkState::new(
			self.network_provider.local_peer_id(),
			external_addresses,
		);
		Ok(OpaqueNetworkState::from(state))
	}

	fn timestamp(&mut self) -> Timestamp {
		timestamp::now()
	}

	fn sleep_until(&mut self, deadline: Timestamp) {
		sleep(timestamp::timestamp_from_now(deadline));
	}

	fn random_seed(&mut self) -> [u8; 32] {
		rand::random()
	}

	fn http_request_start(
		&mut self,
		method: &str,
		uri: &str,
		_meta: &[u8]
	) -> Result<HttpRequestId, ()> {
		self.http.request_start(method, uri)
	}

	fn http_request_add_header(
		&mut self,
		request_id: HttpRequestId,
		name: &str,
		value: &str
	) -> Result<(), ()> {
		self.http.request_add_header(request_id, name, value)
	}

	fn http_request_write_body(
		&mut self,
		request_id: HttpRequestId,
		chunk: &[u8],
		deadline: Option<Timestamp>
	) -> Result<(), HttpError> {
		self.http.request_write_body(request_id, chunk, deadline)
	}

	fn http_response_wait(
		&mut self,
		ids: &[HttpRequestId],
		deadline: Option<Timestamp>
	) -> Vec<HttpRequestStatus> {
		self.http.response_wait(ids, deadline)
	}

	fn http_response_headers(
		&mut self,
		request_id: HttpRequestId
	) -> Vec<(Vec<u8>, Vec<u8>)> {
		self.http.response_headers(request_id)
	}

	fn http_response_read_body(
		&mut self,
		request_id: HttpRequestId,
		buffer: &mut [u8],
		deadline: Option<Timestamp>
	) -> Result<usize, HttpError> {
		self.http.response_read_body(request_id, buffer, deadline)
	}

	fn set_authorized_nodes(&mut self, nodes: Vec<OpaquePeerId>, authorized_only: bool) {
		let peer_ids: HashSet<PeerId> = nodes.into_iter()
			.filter_map(|node| PeerId::from_bytes(&node.0).ok())
			.collect();

		self.network_provider.set_authorized_peers(peer_ids);
		self.network_provider.set_authorized_only(authorized_only);
	}
}

/// Information about the local node's network state.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct NetworkState {
	peer_id: PeerId,
	external_addresses: Vec<Multiaddr>,
}

impl NetworkState {
	fn new(peer_id: PeerId, external_addresses: Vec<Multiaddr>) -> Self {
		NetworkState {
			peer_id,
			external_addresses,
		}
	}
}

impl From<NetworkState> for OpaqueNetworkState {
	fn from(state: NetworkState) -> OpaqueNetworkState {
		let enc = Encode::encode(&state.peer_id.to_bytes());
		let peer_id = OpaquePeerId::new(enc);

		let external_addresses: Vec<OpaqueMultiaddr> = state
			.external_addresses
			.iter()
			.map(|multiaddr| {
				let e = Encode::encode(&multiaddr.to_string());
				OpaqueMultiaddr::new(e)
			})
			.collect();

		OpaqueNetworkState {
			peer_id,
			external_addresses,
		}
	}
}

impl TryFrom<OpaqueNetworkState> for NetworkState {
	type Error = ();

	fn try_from(state: OpaqueNetworkState) -> Result<Self, Self::Error> {
		let inner_vec = state.peer_id.0;

		let bytes: Vec<u8> = Decode::decode(&mut &inner_vec[..]).map_err(|_| ())?;
		let peer_id = PeerId::from_bytes(&bytes).map_err(|_| ())?;

		let external_addresses: Result<Vec<Multiaddr>, Self::Error> = state.external_addresses
			.iter()
			.map(|enc_multiaddr| -> Result<Multiaddr, Self::Error> {
				let inner_vec = &enc_multiaddr.0;
				let bytes = <Vec<u8>>::decode(&mut &inner_vec[..]).map_err(|_| ())?;
				let multiaddr_str = String::from_utf8(bytes).map_err(|_| ())?;
				let multiaddr = Multiaddr::from_str(&multiaddr_str).map_err(|_| ())?;
				Ok(multiaddr)
			})
			.collect();
		let external_addresses = external_addresses?;

		Ok(NetworkState {
			peer_id,
			external_addresses,
		})
	}
}

/// Offchain extensions implementation API
///
/// This is the asynchronous processing part of the API.
pub(crate) struct AsyncApi {
	/// Everything HTTP-related is handled by a different struct.
	http: Option<http::HttpWorker>,
}

impl AsyncApi {
	/// Creates new Offchain extensions API implementation an the asynchronous processing part.
	pub fn new(
		network_provider: Arc<dyn NetworkProvider + Send + Sync>,
		is_validator: bool,
		shared_client: SharedClient,
	) -> (Api, Self) {
		let (http_api, http_worker) = http::http(shared_client);

		let api = Api {
			network_provider,
			is_validator,
			http: http_api,
		};

		let async_api = Self {
			http: Some(http_worker),
		};

		(api, async_api)
	}

	/// Run a processing task for the API
	pub fn process(mut self) -> impl Future<Output = ()> {
		let http = self.http.take().expect("Take invoked only once.");

		http
	}
}


