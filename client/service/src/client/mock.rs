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
use sp_core::{convert_hash, storage::{well_known_keys, ChildInfo, PrefixedStorageKey, StorageData, StorageKey}, ChangesTrieConfiguration, ExecutionContext, NativeOrEncoded, OpaqueMetadata};
#[cfg(feature="test-helpers")]
use sp_keystore::SyncCryptoStorePtr;
use sc_telemetry::{
    telemetry,
    TelemetryHandle,
    SUBSTRATE_INFO,
};
use sp_runtime::{Justification, Justifications, BuildStorage, generic::{BlockId, SignedBlock, DigestItem}, traits::{
    Block as BlockT, Header as HeaderT, Zero, NumberFor,
    HashFor, SaturatedConversion, One, DigestFor, UniqueSaturatedInto,
}, ApplyExtrinsicResult, DispatchError, KeyTypeId};
use sp_state_machine::{DBValue, Backend as StateBackend, ChangesTrieAnchorBlockId, prove_read, prove_child_read, ChangesTrieRootsStorage, ChangesTrieStorage, ChangesTrieConfigurationRange, key_changes, key_changes_proof, Backend, OverlayedChanges, TrieBackendStorage, StorageValue, StateMachineStats, UsageInfo};
use sc_executor::{RuntimeVersion, Codec};
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
use sp_api::{CallApiAt, ConstructRuntimeApi, Core as CoreApi, ApiExt, ApiRef, ProvideRuntimeApi, CallApiAtParams, TransactionOutcome, RuntimeApiInfo, StorageChanges, ApiError, InitializeBlock, StorageTransactionCache};
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

use sp_runtime::traits::{Block, BlindCheckable, Applyable, Dispatchable, Checkable, ValidateUnsigned};
pub use sp_state_machine::ChangesTrieState;
use sp_runtime::transaction_validity::{TransactionValidity, TransactionSource, TransactionPriority, TransactionValidityError};
use sp_inherents::{InherentData, CheckInherentsResult};
// use sp_api::*;
use pallet_grandpa::{AuthorityId as GrandpaId, AuthorityList as GrandpaAuthorityList};
use pallet_grandpa::fg_primitives;
use sp_consensus_babe::{BabeGenesisConfiguration, Epoch, Slot};
use sc_client_db::RefTrackingState;
// use grandpa_primitives::{AuthorityId as GrandpaId, AuthorityList as GrandpaAuthorityList};
// use pallet_grandpa::fg_primitives;
// use pallet_babe;
// use crate::storage_cache::{CachingState, SyncingCachingState, SharedCache, new_shared_cache};
use sc_client_db::storage_cache::SyncingCachingState;
use frame_executive::{Executive, CheckedOf, CallOf};
use frame_support::{
    weights::{GetDispatchInfo, DispatchInfo, DispatchClass},
    traits::{OnInitialize, OnIdle, OnFinalize, OnRuntimeUpgrade, OffchainWorker, ExecuteBlock},
    dispatch::PostDispatchInfo,
};
use crate::client::Client;
use std::cell::RefCell;
use sc_client_api::StateBackend as CStateBackend;
use memory_db::{MemoryDB, PrefixedKey};
use frame_support::sp_runtime::app_crypto::sp_core::Hasher;
use sp_state_machine::backend::Consolidate;

pub type Moment = u64;
pub type BlockNumber = u32;

pub const MILLISECS_PER_BLOCK: Moment = 3000;
pub const SECS_PER_BLOCK: Moment = MILLISECS_PER_BLOCK / 1000;
pub const SLOT_DURATION: Moment = MILLISECS_PER_BLOCK;
// 1 in 4 blocks (on average, not counting collisions) will be primary BABE blocks.
pub const PRIMARY_PROBABILITY: (u64, u64) = (1, 4);
pub const EPOCH_DURATION_IN_BLOCKS: BlockNumber = 10 * MINUTES;
pub const EPOCH_DURATION_IN_SLOTS: u64 = {
    const SLOT_FILL_RATE: f64 = MILLISECS_PER_BLOCK as f64 / SLOT_DURATION as f64;
    (EPOCH_DURATION_IN_BLOCKS as f64 * SLOT_FILL_RATE) as u64
};
// These time units are defined in number of blocks.
pub const MINUTES: BlockNumber = 60 / (SECS_PER_BLOCK as BlockNumber);
pub const HOURS: BlockNumber = MINUTES * 60;
pub const DAYS: BlockNumber = HOURS * 24;

