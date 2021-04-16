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
use sp_state_machine::{DBValue, Backend as StateBackend, ChangesTrieAnchorBlockId, prove_read, prove_child_read, ChangesTrieRootsStorage, ChangesTrieStorage, ChangesTrieConfigurationRange, key_changes, key_changes_proof, Backend, OverlayedChanges};
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

pub struct MockRuntimeAPi<C: 'static , Block> where
    Block: BlockT,
    C: CallApiAt<Block> + 'static + Send + Sync,
    C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>,
{
    pub call: &'static C,
    pub changes: RefCell<OverlayedChanges>,
    pub storage: RefCell<StorageTransactionCache<Block, C::StateBackend>>,
    pub(crate) _ph: PhantomData<Block>
}

// impl<B, E, Block> From<MockRuntimeAPi<&Client<B, E, Block>, Block>>  for ApiRef<'_, MockRuntimeAPi<Client<B, E, Block>, Block>>
//     where Block: BlockT, {
//     fn from(_: MockRuntimeAPi<&Client<B, E, Block>, Block>) -> Self {
//         ApiRef(api, Default::default())
//     }
// }

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

impl<C, Block> ApiExt<Block> for MockRuntimeAPi<C,Block> where
    Block: BlockT,
    C: CallApiAt<Block> + 'static + Send + Sync,
    C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
{
    // type StateBackend = sp_api::InMemoryBackend<sp_api::HashFor<Block>>;
    type StateBackend = SyncingCachingState<RefTrackingState<Block>, Block>;

    fn execute_in_transaction<F: FnOnce(&Self) -> TransactionOutcome<R>, R>(&self, call: F) -> R where Self: Sized {
        unimplemented!()
    }

    fn has_api<A: RuntimeApiInfo + ?Sized>(&self, at: &BlockId<Block>) -> Result<bool, ApiError> where Self: Sized {
        // unimplemented!()
        Ok(true)
    }

    fn has_api_with<A: RuntimeApiInfo + ?Sized, P: Fn(u32) -> bool>(&self, at: &BlockId<Block>, pred: P) -> Result<bool, ApiError> where Self: Sized {
        // unimplemented!()
        Ok(true)
    }

    fn record_proof(&mut self) {
        unimplemented!()
    }

    fn extract_proof(&mut self) -> Option<StorageProof> {
        unimplemented!()
    }

    fn into_storage_changes(&self, backend: &Self::StateBackend,
                            changes_trie_state: Option<&ChangesTrieState<HashFor<Block>, NumberFor<Block>>>,
                            parent_hash: <Block as BlockT>::Hash
    ) -> Result<StorageChanges<Self::StateBackend, Block>, String> where Self: Sized {
        // self.initialized_block.borrow_mut().take();
        self.changes
            .replace(Default::default())
            .into_storage_changes(
                backend,
                changes_trie_state,
                parent_hash,
                // self.storage.replace(Default::default()),
                Default::default(),
            )
        // unimplemented!()
    }
}

