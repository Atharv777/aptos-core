// Copyright © Aptos Foundation
// SPDX-License-Identifier: Apache-2.0

use crate::{
    access_path_cache::AccessPathCache,
    data_cache::MoveResolverWithVMMetadata,
    move_vm_ext::{MoveResolverExt, MoveVmExt},
    transaction_metadata::TransactionMetadata,
};
use aptos_aggregator::{
    aggregator_extension::AggregatorID,
    delta_change_set::{serialize, DeltaChangeSet},
    transaction::ChangeSetExt,
};
use aptos_crypto::{hash::CryptoHash, HashValue};
use aptos_crypto_derive::{BCSCryptoHash, CryptoHasher};
use aptos_framework::natives::{
    aggregator_natives::{AggregatorChange, AggregatorChangeSet, NativeAggregatorContext},
    code::{NativeCodeContext, PublishRequest},
};
use aptos_gas::ChangeSetConfigs;
use aptos_logger::error;
use aptos_types::{
    block_metadata::BlockMetadata,
    contract_event::ContractEvent,
    on_chain_config::{CurrentTimeMicroseconds, OnChainConfig},
    state_store::{state_key::StateKey, state_value::StateValueMetadata, table::TableHandle},
    transaction::{ChangeSet, SignatureCheckedTransaction},
    write_set::{WriteOp, WriteSetMut},
};
use move_binary_format::errors::{Location, PartialVMError, VMResult};
use move_core_types::{
    account_address::AccountAddress,
    effects::{
        AccountChangeSet, ChangeSet as MoveChangeSet, Event as MoveEvent, Op as MoveStorageOp,
    },
    language_storage::{ModuleId, StructTag, CORE_CODE_ADDRESS},
    vm_status::{StatusCode, StatusCode::UNKNOWN_INVARIANT_VIOLATION_ERROR, VMStatus},
};
use move_table_extension::{NativeTableContext, TableChangeSet};
use move_vm_runtime::session::Session;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[derive(BCSCryptoHash, CryptoHasher, Deserialize, Serialize)]
pub enum SessionId {
    Txn {
        sender: AccountAddress,
        sequence_number: u64,
        script_hash: Vec<u8>,
    },
    BlockMeta {
        // block id
        id: HashValue,
    },
    Genesis {
        // id to identify this specific genesis build
        id: HashValue,
    },
    // For those runs that are not a transaction and the output of which won't be committed.
    Void,
}

impl SessionId {
    pub fn txn(txn: &SignatureCheckedTransaction) -> Self {
        Self::txn_meta(&TransactionMetadata::new(&txn.clone().into_inner()))
    }

    pub fn txn_meta(txn_data: &TransactionMetadata) -> Self {
        Self::Txn {
            sender: txn_data.sender,
            sequence_number: txn_data.sequence_number,
            script_hash: txn_data.script_hash.clone(),
        }
    }

    pub fn genesis(id: HashValue) -> Self {
        Self::Genesis { id }
    }

    pub fn block_meta(block_meta: &BlockMetadata) -> Self {
        Self::BlockMeta {
            id: block_meta.id(),
        }
    }

    pub fn void() -> Self {
        Self::Void
    }

    pub fn as_uuid(&self) -> HashValue {
        self.hash()
    }

    pub fn sender(&self) -> Option<AccountAddress> {
        match self {
            SessionId::Txn { sender, .. } => Some(*sender),
            SessionId::BlockMeta { .. } | SessionId::Genesis { .. } | SessionId::Void => None,
        }
    }
}

pub struct SessionExt<'r, 'l, S> {
    inner: Session<'r, 'l, S>,
    remote: MoveResolverWithVMMetadata<'r, 'l, S>,
    new_slot_payer: Option<AccountAddress>,
}