pub const EpochDuration: u64 = EPOCH_DURATION_IN_SLOTS;
/// The BABE epoch configuration at genesis.
pub const BABE_GENESIS_EPOCH_CONFIG: sp_consensus_babe::BabeEpochConfiguration =
    sp_consensus_babe::BabeEpochConfiguration {
        c: PRIMARY_PROBABILITY,
        allowed_slots: sp_consensus_babe::AllowedSlots::PrimaryAndSecondaryPlainSlots
    };

// types
pub type Babe<C,B> = pallet_babe::Pallet<MockRuntimeAPi<C,B>>;
// pub type AuthorityDiscovery<C,B> = pallet_authority_discovery::Pallet<MockRuntimeAPi<C,B>>;

pub struct MockRuntimeAPi<C: 'static , Block> where
    Block: BlockT,
    C: CallApiAt<Block> + 'static + Send + Sync,
    C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>,
{
    pub call: &'static C,
    pub initialized_block: RefCell<Option<BlockId<Block>>>,
    pub changes: RefCell<OverlayedChanges>,
    pub storage: RefCell<StorageTransactionCache<Block, C::StateBackend>>,
    pub(crate) _ph: PhantomData<Block>
}

unsafe impl<C, Block> Sync for MockRuntimeAPi<C, Block> where
    Block: BlockT,
    C: CallApiAt<Block> + 'static + Send + Sync,
    C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>,
{
}

unsafe impl<C, Block> Send for MockRuntimeAPi<C, Block> where
    Block: BlockT,
    C: CallApiAt<Block> + 'static + Send + Sync,
    C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>,
{
}

impl<C, Block> MockRuntimeAPi<C,Block> where
    Block: BlockT,
    C: CallApiAt<Block> + 'static + Send + Sync,
    C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
{
    fn commit_or_rollback(&self, commit: bool) {
        let proof = "\
					We only close a transaction when we opened one ourself.
					Other parts of the runtime that make use of transactions (state-machine)
					also balance their transactions. The runtime cannot close client initiated
					transactions. qed";
        // if *self.commit_on_success.borrow() {
        if commit {
            self.changes.borrow_mut().commit_transaction().expect(proof);
        } else {
            self.changes.borrow_mut().rollback_transaction().expect(proof);
        }
        // }
    }

    // fn _call_api_at<T>(&self, method: &str, init_block: bool, at: &BlockId<Block>, tuple: &()) -> Result<T, sp_api::ApiError> {
    //     self.changes.borrow_mut().start_transaction();
    //     let runtime_api_impl_params_encoded = sp_api::Encode::encode(tuple);
    //     let initialize_block = if init_block {
    //         InitializeBlock::Skip
    //     } else {
    //         InitializeBlock::Do(&self.initialized_block)
    //     };
    //     let call_params: CallApiAtParams<'_, Block, MockRuntimeAPi<C, Block>, fn() -> Result<T, sp_api::ApiError>, <C as CallApiAt<Block>>::StateBackend>
    //         = CallApiAtParams {
    //         core_api: self.clone(),
    //         at: at,
    //         function: method,
    //         native_call: None,
    //         arguments: runtime_api_impl_params_encoded,
    //         overlayed_changes: &self.changes,
    //         storage_transaction_cache: &self.storage,
    //         initialize_block: initialize_block,
    //         context: ExecutionContext::OffchainCall(None),
    //         recorder: &None
    //     };
    //     let res = self.call.call_api_at(call_params);
    //     if res.is_ok() {
    //         self.changes.borrow_mut().commit_transaction().expect("commit error!");
    //     } else {
    //         self.changes.borrow_mut().rollback_transaction().expect("rollback error!");
    //     }
    //     let res = res.and_then(|r| match r {
    //         sp_api::NativeOrEncoded::Native(n) => Ok(n),
    //         sp_api::NativeOrEncoded::Encoded(r) =>
    //             < T as sp_api::Decode>::decode (& mut & r [..])
    //                 .map_err (| err | sp_api :: ApiError :: FailedToDecodeReturnValue {
    //                     function : method ,
    //                     error : err , }) });
    //     res
    // }
}

