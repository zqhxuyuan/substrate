use std::{
    marker::PhantomData,
    collections::{HashSet, BTreeMap, HashMap},
    sync::Arc, panic::UnwindSafe, result,
    path::PathBuf
};
use log::{info, trace, warn};
use parking_lot::{Mutex, RwLock};
use codec::{Encode, Decode};
use hash_db::Prefix;
use sp_core::{
    convert_hash,
    storage::{well_known_keys, ChildInfo, PrefixedStorageKey, StorageData, StorageKey},
    ChangesTrieConfiguration, ExecutionContext, NativeOrEncoded,
};
#[cfg(feature="test-helpers")]
use sp_keystore::SyncCryptoStorePtr;
use sc_telemetry::{
    telemetry,
    TelemetryHandle,
    SUBSTRATE_INFO,
};
use sp_runtime::{
    Justification, Justifications, BuildStorage,
    generic::{BlockId, SignedBlock, DigestItem},
    traits::{
        Block as BlockT, Header as HeaderT, Zero, NumberFor,
        HashFor, SaturatedConversion, One, DigestFor, UniqueSaturatedInto,
    },
};
use sp_state_machine::{DBValue, Backend as StateBackend, ChangesTrieAnchorBlockId, prove_read, prove_child_read, ChangesTrieRootsStorage, ChangesTrieStorage, ChangesTrieConfigurationRange, key_changes, key_changes_proof, Backend};
use sc_executor::RuntimeVersion;
use sp_consensus::{
    Error as ConsensusError, BlockStatus, BlockImportParams, BlockCheckParams,
    ImportResult, BlockOrigin, ForkChoiceStrategy,
};
use sp_blockchain::{
    self as blockchain,
    Backend as ChainBackend,
    HeaderBackend as ChainHeaderBackend, ProvideCache, Cache,
    well_known_cache_keys::Id as CacheKeyId,
    HeaderMetadata, CachedHeaderMetadata,
};
use sp_trie::StorageProof;
use sp_api::{CallApiAt, ConstructRuntimeApi, Core as CoreApi, ApiExt, ApiRef, ProvideRuntimeApi, CallApiAtParams, TransactionOutcome, RuntimeApiInfo, StorageChanges, ApiError};
use sc_block_builder::{BlockBuilderApi, BlockBuilderProvider, RecordProof};
use sc_client_api::{
    backend::{
        self, BlockImportOperation, PrunableStateChangesTrieStorage,
        ClientImportOperation, Finalizer, ImportSummary, NewBlockState,
        changes_tries_state_at_block, StorageProvider,
        LockImportRun, apply_aux,
    },
    client::{
        ImportNotifications, FinalityNotification, FinalityNotifications, BlockImportNotification,
        ClientInfo, BlockchainEvents, BlockBackend, ProvideUncles, BadBlocks, ForkBlocks,
        BlockOf,
    },
    execution_extensions::ExecutionExtensions,
    notifications::{StorageNotifications, StorageEventStream},
    KeyIterator, CallExecutor, ExecutorProvider, ProofProvider,
    cht, UsageProvider
};
use sp_utils::mpsc::{TracingUnboundedSender, tracing_unbounded};
use sp_blockchain::Error;
use prometheus_endpoint::Registry;
use super::{
    genesis, block_rules::{BlockRules, LookupResult as BlockLookupResult},
};
use sc_light::{call_executor::prove_execution, fetcher::ChangesProof};
use rand::Rng;

#[cfg(feature="test-helpers")]
use {
    sp_core::traits::{CodeExecutor, SpawnNamed},
    sc_client_api::in_mem,
    sc_executor::RuntimeInfo,
    super::call_executor::LocalCallExecutor,
};
// use sp_runtime::traits::Block;
// use sp_consensus_babe::AuthorityId;
use sp_authority_discovery::{AuthorityDiscoveryApi, AuthorityId};
use sp_runtime::traits::Block;
pub use sp_state_machine::ChangesTrieState;
use sp_runtime::transaction_validity::{TransactionValidity, TransactionSource, TransactionPriority};
// use sp_api::*;

// construct_runtime!(
// 	pub enum MockRuntimeAPi where
// 		Block = Block,
// 		NodeBlock = node_primitives::Block,
// 		UncheckedExtrinsic = UncheckedExtrinsic
// 	{
// 		System: frame_system::{Pallet, Call, Config, Storage, Event<T>},
// 		Utility: pallet_utility::{Pallet, Call, Event},
// 	}
// );

pub struct MockRuntimeAPi<Block> {
    pub(crate) _ph: PhantomData<Block>
}

impl<Block> ApiExt<Block> for MockRuntimeAPi<Block> where Block: BlockT {
    type StateBackend = sp_api::InMemoryBackend<sp_api::HashFor<Block>>;

