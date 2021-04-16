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

//! RPC API for GRANDPA.
#![warn(missing_docs)]

use std::sync::Arc;
use futures::{FutureExt, TryFutureExt, TryStreamExt, StreamExt};
use log::warn;
use jsonrpc_derive::rpc;
use jsonrpc_pubsub::{typed::Subscriber, SubscriptionId, manager::SubscriptionManager};
use jsonrpc_core::futures::{
	sink::Sink as Sink01,
	stream::Stream as Stream01,
	future::Future as Future01,
	future::Executor as Executor01,
};

mod error;
mod finality;
mod notification;
mod report;

use sc_finality_grandpa::GrandpaJustificationStream;
use sp_runtime::traits::{Block as BlockT, NumberFor};

use finality::{EncodedFinalityProof, RpcFinalityProofProvider};
use report::{ReportAuthoritySet, ReportVoterState, ReportedRoundStates};
use notification::JustificationNotification;

type FutureResult<T> =
	Box<dyn jsonrpc_core::futures::Future<Item = T, Error = jsonrpc_core::Error> + Send>;

/// Provides RPC methods for interacting with GRANDPA.
#[rpc]
pub trait GrandpaApi<Notification, Hash, Number> {
	/// RPC Metadata
	type Metadata;

	/// Returns the state of the current best round state as well as the
	/// ongoing background rounds.
	#[rpc(name = "grandpa_roundState")]
	fn round_state(&self) -> FutureResult<ReportedRoundStates>;

	/// Returns the block most recently finalized by Grandpa, alongside
	/// side its justification.
	#[pubsub(
		subscription = "grandpa_justifications",
		subscribe,
		name = "grandpa_subscribeJustifications"
	)]
	fn subscribe_justifications(
		&self,
		metadata: Self::Metadata,
		subscriber: Subscriber<Notification>
	);

	/// Unsubscribe from receiving notifications about recently finalized blocks.
	#[pubsub(
		subscription = "grandpa_justifications",
		unsubscribe,
		name = "grandpa_unsubscribeJustifications"
	)]
	fn unsubscribe_justifications(
		&self,
		metadata: Option<Self::Metadata>,
		id: SubscriptionId
	) -> jsonrpc_core::Result<bool>;

	/// Prove finality for the given block number by returning the Justification for the last block
	/// in the set and all the intermediary headers to link them together.
	#[rpc(name = "grandpa_proveFinality")]
	fn prove_finality(
		&self,
		block: Number,
	) -> FutureResult<Option<EncodedFinalityProof>>;
}

/// Implements the GrandpaApi RPC trait for interacting with GRANDPA.
pub struct GrandpaRpcHandler<AuthoritySet, VoterState, Block: BlockT, ProofProvider> {
	authority_set: AuthoritySet,
	voter_state: VoterState,
	justification_stream: GrandpaJustificationStream<Block>,
	manager: SubscriptionManager,
	finality_proof_provider: Arc<ProofProvider>,
}

impl<AuthoritySet, VoterState, Block: BlockT, ProofProvider>
	GrandpaRpcHandler<AuthoritySet, VoterState, Block, ProofProvider>
{
	/// Creates a new GrandpaRpcHandler instance.
	pub fn new<E>(
		authority_set: AuthoritySet,
		voter_state: VoterState,
		justification_stream: GrandpaJustificationStream<Block>,
		executor: E,
		finality_proof_provider: Arc<ProofProvider>,
	) -> Self
	where
		E: Executor01<Box<dyn Future01<Item = (), Error = ()> + Send>> + Send + Sync + 'static,
	{
		let manager = SubscriptionManager::new(Arc::new(executor));
		Self {
			authority_set,
			voter_state,
			justification_stream,
			manager,
			finality_proof_provider,
		}
	}
}

impl<AuthoritySet, VoterState, Block, ProofProvider>
	GrandpaApi<JustificationNotification, Block::Hash, NumberFor<Block>>
	for GrandpaRpcHandler<AuthoritySet, VoterState, Block, ProofProvider>
where
	VoterState: ReportVoterState + Send + Sync + 'static,
	AuthoritySet: ReportAuthoritySet + Send + Sync + 'static,
	Block: BlockT,
	ProofProvider: RpcFinalityProofProvider<Block> + Send + Sync + 'static,
{
	type Metadata = sc_rpc::Metadata;

	fn round_state(&self) -> FutureResult<ReportedRoundStates> {
		let round_states = ReportedRoundStates::from(&self.authority_set, &self.voter_state);
		let future = async move { round_states }.boxed();
		Box::new(future.map_err(jsonrpc_core::Error::from).compat())
	}

	fn subscribe_justifications(
		&self,
		_metadata: Self::Metadata,
		subscriber: Subscriber<JustificationNotification>
	) {
		let stream = self.justification_stream.subscribe()
			.map(|x| Ok::<_,()>(JustificationNotification::from(x)))
			.map_err(|e| warn!("Notification stream error: {:?}", e))
			.compat();

		self.manager.add(subscriber, |sink| {
			let stream = stream.map(|res| Ok(res));
			sink.sink_map_err(|e| warn!("Error sending notifications: {:?}", e))
				.send_all(stream)
				.map(|_| ())
		});
	}

	fn unsubscribe_justifications(
		&self,
		_metadata: Option<Self::Metadata>,
		id: SubscriptionId
	) -> jsonrpc_core::Result<bool> {
		Ok(self.manager.cancel(id))
	}

	fn prove_finality(
		&self,
		block: NumberFor<Block>,
	) -> FutureResult<Option<EncodedFinalityProof>> {
		let result = self.finality_proof_provider.rpc_prove_finality(block);
		let future = async move { result }.boxed();
		Box::new(
			future
				.map_err(|e| {
					warn!("Error proving finality: {}", e);
					error::Error::ProveFinalityFailed(e)
				})
				.map_err(jsonrpc_core::Error::from)
				.compat()
		)
	}
}