impl<C, Block> ApiExt<Block> for MockRuntimeAPi<C,Block> where
    Block: BlockT,
    C: CallApiAt<Block> + 'static + Send + Sync,
    C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
{
    // type StateBackend = dyn sp_api::StateBackend<sp_api::HashFor<Block>>;
    // type StateBackend = sp_api::InMemoryBackend<sp_api::HashFor<Block>>;
    // type StateBackend = SyncingCachingState<RefTrackingState<Block>, Block>;
    type StateBackend = C::StateBackend;

    fn execute_in_transaction<F: FnOnce(&Self) -> TransactionOutcome<R>, R>(&self, call: F) -> R where Self: Sized {
        info!("call execute_in_transaction");
        // unimplemented!()
        self.changes.borrow_mut().start_transaction();
        //TODO add commit_on_success flag to MockRuntimeAPi
        // *self.commit_on_success.borrow_mut() = false;
        let res = call(self);
        // *self.commit_on_success.borrow_mut() = true;
        self.commit_or_rollback(match res {
            sp_api::TransactionOutcome::Commit(_,) => true,
            _ => false,
        });
        res.into_inner()
    }

    fn has_api<A: RuntimeApiInfo + ?Sized>(&self, at: &BlockId<Block>) -> Result<bool, ApiError> where Self: Sized {
        // unimplemented!()
        info!("call has_api");
        // Ok(true)
        self.call.runtime_version_at(at)
            .map(|v| v.has_api_with(&A::ID, |v| v == A::VERSION))
    }

    fn has_api_with<A: RuntimeApiInfo + ?Sized, P: Fn(u32) -> bool>(&self, at: &BlockId<Block>, pred: P) -> Result<bool, ApiError> where Self: Sized {
        // unimplemented!()
        info!("call has_api_with");
        // Ok(true)
        self.call.runtime_version_at(at)
            .map(|v| v.has_api_with(&A::ID, pred))
    }

    fn record_proof(&mut self) {
        info!("call record_proof");
        unimplemented!()
    }

    fn extract_proof(&mut self) -> Option<StorageProof> {
        info!("call extract_proof");
        unimplemented!()
    }

    // block import prepare_block_storage_changes() will invoke this method after execute_block()
    fn into_storage_changes(&self,
                            backend: &Self::StateBackend,
                            changes_trie_state: Option<&ChangesTrieState<HashFor<Block>, NumberFor<Block>>>,
                            parent_hash: <Block as BlockT>::Hash
    ) -> Result<StorageChanges<Self::StateBackend, Block>, String> where Self: Sized {
        // info!("call into_storage_changes");
        self.initialized_block.borrow_mut().take();
        self.changes
            .replace(Default::default())
            .into_storage_changes(
                backend,
                changes_trie_state,
                parent_hash,
                self.storage.replace(Default::default()),
            )
    }
}

const ID: [u8; 8] = [223u8, 106u8, 203u8, 104u8, 153u8, 7u8, 96u8, 155u8];

