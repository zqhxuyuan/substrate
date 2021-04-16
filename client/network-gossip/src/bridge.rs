// This file is part of Substrate.

// Copyright (C) 2019-2021 Parity Technologies (UK) Ltd.
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

use crate::{Network, Validator};
use crate::state_machine::{ConsensusGossip, TopicNotification, PERIODIC_MAINTENANCE_INTERVAL};

use sc_network::{Event, ReputationChange};

use futures::prelude::*;
use futures::channel::mpsc::{channel, Sender, Receiver};
use libp2p::PeerId;
use log::trace;
use prometheus_endpoint::Registry;
use sp_runtime::traits::Block as BlockT;
use std::{
	borrow::Cow,
	collections::{HashMap, VecDeque},
	pin::Pin,
	sync::Arc,
	task::{Context, Poll},
};

/// Wraps around an implementation of the `Network` crate and provides gossiping capabilities on
/// top of it.
pub struct GossipEngine<B: BlockT> {
	state_machine: ConsensusGossip<B>,
	network: Box<dyn Network<B> + Send>,
	periodic_maintenance_interval: futures_timer::Delay,
	protocol: Cow<'static, str>,

	/// Incoming events from the network.
	network_event_stream: Pin<Box<dyn Stream<Item = Event> + Send>>,
	/// Outgoing events to the consumer.
	message_sinks: HashMap<B::Hash, Vec<Sender<TopicNotification>>>,
	/// Buffered messages (see [`ForwardingState`]).
	forwarding_state: ForwardingState<B>,
}

/// A gossip engine receives messages from the network via the `network_event_stream` and forwards
/// them to upper layers via the `message sinks`. In the scenario where messages have been received
/// from the network but a subscribed message sink is not yet ready to receive the messages, the
/// messages are buffered. To model this process a gossip engine can be in two states.
enum ForwardingState<B: BlockT> {
	/// The gossip engine is currently not forwarding any messages and will poll the network for
	/// more messages to forward.
	Idle,
	/// The gossip engine is in the progress of forwarding messages and thus will not poll the
	/// network for more messages until it has send all current messages into the subscribed message
	/// sinks.
	Busy(VecDeque<(B::Hash, TopicNotification)>),
}

impl<B: BlockT> Unpin for GossipEngine<B> {}

impl<B: BlockT> GossipEngine<B> {
	/// Create a new instance.
	pub fn new<N: Network<B> + Send + Clone + 'static>(
		network: N,
		protocol: impl Into<Cow<'static, str>>,
		validator: Arc<dyn Validator<B>>,
		metrics_registry: Option<&Registry>,
	) -> Self where B: 'static {
		let protocol = protocol.into();
		let network_event_stream = network.event_stream();

		GossipEngine {
			state_machine: ConsensusGossip::new(validator, protocol.clone(), metrics_registry),
			network: Box::new(network),
			periodic_maintenance_interval: futures_timer::Delay::new(PERIODIC_MAINTENANCE_INTERVAL),
			protocol,

			network_event_stream,
			message_sinks: HashMap::new(),
			forwarding_state: ForwardingState::Idle,
		}
	}

	pub fn report(&self, who: PeerId, reputation: ReputationChange) {
		self.network.report_peer(who, reputation);
	}

	/// Registers a message without propagating it to any peers. The message
	/// becomes available to new peers or when the service is asked to gossip
	/// the message's topic. No validation is performed on the message, if the
	/// message is already expired it should be dropped on the next garbage
	/// collection.
	pub fn register_gossip_message(
		&mut self,
		topic: B::Hash,
		message: Vec<u8>,
	) {
		self.state_machine.register_message(topic, message);
	}

	/// Broadcast all messages with given topic.
	pub fn broadcast_topic(&mut self, topic: B::Hash, force: bool) {
		self.state_machine.broadcast_topic(&mut *self.network, topic, force);
	}

	/// Get data of valid, incoming messages for a topic (but might have expired meanwhile).
	pub fn messages_for(&mut self, topic: B::Hash)
		-> Receiver<TopicNotification>
	{
		let past_messages = self.state_machine.messages_for(topic).collect::<Vec<_>>();
		// The channel length is not critical for correctness. By the implementation of `channel`
		// each sender is guaranteed a single buffer slot, making it a non-rendezvous channel and
		// thus preventing direct dead-locks. A minimum channel length of 10 is an estimate based on
		// the fact that despite `NotificationsReceived` having a `Vec` of messages, it only ever
		// contains a single message.
		let (mut tx, rx) = channel(usize::max(past_messages.len(), 10));

		for notification in past_messages{
			tx.try_send(notification)
				.expect("receiver known to be live, and buffer size known to suffice; qed");
		}

		self.message_sinks.entry(topic).or_default().push(tx);

		rx
	}

	/// Send all messages with given topic to a peer.
	pub fn send_topic(
		&mut self,
		who: &PeerId,
		topic: B::Hash,
		force: bool
	) {
		self.state_machine.send_topic(&mut *self.network, who, topic, force)
	}

	/// Multicast a message to all peers.
	pub fn gossip_message(
		&mut self,
		topic: B::Hash,
		message: Vec<u8>,
		force: bool,
	) {
		self.state_machine.multicast(&mut *self.network, topic, message, force)
	}

	/// Send addressed message to the given peers. The message is not kept or multicast
	/// later on.
	pub fn send_message(&mut self, who: Vec<sc_network::PeerId>, data: Vec<u8>) {
		for who in &who {
			self.state_machine.send_message(&mut *self.network, who, data.clone());
		}
	}

	/// Notify everyone we're connected to that we have the given block.
	///
	/// Note: this method isn't strictly related to gossiping and should eventually be moved
	/// somewhere else.
	pub fn announce(&self, block: B::Hash, associated_data: Option<Vec<u8>>) {
		self.network.announce(block, associated_data);
	}
}