impl<'r, 'l, S> SessionExt<'r, 'l, S>
where
    S: MoveResolverExt + 'r,
{
    pub fn new(
        inner: Session<'r, 'l, S>,
        move_vm: &'l MoveVmExt,
        remote: &'r S,
        new_slot_payer: Option<AccountAddress>,
    ) -> Self {
        Self {
            inner,
            remote: MoveResolverWithVMMetadata::new(remote, move_vm),
            new_slot_payer,
        }
    }

    pub fn finish<C: AccessPathCache>(
        self,
        ap_cache: &mut C,
        configs: &ChangeSetConfigs,
    ) -> VMResult<ChangeSetExt> {
        let (change_set_ext, _current_timestamp) =
            self.finish_with_current_timestamp(ap_cache, configs)?;

        Ok(change_set_ext)
    }

    pub fn finish_with_current_timestamp<C: AccessPathCache>(
        self,
        ap_cache: &mut C,
        configs: &ChangeSetConfigs,
    ) -> VMResult<(ChangeSetExt, Option<CurrentTimeMicroseconds>)> {
        let (change_set, events, mut extensions) = self.inner.finish_with_extensions()?;
        let (change_set, resource_group_change_set, updated_timestamp) =
            Self::split_and_merge_resource_groups(&self.remote, change_set)?;
        let current_time = Self::get_current_timestamp(updated_timestamp, &self.remote);

        let table_context: NativeTableContext = extensions.remove();
        let table_change_set = table_context
            .into_change_set()
            .map_err(|e| e.finish(Location::Undefined))?;

        let aggregator_context: NativeAggregatorContext = extensions.remove();
        let aggregator_change_set = aggregator_context.into_change_set();

        let change_set_ext = Self::convert_change_set(
            &self.remote,
            self.new_slot_payer,
            current_time.as_ref(),
            change_set,
            resource_group_change_set,
            events,
            table_change_set,
            aggregator_change_set,
            ap_cache,
            configs,
        )
        .map_err(|status| PartialVMError::new(status.status_code()).finish(Location::Undefined))?;

        Ok((change_set_ext, current_time))
    }

    pub fn extract_publish_request(&mut self) -> Option<PublishRequest> {
        let ctx = self.get_native_extensions().get_mut::<NativeCodeContext>();
        ctx.requested_module_bundle.take()
    }

    /// * Separate the resource groups from the non-resource groups
    /// * non-resource groups are kept as is
    /// * resource groups are merged into the correct format as deltas to the source data
    ///   * Remove resource group data from the deltas
    ///   * Attempt to read the existing resource group data or create a new empty container
    ///   * Apply the deltas to the resource group data
    /// The process for translating Move deltas of resource groups to resources is
    /// * Add -- insert element in container
    ///   * If entry exists, Unreachable
    ///   * If group exists, Modify
    ///   * If group doesn't exist, Add
    /// * Modify -- update element in container
    ///   * If group or data doesn't exist, Unreachable
    ///   * Otherwise modify
    /// * Delete -- remove element from container
    ///   * If group or data does't exist, Unreachable
    ///   * If elements remain, Modify
    ///   * Otherwise delete
    fn split_and_merge_resource_groups(
        remote: &MoveResolverWithVMMetadata<S>,
        change_set: MoveChangeSet,
    ) -> VMResult<(
        MoveChangeSet,
        MoveChangeSet,
        Result<Option<CurrentTimeMicroseconds>, ()>,
    )> {
        // The use of this implies that we could theoretically call unwrap with no consequences,
        // but using unwrap means the code panics if someone can come up with an attack.
        let common_error = PartialVMError::new(StatusCode::UNKNOWN_INVARIANT_VIOLATION_ERROR)
            .finish(Location::Undefined);
        let mut change_set_filtered = MoveChangeSet::new();
        let mut resource_group_change_set = MoveChangeSet::new();
        // Assuming the timestamp has not been updated in this session by returning Ok(None), which
        //   results in the call site to assume the timestamp to be the same as in the base state
        //   view
        let mut updated_timestamp: Result<Option<CurrentTimeMicroseconds>, ()> = Ok(None);

        for (addr, account_changeset) in change_set.into_inner() {
            let mut resource_groups: BTreeMap<StructTag, AccountChangeSet> = BTreeMap::new();
            let (modules, resources) = account_changeset.into_inner();

            for (struct_tag, blob_op) in resources {
                let resource_group = remote
                    .get_resource_group(&struct_tag)
                    .map_err(|_| common_error.clone())?;
                if let Some(resource_group) = resource_group {
                    resource_groups
                        .entry(resource_group)
                        .or_insert_with(AccountChangeSet::new)
                        .add_resource_op(struct_tag, blob_op)
                        .map_err(|_| common_error.clone())?;
                } else {
                    if addr == CORE_CODE_ADDRESS
                        && CurrentTimeMicroseconds::struct_tag() == struct_tag
                    {
                        updated_timestamp = Self::parse_updated_timestamp(&blob_op)
                    }

                    change_set_filtered
                        .add_resource_op(addr, struct_tag, blob_op)
                        .map_err(|_| common_error.clone())?;
                }
            }

            for (name, blob_op) in modules {
                change_set_filtered
                    .add_module_op(ModuleId::new(addr, name), blob_op)
                    .map_err(|_| common_error.clone())?;
            }

            for (resource_tag, resources) in resource_groups {
                let source_data = remote
                    .get_resource_group_data(&addr, &resource_tag)
                    .map_err(|_| common_error.clone())?;
                let (mut source_data, create) = if let Some(source_data) = source_data {
                    let source_data =
                        bcs::from_bytes(&source_data).map_err(|_| common_error.clone())?;
                    (source_data, false)
                } else {
                    (BTreeMap::new(), true)
                };

                for (struct_tag, current_op) in resources.into_resources() {
                    match current_op {
                        MoveStorageOp::Delete => {
                            source_data
                                .remove(&struct_tag)
                                .ok_or_else(|| common_error.clone())?;
                        },
                        MoveStorageOp::Modify(new_data) => {
                            let data = source_data
                                .get_mut(&struct_tag)
                                .ok_or_else(|| common_error.clone())?;
                            *data = new_data;
                        },
                        MoveStorageOp::New(data) => {
                            let data = source_data.insert(struct_tag, data);
                            if data.is_some() {
                                return Err(common_error);
                            }
                        },
                    }
                }

                let op = if source_data.is_empty() {
                    MoveStorageOp::Delete
                } else if create {
                    MoveStorageOp::New(
                        bcs::to_bytes(&source_data).map_err(|_| common_error.clone())?,
                    )
                } else {
                    MoveStorageOp::Modify(
                        bcs::to_bytes(&source_data).map_err(|_| common_error.clone())?,
                    )
                };
                resource_group_change_set
                    .add_resource_op(addr, resource_tag, op)
                    .map_err(|_| common_error.clone())?;
            }
        }

        Ok((
            change_set_filtered,
            resource_group_change_set,
            updated_timestamp,
        ))
    }

    // Seen the global timestamp resource being updated.
    //   returns Ok(Some(timestamp)) if successfully parsed it
    //   returns Err(()) if failed to parse or seeing a delete, which will result in the call site
    //     assuming the timestamp to be None
    fn parse_updated_timestamp(
        op: &MoveStorageOp<Vec<u8>>,
    ) -> Result<Option<CurrentTimeMicroseconds>, ()> {
        match op {
            MoveStorageOp::New(bytes) | MoveStorageOp::Modify(bytes) => {
                match bcs::from_bytes(bytes) {
                    Ok(current_time) => Ok(Some(current_time)),
                    Err(err) => {
                        error!("Failed to deserialize CurrentTimeMicroseconds. {}", err);
                        Err(())
                    },
                }
            },
            MoveStorageOp::Delete => {
                error!("CurrentTimeMicroseconds got deleted.");
                Err(())
            },
        }
    }

    fn get_current_timestamp(
        updated_timestamp: Result<Option<CurrentTimeMicroseconds>, ()>,
        remote: &MoveResolverWithVMMetadata<S>,
    ) -> Option<CurrentTimeMicroseconds> {
        match updated_timestamp {
            Ok(Some(timestamp)) => Some(timestamp),
            Ok(None) => CurrentTimeMicroseconds::fetch_config(remote),
            Err(()) => None,
        }
    }

    pub fn convert_change_set<C: AccessPathCache>(
        remote: &MoveResolverWithVMMetadata<S>,
        new_slot_payer: Option<AccountAddress>,
        current_time: Option<&CurrentTimeMicroseconds>,
        change_set: MoveChangeSet,
        resource_group_change_set: MoveChangeSet,
        events: Vec<MoveEvent>,
        table_change_set: TableChangeSet,
        aggregator_change_set: AggregatorChangeSet,
        ap_cache: &mut C,
        configs: &ChangeSetConfigs,
    ) -> Result<ChangeSetExt, VMStatus> {
        let mut write_set_mut = WriteSetMut::new(Vec::new());
        let mut delta_change_set = DeltaChangeSet::empty();
        let mut new_slot_metadata: Option<StateValueMetadata> = None;
        // if remote.vm().features().is_storage_slot_metadata_enabled() {
        if let Some(payer) = new_slot_payer {
            if let Some(current_time) = current_time {
                new_slot_metadata = Some(StateValueMetadata::new(payer, 0, current_time));
            }
        }
        // }
        let woc = WriteOpConverter {
            remote,
            new_slot_metadata,
        };

        for (addr, account_changeset) in change_set.into_inner() {
            let (modules, resources) = account_changeset.into_inner();
            for (struct_tag, blob_op) in resources {
                let state_key = StateKey::access_path(ap_cache.get_resource_path(addr, struct_tag));
                let op = woc.convert(
                    &state_key,
                    blob_op,
                    configs.legacy_resource_creation_as_modification(),
                )?;

                write_set_mut.insert((state_key, op))
            }

            for (name, blob_op) in modules {
                let state_key =
                    StateKey::access_path(ap_cache.get_module_path(ModuleId::new(addr, name)));
                let op = woc.convert(&state_key, blob_op, false)?;
                write_set_mut.insert((state_key, op))
            }
        }

        for (addr, account_changeset) in resource_group_change_set.into_inner() {
            let (_, resources) = account_changeset.into_inner();
            for (struct_tag, blob_op) in resources {
                let state_key =
                    StateKey::access_path(ap_cache.get_resource_group_path(addr, struct_tag));
                let op = woc.convert(&state_key, blob_op, false)?;
                write_set_mut.insert((state_key, op))
            }
        }

        for (handle, change) in table_change_set.changes {
            for (key, value_op) in change.entries {
                let state_key = StateKey::table_item(handle.into(), key);
                let op = woc.convert(&state_key, value_op, false)?;
                write_set_mut.insert((state_key, op))
            }
        }

        for (id, change) in aggregator_change_set.changes {
            let AggregatorID { handle, key } = id;
            let key_bytes = key.0.to_vec();
            let state_key = StateKey::table_item(TableHandle::from(handle), key_bytes);

            match change {
                AggregatorChange::Write(value) => {
                    let bytes = serialize(&value);
                    let move_op = match write_set_mut.get(&state_key) {
                        None => MoveStorageOp::New(bytes),
                        Some(direct_write) => match direct_write {
                            WriteOp::Creation(_) | WriteOp::CreationWithMetadata { .. } => {
                                MoveStorageOp::New(bytes)
                            },
                            // FIXME(alden): is this an error?
                            WriteOp::Modification(_)
                            | WriteOp::ModificationWithMetadata { .. }
                            | WriteOp::Deletion
                            | WriteOp::DeletionWithMetadata { .. } => {
                                return Err(VMStatus::Error(
                                    UNKNOWN_INVARIANT_VIOLATION_ERROR,
                                    Some("Direct write to aggregator value".to_string()),
                                ))
                            },
                        },
                    };

                    let write_op = woc.convert(&state_key, move_op, false)?;
                    write_set_mut.insert((state_key, write_op));
                },
                AggregatorChange::Merge(delta_op) => delta_change_set.insert((state_key, delta_op)),
                AggregatorChange::Delete => {
                    // FIXME(alden): should we ensure a deletion in the write set already?
                    let write_op = woc.convert(&state_key, MoveStorageOp::Delete, false)?;
                    write_set_mut.insert((state_key, write_op));
                },
            }
        }

        let write_set = write_set_mut
            .freeze()
            .map_err(|_| VMStatus::Error(StatusCode::DATA_FORMAT_ERROR, None))?;

        let events = events
            .into_iter()
            .map(|(guid, seq_num, ty_tag, blob)| {
                let key = bcs::from_bytes(guid.as_slice())
                    .map_err(|_| VMStatus::Error(StatusCode::EVENT_KEY_MISMATCH, None))?;
                Ok(ContractEvent::new(key, seq_num, ty_tag, blob))
            })
            .collect::<Result<Vec<_>, VMStatus>>()?;

        let change_set = ChangeSet::new(write_set, events, configs)?;
        Ok(ChangeSetExt::new(
            delta_change_set,
            change_set,
            Arc::new(configs.clone()),
        ))
    }
}