// impl<Block, UnsignedValidator, Context: Default> sp_api::Core<Block> for MockRuntimeAPi<Block> where
impl<C,Block> sp_api::Core<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>,
        // Context: Default,
        // Block::Extrinsic: Checkable<Context> + Codec,
        // CheckedOf<Block::Extrinsic, Context>: Applyable + GetDispatchInfo,
        // CallOf<Block::Extrinsic, Context>:
        // Dispatchable<Info = DispatchInfo, PostInfo = PostDispatchInfo>,
        // UnsignedValidator: ValidateUnsigned<Call=CallOf<Block::Extrinsic, Context>>,
{
    fn version(&self, at: &BlockId<Block>) -> Result<RuntimeVersion, sp_api::ApiError> {
        // VERSION
        info!("call version");
        unimplemented!()
    }
    fn execute_block(&self, at: &BlockId<Block>, block: Block) -> Result<(), sp_api::ApiError> {
        // let res = Executive::execute_block(block);
        // Ok(res)
        unimplemented!()
    }
    fn initialize_block(&self, at: &BlockId<Block>, header: &<Block as BlockT>::Header) -> Result<(), sp_api::ApiError> {
        // Executive::initialize_block(header)
        unimplemented!()
    }
    fn Core_version_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
                                     -> std::result::Result<NativeOrEncoded<RuntimeVersion>, sp_api::ApiError> {
        info!("call Core_version");
        unimplemented!() }
    fn Core_execute_block_runtime_api_impl(&self, at: &BlockId<Block>, context: ExecutionContext, params: std::option::Option<Block>, params_encoded: Vec<u8>)
                                           -> std::result::Result<NativeOrEncoded<()>, sp_api::ApiError> {
        // info!("execute block at {}", at);
        // Executive::execute_block(params.unwrap());
        // Ok(NativeOrEncoded::Encoded(vec![]))
        let runtime_api_impl_params_encoded = sp_api::Encode::encode(&(&params.unwrap()));
        let _changes: OverlayedChanges = Default::default();
        let _storage: StorageTransactionCache<Block, C::StateBackend> = Default::default();
        let call_params: CallApiAtParams<'_, Block, MockRuntimeAPi<C, Block>, fn() -> Result<(), sp_api::ApiError>, <C as CallApiAt<Block>>::StateBackend>
            = CallApiAtParams {
            core_api: self.clone(),
            at: at,
            function: "Core_execute_block",
            native_call: None, //Some(|| -> Result<(), sp_api::ApiError>{ Ok(())}),
            arguments: runtime_api_impl_params_encoded,
            overlayed_changes: &RefCell::new(_changes),
            storage_transaction_cache: &RefCell::new(_storage),
            initialize_block: InitializeBlock::Skip,
            context,
            recorder: &None
        };
        self.call.call_api_at(call_params);
        Ok(NativeOrEncoded::Encoded(vec![]))
        // unimplemented!()
    }
    fn Core_initialize_block_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<&<Block as BlockT>::Header>, _: Vec<u8>)
                                              -> std::result::Result<NativeOrEncoded<()>, sp_api::ApiError> {

        unimplemented!()
    }
}
impl<C,Block> sp_api::Metadata<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
        // <Block as BlockT>::Extrinsic: BlindCheckable,
        // <<Block as BlockT>::Extrinsic as BlindCheckable>::Checked: Applyable
{
    fn metadata(&self, at: &BlockId<Block>) -> Result<sp_core::OpaqueMetadata, sp_api::ApiError> {
        // Runtime::metadata().into()
        unimplemented!()
    }
    fn Metadata_metadata_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
                                          -> std::result::Result<NativeOrEncoded<OpaqueMetadata>, sp_api::ApiError> { unimplemented!() }
}

impl<C,Block> sp_block_builder::BlockBuilder<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
        // <Block as BlockT>::Extrinsic: BlindCheckable,
        // <<Block as BlockT>::Extrinsic as BlindCheckable>::Checked: Applyable
{
    fn apply_extrinsic(&self, at: &BlockId<Block>, extrinsic: <Block as BlockT>::Extrinsic) -> Result<ApplyExtrinsicResult, sp_api::ApiError> {
        // Executive::apply_extrinsic(extrinsic)
        unimplemented!()
    }
    fn finalize_block(&self, at: &BlockId<Block>) -> Result<<Block as BlockT>::Header,sp_api::ApiError> {
        // Executive::finalize_block()
        unimplemented!()
    }
    fn inherent_extrinsics(&self, at: &BlockId<Block>, data: InherentData) -> Result<Vec<<Block as BlockT>::Extrinsic>, sp_api::ApiError> {
        // data.create_extrinsics()
        unimplemented!()
    }
    fn check_inherents(&self, at: &BlockId<Block>, block: Block, data: InherentData) -> Result<CheckInherentsResult,sp_api::ApiError> {
        // data.check_extrinsics(&block)
        unimplemented!()
    }
    fn random_seed(&self, at: &BlockId<Block>) -> Result<<Block as BlockT>::Hash, sp_api::ApiError> {
        // pallet_babe::RandomnessFromOneEpochAgo::<Runtime>::random_seed().0
        unimplemented!()
    }

    fn BlockBuilder_apply_extrinsic_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<<Block as BlockT>::Extrinsic>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<std::result::Result<std::result::Result<(), DispatchError>, TransactionValidityError>>, sp_api::ApiError> {
        unimplemented!()
    }
    fn BlockBuilder_finalize_block_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<<Block as BlockT>::Header>, sp_api::ApiError> {
        unimplemented!()
    }
    fn BlockBuilder_inherent_extrinsics_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<sp_consensus::InherentData>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<Vec<<Block as BlockT>::Extrinsic>>, sp_api::ApiError> {
        unimplemented!()
    }
    fn BlockBuilder_check_inherents_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<(Block, sp_consensus::InherentData)>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<CheckInherentsResult>, sp_api::ApiError> {
        unimplemented!()
    }
    fn BlockBuilder_random_seed_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<<Block as BlockT>::Hash>, sp_api::ApiError> {
        unimplemented!()
    }
}

