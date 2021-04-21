use sp_api::{BlockT, HeaderT};
use sc_client_api::{StorageNotifications, BlockImportNotification, FinalityNotification, ImportSummary, CallExecutor, Backend};
use parking_lot::{Mutex, RwLock};
use sp_utils::mpsc::TracingUnboundedSender;
use std::sync::Arc;
use sp_runtime::generic::BlockId;
use crate::client::Client;
use sc_client_api::blockchain::HeaderBackend;
use sp_runtime::traits::{Header, NumberFor};
use codec::Encode;

type NotificationSinks<T> = Mutex<Vec<TracingUnboundedSender<T>>>;

pub struct ClientNotifies<B, Block>
    where
        Block: BlockT,
        Block::Header: Clone,
        // NumberFor<Block>: Into<u32>,
{
    pub backend: Arc<B>,
    pub storage_notifications: Mutex<StorageNotifications<Block>>,
    pub import_notification_sinks: NotificationSinks<BlockImportNotification<Block>>,
    pub finality_notification_sinks: NotificationSinks<FinalityNotification<Block>>,
}

impl<B, Block> ClientNotifies<B, Block>
    where
        B: Backend<Block>,
        Block: BlockT,
        Block::Header: Clone,
        // NumberFor<Block>: Into<u32>,
{
    pub fn new(backend: Arc<B>, storage_notifications: Mutex<StorageNotifications<Block>>) -> Self {
        ClientNotifies {
            backend: backend,
            storage_notifications: storage_notifications,
            import_notification_sinks: Default::default(),
            finality_notification_sinks: Default::default(),
        }
    }

    pub fn notify_finalized(&self, notify_finalized: Vec<Block::Hash>, ) -> sp_blockchain::Result<()> {
        let mut sinks = self.finality_notification_sinks.lock();

        if notify_finalized.is_empty() {
            // cleanup any closed finality notification sinks
            // since we won't be running the loop below which
            // would also remove any closed sinks.
            sinks.retain(|sink| !sink.is_closed());

            return Ok(());
        }

        // We assume the list is sorted and only want to inform the
        // telemetry once about the finalized block.
        if let Some(last) = notify_finalized.last() {
            // let header = self.client.header(&BlockId::Hash(*last))?
            //     .expect(
            //         "Header already known to exist in DB because it is \
			// 		indicated in the tree route; qed"
            //     );

            // telemetry!(
			// 	self.telemetry;
			// 	SUBSTRATE_INFO;
			// 	"notify.finalized";
			// 	"height" => format!("{}", header.number()),
			// 	"best" => ?last,
			// );
        }

        for finalized_hash in notify_finalized {
            let header = self.backend.blockchain().header(BlockId::Hash(finalized_hash))?
                .expect(
                    "Header already known to exist in DB because it is \
					indicated in the tree route; qed"
                );

            let notification = FinalityNotification {
                header,
                hash: finalized_hash,
            };

            sinks.retain(|sink| sink.unbounded_send(notification.clone()).is_ok());
        }

        Ok(())
    }

    pub fn notify_imported(
        &self,
        notify_import: Option<ImportSummary<Block>>,
    ) -> sp_blockchain::Result<()> {
        let notify_import = match notify_import {
            Some(notify_import) => notify_import,
            None => {
                // cleanup any closed import notification sinks since we won't
                // be sending any notifications below which would remove any
                // closed sinks. this is necessary since during initial sync we
                // won't send any import notifications which could lead to a
                // temporary leak of closed/discarded notification sinks (e.g.
                // from consensus code).
                self.import_notification_sinks
                    .lock()
                    .retain(|sink| !sink.is_closed());

                return Ok(());
            }
        };

        if let Some(storage_changes) = notify_import.storage_changes {
            // v1: archive logic done here
            let hash = notify_import.hash;
            let number = notify_import.header.number();
            let header: Block::Header = notify_import.header.clone();
            let _parent_hash: &[u8] = header.parent_hash().as_ref();
            // From<<<Block as BlockT>::Header as HeaderT>::Number>` is not implemented for `u32`
            // let _block_num: u32 = (*header.number()).into();
            let _state_root: &[u8] = header.state_root().as_ref();
            let _extrinsics_root: &[u8] = header.extrinsics_root().as_ref();
            let _digest = header.digest().encode();
            let _body = notify_import.body.unwrap().encode();
            // let _justification = notify_import.justifications;


            // TODO [ToDr] How to handle re-orgs? Should we re-emit all storage changes?
            self.storage_notifications.lock()
                .trigger(
                    &notify_import.hash,
                    storage_changes.0.into_iter(),
                    storage_changes.1.into_iter().map(|(sk, v)| (sk, v.into_iter())),
                );
        }

        let notification = BlockImportNotification::<Block> {
            hash: notify_import.hash,
            origin: notify_import.origin,
            header: notify_import.header,
            is_new_best: notify_import.is_new_best,
            tree_route: notify_import.tree_route.map(Arc::new),
        };

        self.import_notification_sinks.lock()
            .retain(|sink| sink.unbounded_send(notification.clone()).is_ok());

        Ok(())
    }
}

