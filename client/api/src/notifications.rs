// This file is part of Substrate.

// Copyright (C) 2017-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Storage notifications

use std::{
	collections::{HashSet, HashMap},
	sync::Arc,
};

use fnv::{FnvHashSet, FnvHashMap};
use sp_core::storage::{StorageKey, StorageData};
use sp_runtime::traits::Block as BlockT;
use sp_utils::mpsc::{TracingUnboundedSender, TracingUnboundedReceiver, tracing_unbounded};
use prometheus_endpoint::{Registry, CounterVec, Opts, U64, register};

/// Storage change set
#[derive(Debug)]
pub struct StorageChangeSet {
	changes: Arc<Vec<(StorageKey, Option<StorageData>)>>,
	child_changes: Arc<Vec<(StorageKey, Vec<(StorageKey, Option<StorageData>)>)>>,
	filter: Option<HashSet<StorageKey>>,
	child_filters: Option<HashMap<StorageKey, Option<HashSet<StorageKey>>>>,
}

impl StorageChangeSet {
	/// Convert the change set into iterator over storage items.
	pub fn iter<'a>(&'a self)
		-> impl Iterator<Item=(Option<&'a StorageKey>, &'a StorageKey, Option<&'a StorageData>)> + 'a {
		let top = self.changes
			.iter()
			.filter(move |&(key, _)| match self.filter {
				Some(ref filter) => filter.contains(key),
				None => true,
			})
			.map(move |(k,v)| (None, k, v.as_ref()));
		let children = self.child_changes
			.iter()
			.filter_map(move |(sk, changes)| {
				if let Some(cf) = self.child_filters.as_ref() {
					if let Some(filter) = cf.get(sk) {
						Some(changes
							.iter()
							.filter(move |&(key, _)| match filter {
								Some(ref filter) => filter.contains(key),
								None => true,
							})
							.map(move |(k,v)| (Some(sk), k, v.as_ref())))
					} else { None }
				} else { None	}
			})
			.flatten();
		top.chain(children)
	}
}

/// Type that implements `futures::Stream` of storage change events.
pub type StorageEventStream<H> = TracingUnboundedReceiver<(H, StorageChangeSet)>;

type SubscriberId = u64;

type SubscribersGauge = CounterVec<U64>;

/// Manages storage listeners.
#[derive(Debug)]
pub struct StorageNotifications<Block: BlockT> {
	metrics: Option<SubscribersGauge>,
	next_id: SubscriberId,
	wildcard_listeners: FnvHashSet<SubscriberId>,
	listeners: HashMap<StorageKey, FnvHashSet<SubscriberId>>,
	child_listeners: HashMap<StorageKey, (
		HashMap<StorageKey, FnvHashSet<SubscriberId>>,
		FnvHashSet<SubscriberId>
	)>,
	sinks: FnvHashMap<SubscriberId, (
		TracingUnboundedSender<(Block::Hash, StorageChangeSet)>,
		Option<HashSet<StorageKey>>,
		Option<HashMap<StorageKey, Option<HashSet<StorageKey>>>>,
	)>,
}

impl<Block: BlockT> Default for StorageNotifications<Block> {
	fn default() -> Self {
		Self {
			metrics: Default::default(),
			next_id: Default::default(),
			wildcard_listeners: Default::default(),
			listeners: Default::default(),
			child_listeners: Default::default(),
			sinks: Default::default(),
		}
	}
}

impl<Block: BlockT> StorageNotifications<Block> {
	/// Initialize a new StorageNotifications
	/// optionally pass a prometheus registry to send subscriber metrics to
	pub fn new(prometheus_registry: Option<Registry>) -> Self {
		let metrics = prometheus_registry.and_then(|r|
			CounterVec::new(
				Opts::new(
					"storage_notification_subscribers",
					"Number of subscribers in storage notification sytem"
				),
				&["action"], //added | removed
			).and_then(|g| register(g, &r))
			.ok()
		);

		StorageNotifications {
			metrics,
			next_id: Default::default(),
			wildcard_listeners: Default::default(),
			listeners: Default::default(),
			child_listeners: Default::default(),
			sinks: Default::default(),
		}
	}
	/// Trigger notification to all listeners.
	///
	/// Note the changes are going to be filtered by listener's filter key.
	/// In fact no event might be sent if clients are not interested in the changes.
	pub fn trigger(
		&mut self,
		hash: &Block::Hash,
		changeset: impl Iterator<Item=(Vec<u8>, Option<Vec<u8>>)>,
		child_changeset: impl Iterator<
			Item=(Vec<u8>, impl Iterator<Item=(Vec<u8>, Option<Vec<u8>>)>)
		>,
	) {

		let has_wildcard = !self.wildcard_listeners.is_empty();

		// early exit if no listeners
		if !has_wildcard && self.listeners.is_empty() && self.child_listeners.is_empty() {
			return;
		}

		let mut subscribers = self.wildcard_listeners.clone();
		let mut changes = Vec::new();
		let mut child_changes = Vec::new();

		// Collect subscribers and changes
		for (k, v) in changeset {
			let k = StorageKey(k);
			let listeners = self.listeners.get(&k);

			if let Some(ref listeners) = listeners {
				subscribers.extend(listeners.iter());
			}

			if has_wildcard || listeners.is_some() {
				changes.push((k, v.map(StorageData)));
			}
		}
		for (sk, changeset) in child_changeset {
			let sk = StorageKey(sk);
			if let Some((cl, cw)) = self.child_listeners.get(&sk) {
				let mut changes = Vec::new();
				for (k, v) in changeset {
					let k = StorageKey(k);
					let listeners = cl.get(&k);

					if let Some(ref listeners) = listeners {
						subscribers.extend(listeners.iter());
					}

					subscribers.extend(cw.iter());

					if !cw.is_empty() || listeners.is_some() {
						changes.push((k, v.map(StorageData)));
					}
				}
				if !changes.is_empty() {
					child_changes.push((sk, changes));
				}
			}
		}

		// Don't send empty notifications
		if changes.is_empty() && child_changes.is_empty() {
			return;
		}

		let changes = Arc::new(changes);
		let child_changes = Arc::new(child_changes);
		// Trigger the events

		let to_remove = self.sinks
			.iter()
			.filter_map(|(subscriber, &(ref sink, ref filter, ref child_filters))| {
				let should_remove = {
					if subscribers.contains(subscriber) {
						sink.unbounded_send((hash.clone(), StorageChangeSet {
							changes: changes.clone(),
							child_changes: child_changes.clone(),
							filter: filter.clone(),
							child_filters: child_filters.clone(),
						})).is_err()
					} else {
						sink.is_closed()
					}
				};

				if should_remove {
					Some(subscriber.clone())
				} else {
					None
				}
			}).collect::<Vec<_>>();

		for sub_id in to_remove {
			self.remove_subscriber(sub_id);
		}
	}