    fn execute_in_transaction<F: FnOnce(&Self) -> TransactionOutcome<R>, R>(&self, call: F) -> R where Self: Sized {
        unimplemented!()
    }

    fn has_api<A: RuntimeApiInfo + ?Sized>(&self, at: &BlockId<Block>) -> Result<bool, ApiError> where Self: Sized {
        unimplemented!()
    }

    fn has_api_with<A: RuntimeApiInfo + ?Sized, P: Fn(u32) -> bool>(&self, at: &BlockId<Block>, pred: P) -> Result<bool, ApiError> where Self: Sized {
        unimplemented!()
    }

    fn record_proof(&mut self) {
        unimplemented!()
    }

    fn extract_proof(&mut self) -> Option<StorageProof> {
        unimplemented!()
    }

    fn into_storage_changes(&self, backend: &Self::StateBackend,
                            // changes_trie_state: Option<&State<'a, _, _>>, parent_hash: <Block as BlockT>::Hash
                            changes_trie_state: Option<&ChangesTrieState<HashFor<Block>, NumberFor<Block>>>,
                            parent_hash: Block::Hash
    )
                            -> Result<StorageChanges<Self::StateBackend, Block>, String> where Self: Sized {
        unimplemented!()
    }
}

// use sp_core::OpaqueMetadata;
// sp_api::mock_impl_runtime_apis! {
// 	impl AuthorityDiscoveryApi<Block> for MockRuntimeAPi {
// 		fn authorities(&self) -> Vec<AuthorityId> {
// 			self.authorities.clone()
// 		}
// 	}
// 	impl sp_api::Metadata<BlockT> for MockRuntimeAPi {
// 		fn metadata() -> OpaqueMetadata {
// 			unimplemented!()
// 		}
// 	}
// }

// use sp_api::impl_runtime_apis;
//
// impl_runtime_apis! {
// 	impl<Block> sp_api::Core<Block> for MockRuntimeAPi<Block> {
// 		fn version() -> RuntimeVersion {
// 			unimplemented!()
// 		}
//
// 		fn execute_block(block: Block) {
// 			unimplemented!()
// 		}
//
// 		fn initialize_block(header: &<Block as BlockT>::Header) {
// 			unimplemented!()
// 		}
// 	}
//
// 	impl<Block> sp_api::Metadata<Block> for MockRuntimeAPi<Block> {
// 		fn metadata() -> OpaqueMetadata {
// 			unimplemented!()
// 		}
// 	}
//
// 	impl<Block> sp_block_builder::BlockBuilder<Block> for MockRuntimeAPi<Block> {
// 		fn apply_extrinsic(extrinsic: <Block as BlockT>::Extrinsic) -> ApplyExtrinsicResult {
// 			unimplemented!()
// 		}
//
// 		fn finalize_block() -> <Block as BlockT>::Header {
// 			unimplemented!()
// 		}
//
// 		fn inherent_extrinsics(data: InherentData) -> Vec<<Block as BlockT>::Extrinsic> {
// 			unimplemented!()
// 		}
//
// 		fn check_inherents(block: Block, data: InherentData) -> CheckInherentsResult {
// 			unimplemented!()
// 		}
//
// 		fn random_seed() -> <Block as BlockT>::Hash {
// 			unimplemented!()
// 		}
// 	}
//
// 	impl<Block> sp_transaction_pool::runtime_api::TaggedTransactionQueue<Block> for MockRuntimeAPi<Block> {
// 		fn validate_transaction(
// 			source: TransactionSource,
// 			tx: <Block as BlockT>::Extrinsic,
// 		) -> TransactionValidity {
// 			unimplemented!()
// 		}
// 	}
// }

// impl<Block> sp_transaction_pool::runtime_api::TaggedTransactionQueue<Block> for MockRuntimeAPi<Block> where Block: BlockT {
//     fn validate_transaction(&mut self,
//         source: TransactionSource,
//         tx: <Block as BlockT>::Extrinsic,
//     ) -> TransactionValidity {
//         unimplemented!()
//     }
// }
//
// impl<Block> sp_api::Core<Block> for MockRuntimeAPi<Block> where Block: BlockT {
//     fn version(&mut self) -> RuntimeVersion {
//         unimplemented!()
//     }
//
//     fn execute_block(&mut self,block: Block) {
//         unimplemented!()
//     }
//
//     fn initialize_block(&mut self, header: &<Block as BlockT>::Header) {
//         unimplemented!()
//     }
// }
//
// impl<Block> sp_api::Metadata<Block> for MockRuntimeAPi<Block> where Block: BlockT {
//     fn metadata(&mut self) -> sp_core::OpaqueMetadata {
//         unimplemented!()
//     }
// }