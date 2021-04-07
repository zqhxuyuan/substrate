use sp_consensus::{ForkChoiceStrategy, BlockOrigin, BlockImportParams};
use sp_consensus::import_queue::{Verifier, CacheKeyId};
use sc_client_api::blockchain::well_known_cache_keys;

use sp_runtime::{
    generic::{BlockId, OpaqueDigestItemId}, Justifications,
    traits::{Block as BlockT, Header, DigestItemFor, Zero},
};

#[derive(Clone)]
pub struct PassThroughVerifier {
    finalized: bool,
    fork_choice: ForkChoiceStrategy,
}

impl PassThroughVerifier {
    pub fn new(finalized: bool) -> Self {
        Self {
            finalized,
            fork_choice: ForkChoiceStrategy::LongestChain,
        }
    }
}

#[async_trait::async_trait]
impl<B: BlockT> Verifier<B> for PassThroughVerifier {
    async fn verify(
        &mut self,
        origin: BlockOrigin,
        header: B::Header,
        justifications: Option<Justifications>,
        body: Option<Vec<B::Extrinsic>>
    ) -> Result<(BlockImportParams<B, ()>, Option<Vec<(CacheKeyId, Vec<u8>)>>), String> {
        let maybe_keys = header.digest()
            .log(|l| l.try_as_raw(OpaqueDigestItemId::Consensus(b"aura"))
                .or_else(|| l.try_as_raw(OpaqueDigestItemId::Consensus(b"babe")))
            )
            .map(|blob| vec![(well_known_cache_keys::AUTHORITIES, blob.to_vec())]);
        let mut import = BlockImportParams::new(origin, header);
        import.body = body;
        import.finalized = self.finalized;
        import.justifications = justifications;
        import.fork_choice = Some(self.fork_choice.clone());

        Ok((import, maybe_keys))
    }
}