// impl<Block, UnsignedValidator, Context: Default> sp_api::Core<Block> for MockRuntimeAPi<Block> where
impl<C,Block> sp_api::Core<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>,
{
    fn version(&self, at: &BlockId<Block>) -> Result<RuntimeVersion, sp_api::ApiError> {
        // VERSION
        info!("call version");
        unimplemented!()
    }
    fn execute_block(&self, at: &BlockId<Block>, block: Block) -> Result<(), sp_api::ApiError> {
        info!("call execute_block");
        unimplemented!()
    }
    fn initialize_block(&self, at: &BlockId<Block>, header: &<Block as BlockT>::Header) -> Result<(), sp_api::ApiError> {
        info!("call initialize_block");
        // unimplemented!()
        self.changes.borrow_mut().start_transaction();
        let version = self.call.runtime_version_at(at)?;
        let runtime_api_impl_params_encoded = sp_api::Encode::encode(&(&header));
        let initialize_block = if true {
            InitializeBlock::Skip
        } else {
            InitializeBlock::Do(&self.initialized_block)
        };
        let method = if version.apis.iter().any(|(s, v)| s == &ID && *v < 2u32) {
            "Core_initialise_block"
        } else {
            "Core_initialize_block"
        };
        let call_params: CallApiAtParams<'_, Block, MockRuntimeAPi<C, Block>, fn() -> Result<(), sp_api::ApiError>, <C as CallApiAt<Block>>::StateBackend>
            = CallApiAtParams {
            core_api: self.clone(),
            at: at,
            function: method,
            native_call: None,
            arguments: runtime_api_impl_params_encoded,
            overlayed_changes: &self.changes,
            storage_transaction_cache: &self.storage,
            initialize_block: initialize_block,
            context: ExecutionContext::OffchainCall(None),
            recorder: &None
        };
        let res = self.call.call_api_at(call_params);
        if res.is_ok() {
            self.changes.borrow_mut().commit_transaction().expect("commit error!");
        } else {
            self.changes.borrow_mut().rollback_transaction().expect("rollback error!");
        }
        let res = res.and_then(|r| match r {
            sp_api::NativeOrEncoded::Native(n) => Ok(n),
            sp_api::NativeOrEncoded::Encoded(r) =>
                < () as sp_api::Decode>::decode (& mut & r [..])
                    .map_err (| err | sp_api :: ApiError :: FailedToDecodeReturnValue {
                        function : "Core_initialise_block" ,
                        error : err , }) });
        res
    }
    fn Core_version_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
                                     -> std::result::Result<NativeOrEncoded<RuntimeVersion>, sp_api::ApiError> {
        info!("call Core_version");
        unimplemented!() }
    fn Core_execute_block_runtime_api_impl(&self, at: &BlockId<Block>, context: ExecutionContext, params: std::option::Option<Block>, _: Vec<u8>)
                                           -> std::result::Result<NativeOrEncoded<()>, sp_api::ApiError> {
        // info!("call Core_execute_block_runtime_api_impl");
        // Ok(NativeOrEncoded::Encoded(vec![]))
        // unimplemented!()
        self.changes.borrow_mut().start_transaction();
        let runtime_api_impl_params_encoded = sp_api::Encode::encode(&(&params.unwrap()));
        let _initialize_block = if true {
            InitializeBlock::Skip
        } else {
            InitializeBlock::Do(&self.initialized_block)
        };
        let call_params: CallApiAtParams<'_, Block, MockRuntimeAPi<C, Block>, fn() -> Result<(), sp_api::ApiError>, <C as CallApiAt<Block>>::StateBackend>
            = CallApiAtParams {
            core_api: self.clone(),  // core_api is MockRuntimeApi, because MockRuntimeApi implements triat Core
            at: at,
            function: "Core_execute_block",
            native_call: None, //Some(|| -> Result<(), sp_api::ApiError>{ Ok(())}),
            arguments: runtime_api_impl_params_encoded,
            overlayed_changes: &self.changes,
            storage_transaction_cache: &self.storage,
            initialize_block: _initialize_block,
            context,
            recorder: &None
        };
        // self.call is Client, because Client implements trait CallApiAt
        let res = self.call.call_api_at(call_params);
        if res.is_ok() {
            self.changes.borrow_mut().commit_transaction().expect("commit error!");
        } else {
            self.changes.borrow_mut().rollback_transaction().expect("rollback error!");
        }
        res
    }
    fn Core_initialize_block_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<&<Block as BlockT>::Header>, _: Vec<u8>)
                                              -> std::result::Result<NativeOrEncoded<()>, sp_api::ApiError> {
        info!("call Core_initialize_block");
        unimplemented!()
    }
}
impl<C,Block> sp_api::Metadata<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
{
    fn metadata(&self, at: &BlockId<Block>) -> Result<sp_core::OpaqueMetadata, sp_api::ApiError> {
        // Runtime::metadata().into()
        info!("call metadata");
        // unimplemented!()
        self.changes.borrow_mut().start_transaction();
        let runtime_api_impl_params_encoded = sp_api::Encode::encode(&());
        let initialize_block = if true {
            InitializeBlock::Skip
        } else {
            InitializeBlock::Do(&self.initialized_block)
        };
        let call_params: CallApiAtParams<'_, Block, MockRuntimeAPi<C, Block>, fn() -> Result<OpaqueMetadata, sp_api::ApiError>, <C as CallApiAt<Block>>::StateBackend>
            = CallApiAtParams {
            core_api: self.clone(),
            at: at,
            function: "Metadata_metadata",
            native_call: None,
            arguments: runtime_api_impl_params_encoded,
            overlayed_changes: &self.changes,
            storage_transaction_cache: &self.storage,
            initialize_block: initialize_block,
            context: ExecutionContext::OffchainCall(None),
            recorder: &None
        };
        let res = self.call.call_api_at(call_params);
        if res.is_ok() {
            self.changes.borrow_mut().commit_transaction().expect("commit error!");
        } else {
            self.changes.borrow_mut().rollback_transaction().expect("rollback error!");
        }
        let res = res.and_then(|r| match r {
            sp_api::NativeOrEncoded::Native(n) => Ok(n),
            sp_api::NativeOrEncoded::Encoded(r) =>
                < OpaqueMetadata as sp_api::Decode>::decode (& mut & r [..])
                    .map_err (| err | sp_api :: ApiError :: FailedToDecodeReturnValue {
                        function : "Metadata_metadata" ,
                        error : err , }) });
        res
    }
    fn Metadata_metadata_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
                                          -> std::result::Result<NativeOrEncoded<OpaqueMetadata>, sp_api::ApiError> {
        info!("call Metadata_metadata_runtime_api_impl");
        unimplemented!() }
}