impl<'r, 'l, S> Deref for SessionExt<'r, 'l, S> {
    type Target = Session<'r, 'l, S>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'r, 'l, S> DerefMut for SessionExt<'r, 'l, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

struct WriteOpConverter<'r, 'l, S> {
    remote: &'r MoveResolverWithVMMetadata<'r, 'l, S>,
    new_slot_metadata: Option<StateValueMetadata>,
}

impl<'r, 'l, S: MoveResolverExt> WriteOpConverter<'r, 'l, S> {
    fn convert(
        &self,
        state_key: &StateKey,
        move_storage_op: MoveStorageOp<Vec<u8>>,
        legacy_creation_as_modification: bool,
    ) -> Result<WriteOp, VMStatus> {
        use MoveStorageOp::*;
        use WriteOp::*;

        let existing_value_opt = self
            .remote
            .get_state_value(state_key)
            .map_err(|_| VMStatus::Error(StatusCode::STORAGE_ERROR, None))?;

        let write_op = match (existing_value_opt, move_storage_op) {
            (None, Modify(_) | Delete) | (Some(_), New(_)) => {
                return Err(VMStatus::Error(
                    StatusCode::UNKNOWN_INVARIANT_VIOLATION_ERROR,
                    None,
                ))
            },
            (None, New(data)) => match &self.new_slot_metadata {
                None => {
                    if legacy_creation_as_modification {
                        Modification(data)
                    } else {
                        Creation(data)
                    }
                },
                Some(metadata) => CreationWithMetadata {
                    data,
                    metadata: metadata.clone(),
                },
            },
            (Some(existing_value), Modify(data)) => {
                // Inherit metadata even if the feature flags is turned off, for compatibility.
                match existing_value.into_metadata() {
                    None => Modification(data),
                    Some(metadata) => ModificationWithMetadata { data, metadata },
                }
            },
            (Some(existing_value), Delete) => {
                // Inherit metadata even if the feature flags is turned off, for compatibility.
                match existing_value.into_metadata() {
                    None => Deletion,
                    Some(metadata) => DeletionWithMetadata { metadata },
                }
            },
        };
        Ok(write_op)
    }
}