	fn remove_subscriber_from(
		subscriber: &SubscriberId,
		filters: &Option<HashSet<StorageKey>>,
		listeners: &mut HashMap<StorageKey, FnvHashSet<SubscriberId>>,
		wildcards: &mut FnvHashSet<SubscriberId>,
	){
		match filters {
			None => {
				wildcards.remove(subscriber);
			},
			Some(filters) => {

				for key in filters.iter() {
					let remove_key = match listeners.get_mut(key) {
						Some(ref mut set) => {
							set.remove(subscriber);
							set.is_empty()
						},
						None => false,
					};

					if remove_key {
						listeners.remove(key);
					}
				}
			}
		}
	}

	fn remove_subscriber(&mut self, subscriber: SubscriberId) {
		if let Some((_, filters, child_filters)) = self.sinks.remove(&subscriber) {
			Self::remove_subscriber_from(
				&subscriber,
				&filters,
				&mut self.listeners,
				&mut self.wildcard_listeners,
			);
			if let Some(child_filters) = child_filters.as_ref() {
				for (c_key, filters) in child_filters {

					if let Some((listeners, wildcards)) = self.child_listeners.get_mut(&c_key) {
						Self::remove_subscriber_from(
							&subscriber,
							&filters,
							&mut *listeners,
							&mut *wildcards,
						);

						if listeners.is_empty() && wildcards.is_empty() {
							self.child_listeners.remove(&c_key);
						}
					}
				}
			}
			if let Some(m) = self.metrics.as_ref() {
				m.with_label_values(&[&"removed"]).inc();
			}
		}
	}

	fn listen_from(
		current_id: SubscriberId,
		filter_keys: &Option<impl AsRef<[StorageKey]>>,
		listeners: &mut HashMap<StorageKey, FnvHashSet<SubscriberId>>,
		wildcards: &mut FnvHashSet<SubscriberId>,
	) -> Option<HashSet<StorageKey>>
	{
		match filter_keys {
			None => {
				wildcards.insert(current_id);
				None
			},
			Some(keys) => Some(keys.as_ref().iter().map(|key| {
				listeners
					.entry(key.clone())
					.or_insert_with(Default::default)
					.insert(current_id);
				key.clone()
			}).collect())
		}
	}

	/// Start listening for particular storage keys.
	pub fn listen(
		&mut self,
		filter_keys: Option<&[StorageKey]>,
		filter_child_keys: Option<&[(StorageKey, Option<Vec<StorageKey>>)]>,
	) -> StorageEventStream<Block::Hash> {
		self.next_id += 1;
		let current_id = self.next_id;

		// add subscriber for every key
		let keys = Self::listen_from(
			current_id,
			&filter_keys,
			&mut self.listeners,
			&mut self.wildcard_listeners,
		);
		let child_keys = filter_child_keys.map(|filter_child_keys| {
			filter_child_keys.iter().map(|(c_key, o_keys)| {
				let (c_listeners, c_wildcards) = self.child_listeners
					.entry(c_key.clone())
					.or_insert_with(Default::default);

				(c_key.clone(), Self::listen_from(
					current_id,
					o_keys,
					&mut *c_listeners,
					&mut *c_wildcards,
				))
			}).collect()
		});


		// insert sink
		let (tx, rx) = tracing_unbounded("mpsc_storage_notification_items");
		self.sinks.insert(current_id, (tx, keys, child_keys));

		if let Some(m) = self.metrics.as_ref() {
			m.with_label_values(&[&"added"]).inc();
		}

		rx
	}
}