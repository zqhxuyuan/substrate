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

// pub struct MockRuntimeAPi<C, Block> where
//     Block: BlockT,
//     C: CallApiAt<Block> + 'static,
//     C::StateBackend: sp_api::StateBackend<sp_api::HashFor<Block>,>
// {
//     pub call: &'static C,
//     pub(crate) _ph: PhantomData<Block>
// }

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