impl<C,Block> sp_block_builder::BlockBuilder<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
{
    fn apply_extrinsic(&self, at: &BlockId<Block>, extrinsic: <Block as BlockT>::Extrinsic) -> Result<ApplyExtrinsicResult, sp_api::ApiError> {
        // Executive::apply_extrinsic(extrinsic)
        info!("call apply_extrinsic");
        unimplemented!()
    }
    fn finalize_block(&self, at: &BlockId<Block>) -> Result<<Block as BlockT>::Header,sp_api::ApiError> {
        // Executive::finalize_block()
        info!("call finalize_block");
        unimplemented!()
    }
    fn inherent_extrinsics(&self, at: &BlockId<Block>, data: InherentData) -> Result<Vec<<Block as BlockT>::Extrinsic>, sp_api::ApiError> {
        // data.create_extrinsics()
        info!("call inherent_extrinsics");
        unimplemented!()
    }
    fn check_inherents(&self, at: &BlockId<Block>, block: Block, data: InherentData) -> Result<CheckInherentsResult,sp_api::ApiError> {
        // data.check_extrinsics(&block)
        // info!("call check_inherents");
        // unimplemented!()
        self.changes.borrow_mut().start_transaction();
        let runtime_api_impl_params_encoded = sp_api::Encode::encode(&(&block, &data));
        let initialize_block = if true {
            InitializeBlock::Skip
        } else {
            InitializeBlock::Do(&self.initialized_block)
        };
        let call_params: CallApiAtParams<'_, Block, MockRuntimeAPi<C, Block>, fn() -> Result<CheckInherentsResult, sp_api::ApiError>, <C as CallApiAt<Block>>::StateBackend>
            = CallApiAtParams {
            core_api: self.clone(),
            at: at,
            function: "BlockBuilder_check_inherents",
            native_call: None,
            arguments: runtime_api_impl_params_encoded,
            overlayed_changes: &self.changes,
            storage_transaction_cache: &self.storage,
            initialize_block: initialize_block,
            context: ExecutionContext::OffchainCall(None),
            recorder: &None
        };
        let res = self.call.call_api_at(call_params);
        if res.is_ok() {
            self.changes.borrow_mut().commit_transaction().expect("commit error!");
        } else {
            self.changes.borrow_mut().rollback_transaction().expect("rollback error!");
        }
        let res = res.and_then(|r| match r {
            sp_api::NativeOrEncoded::Native(n) => Ok(n),
            sp_api::NativeOrEncoded::Encoded(r) =>
                < CheckInherentsResult as sp_api::Decode>::decode (& mut & r [..])
                    .map_err (| err | sp_api :: ApiError :: FailedToDecodeReturnValue {
                        function : "BlockBuilder_check_inherents" ,
                        error : err , }) });
        res
    }
    fn random_seed(&self, at: &BlockId<Block>) -> Result<<Block as BlockT>::Hash, sp_api::ApiError> {
        // pallet_babe::RandomnessFromOneEpochAgo::<Runtime>::random_seed().0
        info!("call random_seed");
        unimplemented!()
    }

    fn BlockBuilder_apply_extrinsic_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<<Block as BlockT>::Extrinsic>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<std::result::Result<std::result::Result<(), DispatchError>, TransactionValidityError>>, sp_api::ApiError> {
        info!("call BlockBuilder_apply_extrinsic_runtime_api_impl");
        unimplemented!()
    }
    fn BlockBuilder_finalize_block_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<<Block as BlockT>::Header>, sp_api::ApiError> {
        info!("call BlockBuilder_finalize_block_runtime_api_impl");
        unimplemented!()
    }
    fn BlockBuilder_inherent_extrinsics_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<sp_consensus::InherentData>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<Vec<<Block as BlockT>::Extrinsic>>, sp_api::ApiError> {
        info!("call BlockBuilder_inherent_extrinsics_runtime_api_impl");
        unimplemented!()
    }
    fn BlockBuilder_check_inherents_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<(Block, sp_consensus::InherentData)>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<CheckInherentsResult>, sp_api::ApiError> {
        info!("call BlockBuilder_check_inherents_runtime_api_impl");
        unimplemented!()
    }
    fn BlockBuilder_random_seed_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<<Block as BlockT>::Hash>, sp_api::ApiError> {
        info!("call BlockBuilder_random_seed_runtime_api_impl");
        unimplemented!()
    }
}

