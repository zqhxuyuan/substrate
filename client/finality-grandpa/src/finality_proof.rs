// This file is part of Substrate.

// Copyright (C) 2018-2021 Parity Technologies (UK) Ltd.
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

//! GRANDPA block finality proof generation and check.
//!
//! Finality of block B is proved by providing:
//! 1) the justification for the descendant block F;
//! 2) headers sub-chain (B; F] if B != F;
//! 3) proof of GRANDPA::authorities() if the set changes at block F.
//!
//! Since earliest possible justification is returned, the GRANDPA authorities set
//! at the block F is guaranteed to be the same as in the block B (this is because block
//! that enacts new GRANDPA authorities set always comes with justification). It also
//! means that the `set_id` is the same at blocks B and F.
//!
//! Let U be the last finalized block known to caller. If authorities set has changed several
//! times in the (U; F] interval, multiple finality proof fragments are returned (one for each
//! authority set change) and they must be verified in-order.
//!
//! Finality proof provider can choose how to provide finality proof on its own. The incomplete
//! finality proof (that finalizes some block C that is ancestor of the B and descendant
//! of the U) could be returned.

use log::trace;
use std::sync::Arc;

use finality_grandpa::BlockNumberOps;
use parity_scale_codec::{Encode, Decode};
use sp_blockchain::{Backend as BlockchainBackend, Error as ClientError, Result as ClientResult};
use sp_runtime::{
	EncodedJustification, generic::BlockId,
	traits::{NumberFor, Block as BlockT, Header as HeaderT, One},
};
use sc_client_api::backend::Backend;
use sp_finality_grandpa::{AuthorityId, GRANDPA_ENGINE_ID};

use crate::authorities::AuthoritySetChanges;
use crate::justification::GrandpaJustification;
use crate::SharedAuthoritySet;
use crate::VoterSet;

const MAX_UNKNOWN_HEADERS: usize = 100_000;

/// Finality proof provider for serving network requests.
pub struct FinalityProofProvider<BE, Block: BlockT> {
	backend: Arc<BE>,
	shared_authority_set: Option<SharedAuthoritySet<Block::Hash, NumberFor<Block>>>,
}

impl<B, Block: BlockT> FinalityProofProvider<B, Block>
where
	B: Backend<Block> + Send + Sync + 'static,
{
	/// Create new finality proof provider using:
	///
	/// - backend for accessing blockchain data;
	/// - authority_provider for calling and proving runtime methods.
	/// - shared_authority_set for accessing authority set data
	pub fn new(
		backend: Arc<B>,
		shared_authority_set: Option<SharedAuthoritySet<Block::Hash, NumberFor<Block>>>,
	) -> Self {
		FinalityProofProvider {
			backend,
			shared_authority_set,
		}
	}

	/// Create new finality proof provider for the service using:
	///
	/// - backend for accessing blockchain data;
	/// - storage_provider, which is generally a client.
	/// - shared_authority_set for accessing authority set data
	pub fn new_for_service(
		backend: Arc<B>,
		shared_authority_set: Option<SharedAuthoritySet<Block::Hash, NumberFor<Block>>>,
	) -> Arc<Self> {
		Arc::new(Self::new(backend, shared_authority_set))
	}
}

impl<B, Block> FinalityProofProvider<B, Block>
where
	Block: BlockT,
	NumberFor<Block>: BlockNumberOps,
	B: Backend<Block> + Send + Sync + 'static,
{
	/// Prove finality for the given block number by returning a Justification for the last block of
	/// the authority set.
	pub fn prove_finality(
		&self,
		block: NumberFor<Block>
	) -> Result<Option<Vec<u8>>, FinalityProofError> {
		let authority_set_changes = if let Some(changes) = self
			.shared_authority_set
			.as_ref()
			.map(SharedAuthoritySet::authority_set_changes)
		{
			changes
		} else {
			return Ok(None);
		};

		prove_finality::<_, _, GrandpaJustification<Block>>(
			&*self.backend.blockchain(),
			authority_set_changes,
			block,
		)
	}
}

/// Finality for block B is proved by providing:
/// 1) the justification for the descendant block F;
/// 2) headers sub-chain (B; F] if B != F;
#[derive(Debug, PartialEq, Encode, Decode, Clone)]
pub struct FinalityProof<Header: HeaderT> {
	/// The hash of block F for which justification is provided.
	pub block: Header::Hash,
	/// Justification of the block F.
	pub justification: Vec<u8>,
	/// The set of headers in the range (B; F] that we believe are unknown to the caller. Ordered.
	pub unknown_headers: Vec<Header>,
}

