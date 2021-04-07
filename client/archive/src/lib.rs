use std::{
    collections::HashMap, sync::Arc, u64, pin::Pin, borrow::Cow, convert::TryInto,
    time::{Duration, Instant},
};
use sp_core::crypto::Public;
use sp_application_crypto::AppKey;
use sp_keystore::{SyncCryptoStorePtr, SyncCryptoStore};
use sp_runtime::{
    generic::{BlockId, OpaqueDigestItemId}, Justifications,
    traits::{Block as BlockT, Header, DigestItemFor, Zero},
};
use sp_api::{ProvideRuntimeApi, NumberFor};
use parking_lot::Mutex;
use sp_inherents::{InherentDataProviders, InherentData};
use sc_telemetry::{telemetry, TelemetryHandle, CONSENSUS_TRACE, CONSENSUS_DEBUG};
use sp_consensus::{BlockImport, Environment, Proposer, BlockCheckParams, ForkChoiceStrategy, BlockImportParams, BlockOrigin, Error as ConsensusError, SelectChain, SlotData, import_queue::{Verifier, BasicQueue, DefaultImportQueue, CacheKeyId}, ImportResult};
use sc_client_api::{BlockchainEvents, ProvideUncles, LockImportRun, Finalizer, ExecutorProvider, TransactionFor, AuxStore};
use futures::channel::mpsc::{channel, Sender, Receiver, unbounded};
use futures::channel::oneshot;
use sc_client_api::backend::Backend;

use futures::prelude::*;
use log::{debug, info, log, trace, warn};
use prometheus_endpoint::Registry;
use sp_blockchain::{
    Result as ClientResult, Error as ClientError,
    HeaderBackend, ProvideCache, HeaderMetadata
};
use sp_api::ApiExt;
use std::ptr::null;
use std::marker::PhantomData;
use sc_client_api::blockchain::well_known_cache_keys;
use sp_utils::mpsc::{tracing_unbounded, TracingUnboundedSender, TracingUnboundedReceiver};
use futures::SinkExt;
use crate::verifier::PassThroughVerifier;
use sp_consensus::import_queue::BoxBlockImport;

mod verifier;

// /// A block-import handler for Aura.
// pub struct ArchiveBlockImport<Block: BlockT, C> {
//     // inner: I,
//     client: Arc<C>,
//     _phantom: PhantomData<Block>,
// }
//
// impl<Block: BlockT, C> Clone for ArchiveBlockImport<Block, C> {
//     fn clone(&self) -> Self {
//         ArchiveBlockImport {
//             // inner: self.inner.clone(),
//             client: self.client.clone(),
//             _phantom: PhantomData,
//         }
//     }
// }
//
// impl<Block: BlockT, C> ArchiveBlockImport<Block, C> {
//     /// New aura block import.
//     pub fn new(
//         // inner: I,
//         client: Arc<C>,
//     ) -> Self {
//         Self {
//             // inner,
//             client,
//             _phantom: PhantomData,
//         }
//     }
// }
//
// #[async_trait::async_trait]
// impl<Block: BlockT, C> BlockImport<Block> for ArchiveBlockImport<Block, C> where
//     // I: BlockImport<Block, Transaction = sp_api::TransactionFor<C, Block>> + Send + Sync,
//     // I::Error: Into<ConsensusError>,
//     C: HeaderBackend<Block> + ProvideRuntimeApi<Block> + BlockImport<Block, Transaction = sp_api::TransactionFor<C, Block>> + Send + Sync,
//     for<'a> &'a C:
//         BlockImport<Block, Error = ConsensusError, Transaction = TransactionFor<C, Block>>,
//     C::Error: Into<ConsensusError>,
//     sp_api::TransactionFor<C, Block>: Send + 'static,
// {
//     type Error = ConsensusError;
//     type Transaction = sp_api::TransactionFor<C, Block>;
//
//     async fn check_block(
//         &mut self,
//         block: BlockCheckParams<Block>,
//     ) -> Result<ImportResult, Self::Error> {
//         (&*self.client).check_block(block).await.map_err(Into::into)
//     }
//
//     async fn import_block(
//         &mut self,
//         block: BlockImportParams<Block, Self::Transaction>,
//         new_cache: HashMap<CacheKeyId, Vec<u8>>,
//     ) -> Result<ImportResult, Self::Error> {
//         (&*self.client).import_block(block, new_cache).await.map_err(Into::into)
//     }
// }
//
// pub fn block_import<BE, Block: BlockT, Client>(
//     client: Arc<Client>,
//     // spawner: &impl sp_core::traits::SpawnEssentialNamed,
// ) -> ClientResult<ArchiveBlockImport<BE, Block, Client>>
//     where
//         BE: Backend<Block> + 'static,
//         Client: ClientForArchive<Block, BE> + 'static,
// {
//     // let (tx, rx) = tracing_unbounded("mpsc_archive");
//     let mut import = ArchiveBlockImport::new(client.clone());
//     // spawner.spawn_essential("archive-import", Box::pin(import.consume()));
//     Ok(import)
// }

pub fn import_queue<Block: BlockT, Client, Inner>(
    block_import: Inner,
    _: Arc<Client>,
    spawner: &impl sp_core::traits::SpawnEssentialNamed,
) -> ClientResult<DefaultImportQueue<Block, Client>> where
    Inner: BlockImport<Block, Error = ConsensusError, Transaction = sp_api::TransactionFor<Client, Block>> + Send + Sync + 'static,
    Client: ProvideRuntimeApi<Block> + ProvideCache<Block> + HeaderBackend<Block>
            + HeaderMetadata<Block, Error = sp_blockchain::Error>
            + Send + Sync + 'static,
    Client::Api: ApiExt<Block>,
{

    let verifier = PassThroughVerifier::new(true);

    Ok(BasicQueue::new(
        verifier,
        Box::new(block_import),
        None,
        spawner,
        None,
    ))
}

pub fn import_queue2<Block: BlockT, Transaction>(
    inner: BoxBlockImport<Block, Transaction>,
    spawner: &impl sp_core::traits::SpawnEssentialNamed,
) -> ClientResult<BasicQueue<Block, Transaction>> where
    Transaction: Send + Sync + 'static,
{

    let verifier = PassThroughVerifier::new(true);

    Ok(BasicQueue::new(
        verifier,
        inner,
        None,
        spawner,
        None,
    ))
}