impl<C,Block> fg_primitives::GrandpaApi<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
{
    fn grandpa_authorities(&self, _: &BlockId<Block>,) -> Result<GrandpaAuthorityList, sp_api::ApiError> {
        // Grandpa::grandpa_authorities()
        info!("call grandpa_authorities");
        unimplemented!()
    }
    fn submit_report_equivocation_unsigned_extrinsic(&self, _: &BlockId<Block>,
        equivocation_proof: fg_primitives::EquivocationProof<
            <Block as BlockT>::Hash,
            NumberFor<Block>,
        >,
        key_owner_proof: fg_primitives::OpaqueKeyOwnershipProof,
    ) -> Result<Option<()>, sp_api::ApiError> {
        // let key_owner_proof = key_owner_proof.decode()?;
        // Grandpa::submit_unsigned_equivocation_report(equivocation_proof, key_owner_proof)
        info!("call submit_report_equivocation_unsigned_extrinsic");
        unimplemented!()
    }
    fn generate_key_ownership_proof(&self, _: &BlockId<Block>,
        _set_id: fg_primitives::SetId,
        authority_id: GrandpaId,
    ) -> Result<Option<fg_primitives::OpaqueKeyOwnershipProof>, sp_api::ApiError> {
        // use codec::Encode;
        info!("call generate_key_ownership_proof");
        unimplemented!()
    }
    fn GrandpaApi_grandpa_authorities_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<Vec<(fg_primitives::AuthorityId, u64)>>, sp_api::ApiError> {
        info!("call GrandpaApi_grandpa_authorities_runtime_api_impl");
        todo!() }
    fn GrandpaApi_submit_report_equivocation_unsigned_extrinsic_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext,
        _: std::option::Option<(fg_primitives::EquivocationProof<<Block as BlockT>::Hash, <<Block as BlockT>::Header as HeaderT>::Number>,
                                // pallet_grandpa::sp_finality_grandpa::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
                                fg_primitives::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
                                // pallet_grandpa::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<std::option::Option<()>>, sp_api::ApiError> {
        info!("call GrandpaApi_submit_report_equivocation_unsigned_extrinsic_runtime_api_impl");
        todo!() }
    fn GrandpaApi_generate_key_ownership_proof_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<(u64, fg_primitives::AuthorityId)>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<std::option::Option<fg_primitives::OpaqueKeyOwnershipProof>>, sp_api::ApiError> {
        info!("call GrandpaApi_generate_key_ownership_proof_runtime_api_impl");
        todo!() }
}