/// Errors occurring when trying to prove finality
#[derive(Debug, derive_more::Display, derive_more::From)]
pub enum FinalityProofError {
	/// The requested block has not yet been finalized.
	#[display(fmt = "Block not yet finalized")]
	BlockNotYetFinalized,
	/// The requested block is not covered by authority set changes. Likely this means the block is
	/// in the latest authority set, and the subscription API is more appropriate.
	#[display(fmt = "Block not covered by authority set changes")]
	BlockNotInAuthoritySetChanges,
	/// Errors originating from the client.
	Client(sp_blockchain::Error),
}

fn prove_finality<Block, B, J>(
	blockchain: &B,
	authority_set_changes: AuthoritySetChanges<NumberFor<Block>>,
	block: NumberFor<Block>,
) -> Result<Option<Vec<u8>>, FinalityProofError>
where
	Block: BlockT,
	B: BlockchainBackend<Block>,
	J: ProvableJustification<Block::Header>,
{
	// Early-return if we sure that there are no blocks finalized AFTER begin block
	let info = blockchain.info();
	if info.finalized_number <= block {
		let err = format!(
			"Requested finality proof for descendant of #{} while we only have finalized #{}.",
			block,
			info.finalized_number,
		);
		trace!(target: "afg", "{}", &err);
		return Err(FinalityProofError::BlockNotYetFinalized);
	}

	// Get set_id the block belongs to, and the last block of the set which should contain a
	// Justification we can use to prove the requested block.
	let (_, last_block_for_set) = if let Some(id) = authority_set_changes.get_set_id(block) {
		id
	} else {
		trace!(
			target: "afg",
			"AuthoritySetChanges does not cover the requested block #{}. \
			Maybe the subscription API is more appropriate.",
			block,
		);
		return Err(FinalityProofError::BlockNotInAuthoritySetChanges);
	};

	// Get the Justification stored at the last block of the set
	let last_block_for_set_id = BlockId::Number(last_block_for_set);
	let justification =
		if let Some(grandpa_justification) = blockchain.justifications(last_block_for_set_id)?
			.and_then(|justifications| justifications.into_justification(GRANDPA_ENGINE_ID))
		{
			grandpa_justification
		} else {
			trace!(
				target: "afg",
				"No justification found when making finality proof for {}. Returning empty proof.",
				block,
			);
			return Ok(None);
		};

	// Collect all headers from the requested block until the last block of the set
	let unknown_headers = {
		let mut headers = Vec::new();
		let mut current = block + One::one();
		loop {
			if current >= last_block_for_set || headers.len() >= MAX_UNKNOWN_HEADERS {
				break;
			}
			headers.push(blockchain.expect_header(BlockId::Number(current))?);
			current += One::one();
		}
		headers
	};

	Ok(Some(
		FinalityProof {
			block: blockchain.expect_block_hash_from_id(&last_block_for_set_id)?,
			justification,
			unknown_headers,
		}
		.encode(),
	))
}

/// Check GRANDPA proof-of-finality for the given block.
///
/// Returns the vector of headers that MUST be validated + imported
/// AND if at least one of those headers is invalid, all other MUST be considered invalid.
///
/// This is currently not used, and exists primarily as an example of how to check finality proofs.
#[cfg(test)]
fn check_finality_proof<Header: HeaderT, J>(
	current_set_id: u64,
	current_authorities: sp_finality_grandpa::AuthorityList,
	remote_proof: Vec<u8>,
) -> ClientResult<FinalityProof<Header>>
where
	J: ProvableJustification<Header>,
{
	let proof = FinalityProof::<Header>::decode(&mut &remote_proof[..])
		.map_err(|_| ClientError::BadJustification("failed to decode finality proof".into()))?;

	let justification: J = Decode::decode(&mut &proof.justification[..])
		.map_err(|_| ClientError::JustificationDecode)?;
	justification.verify(current_set_id, &current_authorities)?;

	Ok(proof)
}

/// Justification used to prove block finality.
pub trait ProvableJustification<Header: HeaderT>: Encode + Decode {
	/// Verify justification with respect to authorities set and authorities set id.
	fn verify(&self, set_id: u64, authorities: &[(AuthorityId, u64)]) -> ClientResult<()>;

	/// Decode and verify justification.
	fn decode_and_verify(
		justification: &EncodedJustification,
		set_id: u64,
		authorities: &[(AuthorityId, u64)],
	) -> ClientResult<Self> {
		let justification =
			Self::decode(&mut &**justification).map_err(|_| ClientError::JustificationDecode)?;
		justification.verify(set_id, authorities)?;
		Ok(justification)
	}
}

impl<Block: BlockT> ProvableJustification<Block::Header> for GrandpaJustification<Block>
where
	NumberFor<Block>: BlockNumberOps,
{
	fn verify(&self, set_id: u64, authorities: &[(AuthorityId, u64)]) -> ClientResult<()> {
		let authorities = VoterSet::new(authorities.iter().cloned()).ok_or(
			ClientError::Consensus(sp_consensus::Error::InvalidAuthoritiesSet),
		)?;

		GrandpaJustification::verify_with_voter_set(self, set_id, &authorities)
	}
}