impl<C,Block> fg_primitives::GrandpaApi<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
        // <Block as BlockT>::Extrinsic: BlindCheckable,
        // <<Block as BlockT>::Extrinsic as BlindCheckable>::Checked: Applyable
{
    fn grandpa_authorities(&self, _: &BlockId<Block>,) -> Result<GrandpaAuthorityList, sp_api::ApiError> {
        // Grandpa::grandpa_authorities()
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
        unimplemented!()
    }
    fn generate_key_ownership_proof(&self, _: &BlockId<Block>,
        _set_id: fg_primitives::SetId,
        authority_id: GrandpaId,
    ) -> Result<Option<fg_primitives::OpaqueKeyOwnershipProof>, sp_api::ApiError> {
        // use codec::Encode;
        unimplemented!()
    }
    fn GrandpaApi_grandpa_authorities_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<Vec<(fg_primitives::AuthorityId, u64)>>, sp_api::ApiError> { todo!() }
    fn GrandpaApi_submit_report_equivocation_unsigned_extrinsic_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext,
        _: std::option::Option<(fg_primitives::EquivocationProof<<Block as BlockT>::Hash, <<Block as BlockT>::Header as HeaderT>::Number>,
                                // pallet_grandpa::sp_finality_grandpa::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
                                fg_primitives::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
                                // pallet_grandpa::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<std::option::Option<()>>, sp_api::ApiError> { todo!() }
    fn GrandpaApi_generate_key_ownership_proof_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<(u64, fg_primitives::AuthorityId)>, _: Vec<u8>)
        -> std::result::Result<NativeOrEncoded<std::option::Option<fg_primitives::OpaqueKeyOwnershipProof>>, sp_api::ApiError> { todo!() }
}
impl<C,Block> sp_consensus_babe::BabeApi<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
        // <Block as BlockT>::Extrinsic: BlindCheckable,
        // <<Block as BlockT>::Extrinsic as BlindCheckable>::Checked: Applyable
{
    fn configuration(&self, _: &BlockId<Block>,) -> Result<BabeGenesisConfiguration, sp_api::ApiError> {
        // sp_consensus_babe::BabeGenesisConfiguration {
        //     slot_duration: Babe::slot_duration(),
        //     epoch_length: EpochDuration::get(),
        //     c: BABE_GENESIS_EPOCH_CONFIG.c,
        //     genesis_authorities: Babe::authorities(),
        //     randomness: Babe::randomness(),
        //     allowed_slots: BABE_GENESIS_EPOCH_CONFIG.allowed_slots,
        // }
        unimplemented!()
    }
    fn current_epoch_start(&self, _: &BlockId<Block>,) -> Result<Slot, sp_api::ApiError> {
        // Babe::current_epoch_start()
        unimplemented!()
    }
    fn current_epoch(&self, _: &BlockId<Block>,) -> Result<Epoch, sp_api::ApiError> {
        // Babe::current_epoch()
        unimplemented!()
    }
    fn next_epoch(&self, _: &BlockId<Block>,) -> Result<Epoch, sp_api::ApiError> {
        // Babe::next_epoch()
        unimplemented!()
    }
    fn generate_key_ownership_proof(&self, _: &BlockId<Block>,
        _slot: Slot,
        authority_id: sp_consensus_babe::AuthorityId,
    ) -> Result<Option<sp_consensus_babe::OpaqueKeyOwnershipProof>, sp_api::ApiError> {
        // None
        unimplemented!()
    }
    fn submit_report_equivocation_unsigned_extrinsic(&self, _: &BlockId<Block>,
        equivocation_proof: sp_consensus_babe::EquivocationProof<<Block as BlockT>::Header>,
        key_owner_proof: sp_consensus_babe::OpaqueKeyOwnershipProof,
    ) -> Result<Option<()>, sp_api::ApiError> {
        // let key_owner_proof = key_owner_proof.decode()?;
        // Babe::submit_unsigned_equivocation_report(equivocation_proof, key_owner_proof)
        unimplemented!()
    }

    fn BabeApi_configuration_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>) -> std::result::Result<NativeOrEncoded<BabeGenesisConfiguration>, sp_api::ApiError> { todo!() }
    fn BabeApi_current_epoch_start_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>) -> std::result::Result<NativeOrEncoded<Slot>, sp_api::ApiError> { todo!() }
    fn BabeApi_current_epoch_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>) -> std::result::Result<NativeOrEncoded<Epoch>, sp_api::ApiError> { todo!() }
    fn BabeApi_next_epoch_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>) -> std::result::Result<NativeOrEncoded<Epoch>, sp_api::ApiError> { todo!() }
    fn BabeApi_generate_key_ownership_proof_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<(Slot, sp_consensus_babe::AuthorityId)>, _: Vec<u8>) -> std::result::Result<NativeOrEncoded<std::option::Option<sp_consensus_babe::OpaqueKeyOwnershipProof>>, sp_api::ApiError> { todo!() }
    fn BabeApi_submit_report_equivocation_unsigned_extrinsic_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext,
                                                                              // _: std::option::Option<(sp_consensus_slots::EquivocationProof<<Block as BlockT>::Header, sp_consensus_babe::app::Public>, sp_consensus_babe::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
                                                                              // _: std::option::Option<(sp_consensus_slots::EquivocationProof<<Block as BlockT>::Header, sp_consensus_babe::AuthorityId>, sp_consensus_babe::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
        _: std::option::Option<(sp_consensus_babe::EquivocationProof<<Block as BlockT>::Header>, sp_consensus_babe::OpaqueKeyOwnershipProof)>, _: Vec<u8>)
                                                                              -> std::result::Result<NativeOrEncoded<std::option::Option<()>>, sp_api::ApiError> { todo!() }
}
impl<C,Block> sp_session::SessionKeys<Block> for MockRuntimeAPi<C,Block> where
        Block: BlockT,
        C: CallApiAt<Block> + 'static + Send + Sync,
        C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
        // <Block as BlockT>::Extrinsic: BlindCheckable,
        // <<Block as BlockT>::Extrinsic as BlindCheckable>::Checked: Applyable
{
    fn generate_session_keys(&self, _: &BlockId<Block>,seed: Option<Vec<u8>>) -> Result<Vec<u8>, sp_api::ApiError> {
        // SessionKeys::generate(seed)
        unimplemented!()
    }
    fn decode_session_keys(&self, _: &BlockId<Block>,encoded: Vec<u8>) -> Result<Option<Vec<(Vec<u8>, KeyTypeId)>>, sp_api::ApiError> {
        // SessionKeys::decode_into_raw_public_keys(&encoded)
        unimplemented!()
    }
    fn SessionKeys_generate_session_keys_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<std::option::Option<Vec<u8>>>, _: Vec<u8>) -> std::result::Result<NativeOrEncoded<Vec<u8>>, sp_api::ApiError> { todo!() }
    fn SessionKeys_decode_session_keys_runtime_api_impl(&self, _: &BlockId<Block>, _: ExecutionContext, _: std::option::Option<Vec<u8>>, _: Vec<u8>) -> std::result::Result<NativeOrEncoded<std::option::Option<Vec<(Vec<u8>, KeyTypeId)>>>, sp_api::ApiError> { todo!() }
}
