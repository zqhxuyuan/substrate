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