impl<C,Block> sp_authority_discovery::AuthorityDiscoveryApi<Block> for MockRuntimeAPi<C,Block> where
    Block: BlockT,
    C: CallApiAt<Block> + 'static + Send + Sync,
    C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
{
    fn authorities(&self, _: &BlockId<Block>,) -> Result<Vec<sp_authority_discovery::AuthorityId>, sp_api::ApiError> {
        // AuthorityDiscovery::authorities()
        info!("call authorities");
        unimplemented!()
    }
    fn AuthorityDiscoveryApi_authorities_runtime_api_impl(&self, at: &BlockId<Block>, contxt: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<Vec<sp_authority_discovery::AuthorityId>>, sp_api::ApiError> {
        info!("call AuthorityDiscoveryApi_authorities_runtime_api_impl");

        todo!()
    }
}

impl<C,Block> sp_consensus_babe::BabeApi<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
{
    fn configuration(&self, at: &BlockId<Block>,) -> Result<BabeGenesisConfiguration, sp_api::ApiError> {
        info!("call BabeApi configuration");
        // let _config = BabeGenesisConfiguration {
        //     slot_duration: 1000,
        //     epoch_length: 100,
        //     c: (3, 10),
        //     // Verification failed for block 0xc0096358534ec8d21d01d34b836eed476a1c343f8724fa2153dc0725ad797a90 received from peer: , "Slot author not found"
        //     genesis_authorities: vec![],
        //     // function or associated item cannot be called on `Pallet<MockRuntimeAPi<_, _>>` due to unsatisfied trait bounds
        //     // genesis_authorities: Babe::authorities(),
        //     randomness: [0; 32],
        //     allowed_slots: BABE_GENESIS_EPOCH_CONFIG.allowed_slots,
        // };
        // Ok(_config)
        self.changes.borrow_mut().start_transaction();
        let runtime_api_impl_params_encoded = sp_api::Encode::encode(&());
        let call_params: CallApiAtParams<'_, Block, MockRuntimeAPi<C, Block>, fn() -> Result<BabeGenesisConfiguration, sp_api::ApiError>, <C as CallApiAt<Block>>::StateBackend>
            = CallApiAtParams {
            core_api: self.clone(),  // core_api is MockRuntimeApi, because MockRuntimeApi implements triat Core
            at: at,
            function: "BabeApi_configuration",
            native_call: None, //Some(|| -> Result<(), sp_api::ApiError>{ Ok(())}),
            arguments: runtime_api_impl_params_encoded,
            overlayed_changes: &self.changes,
            storage_transaction_cache: &self.storage,
            initialize_block: InitializeBlock::Skip,
            // context: ExecutionContext::BlockConstruction,
            // context: ExecutionContext::Syncing,
            context: ExecutionContext::OffchainCall(None),
            recorder: &None
        };
        let res = self.call.call_api_at(call_params);
        let res = res.and_then(|r| match r {
            sp_api::NativeOrEncoded::Native(n) => Ok(n),
            sp_api::NativeOrEncoded::Encoded(r) =>
                < BabeGenesisConfiguration as sp_api::Decode>::decode (& mut & r [..])
                    .map_err (| err | sp_api :: ApiError :: FailedToDecodeReturnValue {
                        function : "BabeApi_configuration" ,
                        error : err , }) });
        if res.is_ok() {
            self.changes.borrow_mut().commit_transaction().expect("commit error!");
        } else {
            self.changes.borrow_mut().rollback_transaction().expect("rollback error!");
        }
        res
    }
    fn current_epoch_start(&self, _: &BlockId<Block>,) -> Result<Slot, sp_api::ApiError> {
        info!("call BabeApi current_epoch_start");
        unimplemented!()
        // let _epoch = Babe::current_epoch_start();
        // Ok(_epoch)
    }
    fn current_epoch(&self, _: &BlockId<Block>,) -> Result<Epoch, sp_api::ApiError> {
        info!("call BabeApi current_epoch");
        unimplemented!()
        // let _epoch = Babe::current_epoch();
        // Ok(_epoch)
    }
    fn next_epoch(&self, _: &BlockId<Block>,) -> Result<Epoch, sp_api::ApiError> {
        info!("call BabeApi next_epoch");
        unimplemented!()
        // Babe::next_epoch();
    }
    fn generate_key_ownership_proof(&self, _: &BlockId<Block>,
        _slot: Slot,
        authority_id: sp_consensus_babe::AuthorityId,
    ) -> Result<Option<sp_consensus_babe::OpaqueKeyOwnershipProof>, sp_api::ApiError> {
        info!("call BabeApi generate_key_ownership_proof");
        unimplemented!()
        // Ok(None)
    }
    fn submit_report_equivocation_unsigned_extrinsic(&self, _: &BlockId<Block>,
        equivocation_proof: sp_consensus_babe::EquivocationProof<<Block as BlockT>::Header>,
        key_owner_proof: sp_consensus_babe::OpaqueKeyOwnershipProof,
    ) -> Result<Option<()>, sp_api::ApiError> {
        info!("call BabeApi submit_report_equivocation_unsigned_extrinsic");
        unimplemented!()
        // let key_owner_proof = key_owner_proof.decode()?;
        // Babe::submit_unsigned_equivocation_report(equivocation_proof, key_owner_proof);
        // Ok(None)
    }

    fn BabeApi_configuration_runtime_api_impl(&self, at: &BlockId<Block>, context: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<BabeGenesisConfiguration>, sp_api::ApiError> {
        info!("call BabeApi_configuration_runtime_api_impl");
        let runtime_api_impl_params_encoded = sp_api::Encode::encode(&());
        let call_params: CallApiAtParams<'_, Block, MockRuntimeAPi<C, Block>, fn() -> Result<BabeGenesisConfiguration, sp_api::ApiError>, <C as CallApiAt<Block>>::StateBackend>
            = CallApiAtParams {
            core_api: self.clone(),  // core_api is MockRuntimeApi, because MockRuntimeApi implements triat Core
            at: at,
            function: "BabeApi_configuration",
            native_call: None, //Some(|| -> Result<(), sp_api::ApiError>{ Ok(())}),
            arguments: runtime_api_impl_params_encoded,
            overlayed_changes: &self.changes,
            storage_transaction_cache: &self.storage,
            initialize_block: InitializeBlock::Skip,
            context,
            recorder: &None
        };
        let res = self.call.call_api_at(call_params);
        res
    }
    fn BabeApi_current_epoch_start_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<Slot>, sp_api::ApiError> {
        info!("call BabeApi_current_epoch_start_runtime_api_impl");
        todo!() }
    fn BabeApi_current_epoch_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<Epoch>, sp_api::ApiError> {
        info!("call BabeApi_current_epoch_runtime_api_impl");
        todo!() }
    fn BabeApi_next_epoch_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<Epoch>, sp_api::ApiError> {
        info!("call BabeApi_next_epoch_runtime_api_impl");
        todo!() }
    fn BabeApi_generate_key_ownership_proof_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<(Slot, sp_consensus_babe::AuthorityId)>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<std::option::Option<sp_consensus_babe::OpaqueKeyOwnershipProof>>, sp_api::ApiError> {
        info!("call BabeApi_generate_key_ownership_proof_runtime_api_impl");
        todo!() }
    fn BabeApi_submit_report_equivocation_unsigned_extrinsic_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext,
        _: std::option::Option<(sp_consensus_babe::EquivocationProof<<Block as BlockT>::Header>, sp_consensus_babe::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
                                                                              -> std::result::Result<NativeOrEncoded<std::option::Option<()>>, sp_api::ApiError> {
        info!("call BabeApi_submit_report_equivocation_unsigned_extrinsic_runtime_api_impl");
        todo!() }
}
impl<C,Block> sp_session::SessionKeys<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
{
    fn generate_session_keys(&self, at: &BlockId<Block>, seed: Option<Vec<u8>>) -> Result<Vec<u8>, sp_api::ApiError> {
        info!("call generate_session_keys");
        // unimplemented!()
        // SessionKeys::generate(seed)
        self.changes.borrow_mut().start_transaction();
        let runtime_api_impl_params_encoded = sp_api::Encode::encode(&(&seed));
        let initialize_block = if true {
            InitializeBlock::Skip
        } else {
            InitializeBlock::Do(&self.initialized_block)
        };
        let call_params: CallApiAtParams<'_, Block, MockRuntimeAPi<C, Block>, fn() -> Result<Vec<u8>, sp_api::ApiError>, <C as CallApiAt<Block>>::StateBackend>
            = CallApiAtParams {
            core_api: self.clone(),
            at: at,
            function: "SessionKeys_generate_session_keys",
            native_call: None,
            arguments: runtime_api_impl_params_encoded,
            overlayed_changes: &self.changes,
            storage_transaction_cache: &self.storage,
            initialize_block: initialize_block,
            context: ExecutionContext::OffchainCall(None),
            recorder: &None
        };
        let res = self.call.call_api_at(call_params);
        if res.is_ok() {
            self.changes.borrow_mut().commit_transaction().expect("commit error!");
        } else {
            self.changes.borrow_mut().rollback_transaction().expect("rollback error!");
        }
        let res = res.and_then(|r| match r {
            sp_api::NativeOrEncoded::Native(n) => Ok(n),
            sp_api::NativeOrEncoded::Encoded(r) =>
                < Vec<u8> as sp_api::Decode>::decode (& mut & r [..])
                    .map_err (| err | sp_api :: ApiError :: FailedToDecodeReturnValue {
                        function : "SessionKeys_generate_session_keys" ,
                        error : err , }) });
        res
    }
    fn decode_session_keys(&self, _: &BlockId<Block>,encoded: Vec<u8>) -> Result<Option<Vec<(Vec<u8>, KeyTypeId)>>, sp_api::ApiError> {
        info!("call decode_session_keys");
        unimplemented!()
        // SessionKeys::decode_into_raw_public_keys(&encoded)
    }
    fn SessionKeys_generate_session_keys_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<std::option::Option<Vec<u8>>>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<Vec<u8>>, sp_api::ApiError> {
        info!("call SessionKeys_generate_session_keys_runtime_api_impl");
        todo!()
    }
    fn SessionKeys_decode_session_keys_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<Vec<u8>>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<std::option::Option<Vec<(Vec<u8>, KeyTypeId)>>>, sp_api::ApiError> {
        info!("call SessionKeys_decode_session_keys_runtime_api_impl");
        todo!()
    }
}