impl<B: BlockT> Future for GossipEngine<B> {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		let this = &mut *self;

		'outer: loop {
			match &mut this.forwarding_state {
				ForwardingState::Idle => {
					match this.network_event_stream.poll_next_unpin(cx) {
						Poll::Ready(Some(event)) => match event {
							Event::SyncConnected { remote } => {
								this.network.add_set_reserved(remote, this.protocol.clone());
							}
							Event::SyncDisconnected { remote } => {
								this.network.remove_set_reserved(remote, this.protocol.clone());
							}
							Event::NotificationStreamOpened { remote, protocol, role } => {
								if protocol != this.protocol {
									continue;
								}
								this.state_machine.new_peer(&mut *this.network, remote, role);
							}
							Event::NotificationStreamClosed { remote, protocol } => {
								if protocol != this.protocol {
									continue;
								}
								this.state_machine.peer_disconnected(&mut *this.network, remote);
							},
							Event::NotificationsReceived { remote, messages } => {
								let messages = messages.into_iter().filter_map(|(engine, data)| {
									if engine == this.protocol {
										Some(data.to_vec())
									} else {
										None
									}
								}).collect();

								let to_forward = this.state_machine.on_incoming(
									&mut *this.network,
									remote,
									messages,
								);

								this.forwarding_state = ForwardingState::Busy(to_forward.into());
							},
							Event::Dht(_) => {}
						}
						// The network event stream closed. Do the same for [`GossipValidator`].
						Poll::Ready(None) => return Poll::Ready(()),
						Poll::Pending => break,
					}
				}
				ForwardingState::Busy(to_forward) => {
					let (topic, notification) = match to_forward.pop_front() {
						Some(n) => n,
						None => {
							this.forwarding_state = ForwardingState::Idle;
							continue;
						}
					};

					let sinks = match this.message_sinks.get_mut(&topic) {
						Some(sinks) => sinks,
						None => {
							continue;
						},
					};

					// Make sure all sinks for the given topic are ready.
					for sink in sinks.iter_mut() {
						match sink.poll_ready(cx) {
							Poll::Ready(Ok(())) => {},
							// Receiver has been dropped. Ignore for now, filtered out in (1).
							Poll::Ready(Err(_)) => {},
							Poll::Pending => {
								// Push back onto queue for later.
								to_forward.push_front((topic, notification));
								break 'outer;
							}
						}
					}

					// Filter out all closed sinks.
					sinks.retain(|sink| !sink.is_closed()); // (1)

					if sinks.is_empty() {
						this.message_sinks.remove(&topic);
						continue;
					}

					trace!(
						target: "gossip",
						"Pushing consensus message to sinks for {}.", topic,
					);

					// Send the notification on each sink.
					for sink in sinks {
						match sink.start_send(notification.clone()) {
							Ok(()) => {},
							Err(e) if e.is_full() => unreachable!(
								"Previously ensured that all sinks are ready; qed.",
							),
							// Receiver got dropped. Will be removed in next iteration (See (1)).
							Err(_) => {},
						}
					}
				}
			}
		}


		while let Poll::Ready(()) = this.periodic_maintenance_interval.poll_unpin(cx) {
			this.periodic_maintenance_interval.reset(PERIODIC_MAINTENANCE_INTERVAL);
			this.state_machine.tick(&mut *this.network);

			this.message_sinks.retain(|_, sinks| {
				sinks.retain(|sink| !sink.is_closed());
				!sinks.is_empty()
			});
		}

		Poll::Pending
	}
}

