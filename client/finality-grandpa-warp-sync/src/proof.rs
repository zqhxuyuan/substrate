// Copyright 2021 Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

use codec::{Decode, Encode};

use sc_client_api::Backend as ClientBackend;
use sc_finality_grandpa::{
	find_scheduled_change, AuthoritySetChanges, BlockNumberOps, GrandpaJustification,
};
use sp_blockchain::{Backend as BlockchainBackend, HeaderBackend};
use sp_finality_grandpa::{AuthorityList, SetId, GRANDPA_ENGINE_ID};
use sp_runtime::{
	generic::BlockId,
	traits::{Block as BlockT, Header as HeaderT, NumberFor, One},
};

use crate::HandleRequestError;

/// The maximum number of authority set change proofs to include in a single warp sync proof.
const MAX_CHANGES_PER_WARP_SYNC_PROOF: usize = 256;

/// A proof of an authority set change.
#[derive(Decode, Encode)]
pub struct WarpSyncFragment<Block: BlockT> {
	/// The last block that the given authority set finalized. This block should contain a digest
	/// signaling an authority set change from which we can fetch the next authority set.
	pub header: Block::Header,
	/// A justification for the header above which proves its finality. In order to validate it the
	/// verifier must be aware of the authorities and set id for which the justification refers to.
	pub justification: GrandpaJustification<Block>,
}

/// An accumulated proof of multiple authority set changes.
#[derive(Decode, Encode)]
pub struct WarpSyncProof<Block: BlockT> {
	proofs: Vec<WarpSyncFragment<Block>>,
	is_finished: bool,
}

impl<Block: BlockT> WarpSyncProof<Block> {
	/// Generates a warp sync proof starting at the given block. It will generate authority set
	/// change proofs for all changes that happened from `begin` until the current authority set
	/// (capped by MAX_CHANGES_PER_WARP_SYNC_PROOF).
	pub fn generate<Backend>(
		backend: &Backend,
		begin: Block::Hash,
		set_changes: &AuthoritySetChanges<NumberFor<Block>>,
	) -> Result<WarpSyncProof<Block>, HandleRequestError>
	where
		Backend: ClientBackend<Block>,
	{
		// TODO: cache best response (i.e. the one with lowest begin_number)
		let blockchain = backend.blockchain();

		let begin_number = blockchain
			.block_number_from_id(&BlockId::Hash(begin))?
			.ok_or_else(|| HandleRequestError::InvalidRequest("Missing start block".to_string()))?;

		if begin_number > blockchain.info().finalized_number {
			return Err(HandleRequestError::InvalidRequest(
				"Start block is not finalized".to_string(),
			));
		}

		let canon_hash = blockchain.hash(begin_number)?.expect(
			"begin number is lower than finalized number; \
			 all blocks below finalized number must have been imported; \
			 qed.",
		);

		if canon_hash != begin {
			return Err(HandleRequestError::InvalidRequest(
				"Start block is not in the finalized chain".to_string(),
			));
		}

		let mut proofs = Vec::new();
		let mut proof_limit_reached = false;

		for (_, last_block) in set_changes.iter_from(begin_number) {
			if proofs.len() >= MAX_CHANGES_PER_WARP_SYNC_PROOF {
				proof_limit_reached = true;
				break;
			}

			let header = blockchain.header(BlockId::Number(*last_block))?.expect(
				"header number comes from previously applied set changes; must exist in db; qed.",
			);

			// the last block in a set is the one that triggers a change to the next set,
			// therefore the block must have a digest that signals the authority set change
			if find_scheduled_change::<Block>(&header).is_none() {
				// if it doesn't contain a signal for standard change then the set must have changed
				// through a forced changed, in which case we stop collecting proofs as the chain of
				// trust in authority handoffs was broken.
				break;
			}

			let justification = blockchain
				.justifications(BlockId::Number(*last_block))?
				.and_then(|just| just.into_justification(GRANDPA_ENGINE_ID))
				.expect(
					"header is last in set and contains standard change signal; \
					must have justification; \
					qed.",
				);

			let justification = GrandpaJustification::<Block>::decode(&mut &justification[..])?;

			proofs.push(WarpSyncFragment {
				header: header.clone(),
				justification,
			});
		}

		let is_finished = if proof_limit_reached {
			false
		} else {
			let latest_justification =
				sc_finality_grandpa::best_justification(backend)?.filter(|justification| {
					// the existing best justification must be for a block higher than the
					// last authority set change. if we didn't prove any authority set
					// change then we fallback to make sure it's higher or equal to the
					// initial warp sync block.
					let limit = proofs
						.last()
						.map(|proof| proof.justification.target().0 + One::one())
						.unwrap_or(begin_number);

					justification.target().0 >= limit
				});

			if let Some(latest_justification) = latest_justification {
				let header = blockchain.header(BlockId::Hash(latest_justification.target().1))?
					.expect("header hash corresponds to a justification in db; must exist in db as well; qed.");

				proofs.push(WarpSyncFragment {
					header,
					justification: latest_justification,
				})
			}

			true
		};

		Ok(WarpSyncProof {
			proofs,
			is_finished,
		})
	}

	/// Verifies the warp sync proof starting at the given set id and with the given authorities.
	/// If the proof is valid the new set id and authorities is returned.
	pub fn verify(
		&self,
		set_id: SetId,
		authorities: AuthorityList,
	) -> Result<(SetId, AuthorityList), HandleRequestError>
	where
		NumberFor<Block>: BlockNumberOps,
	{
		let mut current_set_id = set_id;
		let mut current_authorities = authorities;

		for (fragment_num, proof) in self.proofs.iter().enumerate() {
			proof
				.justification
				.verify(current_set_id, &current_authorities)
				.map_err(|err| HandleRequestError::InvalidProof(err.to_string()))?;

			if proof.justification.target().1 != proof.header.hash() {
				return Err(HandleRequestError::InvalidProof(
					"mismatch between header and justification".to_owned()
				));
			}

			if let Some(scheduled_change) = find_scheduled_change::<Block>(&proof.header) {
				current_authorities = scheduled_change.next_authorities;
				current_set_id += 1;
			} else if fragment_num != self.proofs.len() - 1 {
				// Only the last fragment of the proof is allowed to be missing the authority
				// set change.
				return Err(HandleRequestError::InvalidProof(
					"Header is missing authority set change digest".to_string(),
				));
			}
		}

		Ok((current_set_id, current_authorities))
	}
}

