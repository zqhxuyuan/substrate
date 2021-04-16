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

//! Instrumentation implementation for substrate.
//!
//! This crate is unstable and the API and usage may change.
//!
//! # Usage
//!
//! See `sp-tracing` for examples on how to use tracing.
//!
//! Currently we only provide `Log` (default).

#![warn(missing_docs)]

pub mod logging;

use rustc_hash::FxHashMap;
use std::fmt;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use serde::ser::{Serialize, Serializer, SerializeMap};
use tracing::{
	event::Event,
	field::{Visit, Field},
	Level,
	span::{Attributes, Id, Record},
	subscriber::Subscriber,
};
use tracing_subscriber::{
	CurrentSpan,
	layer::{Layer, Context},
};
use sp_tracing::{WASM_NAME_KEY, WASM_TARGET_KEY, WASM_TRACE_IDENTIFIER};

#[doc(hidden)]
pub use tracing;

const ZERO_DURATION: Duration = Duration::from_nanos(0);

/// Responsible for assigning ids to new spans, which are not re-used.
pub struct ProfilingLayer {
	targets: Vec<(String, Level)>,
	trace_handler: Box<dyn TraceHandler>,
	span_data: Mutex<FxHashMap<Id, SpanDatum>>,
	current_span: CurrentSpan,
}

/// Used to configure how to receive the metrics
#[derive(Debug, Clone)]
pub enum TracingReceiver {
	/// Output to logger
	Log,
}

impl Default for TracingReceiver {
	fn default() -> Self {
		Self::Log
	}
}

/// A handler for tracing `SpanDatum`
pub trait TraceHandler: Send + Sync {
	/// Process a `SpanDatum`
	fn handle_span(&self, span: SpanDatum);
	/// Process a `TraceEvent`
	fn handle_event(&self, event: TraceEvent);
}

/// Represents a tracing event, complete with values
#[derive(Debug)]
pub struct TraceEvent {
	/// Name of the event.
	pub name: &'static str,
	/// Target of the event.
	pub target: String,
	/// Level of the event.
	pub level: Level,
	/// Values for this event.
	pub values: Values,
	/// Id of the parent tracing event, if any.
	pub parent_id: Option<Id>,
}

/// Represents a single instance of a tracing span
#[derive(Debug)]
pub struct SpanDatum {
	/// id for this span
	pub id: Id,
	/// id of the parent span, if any
	pub parent_id: Option<Id>,
	/// Name of this span
	pub name: String,
	/// Target, typically module
	pub target: String,
	/// Tracing Level - ERROR, WARN, INFO, DEBUG or TRACE
	pub level: Level,
	/// Line number in source
	pub line: u32,
	/// Time that the span was last entered
	pub start_time: Instant,
	/// Total duration of span while entered
	pub overall_time: Duration,
	/// Values recorded to this span
	pub values: Values,
}

/// Holds associated values for a tracing span
#[derive(Default, Clone, Debug)]
pub struct Values {
	/// HashMap of `bool` values
	pub bool_values: FxHashMap<String, bool>,
	/// HashMap of `i64` values
	pub i64_values: FxHashMap<String, i64>,
	/// HashMap of `u64` values
	pub u64_values: FxHashMap<String, u64>,
	/// HashMap of `String` values
	pub string_values: FxHashMap<String, String>,
}

impl Values {
	/// Returns a new instance of Values
	pub fn new() -> Self {
		Default::default()
	}

	/// Checks if all individual collections are empty
	pub fn is_empty(&self) -> bool {
		self.bool_values.is_empty() &&
			self.i64_values.is_empty() &&
			self.u64_values.is_empty() &&
			self.string_values.is_empty()
	}
}

impl Visit for Values {
	fn record_i64(&mut self, field: &Field, value: i64) {
		self.i64_values.insert(field.name().to_string(), value);
	}

	fn record_u64(&mut self, field: &Field, value: u64) {
		self.u64_values.insert(field.name().to_string(), value);
	}

	fn record_bool(&mut self, field: &Field, value: bool) {
		self.bool_values.insert(field.name().to_string(), value);
	}

	fn record_str(&mut self, field: &Field, value: &str) {
		self.string_values.insert(field.name().to_string(), value.to_owned());
	}

	fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
		self.string_values.insert(field.name().to_string(), format!("{:?}", value).to_owned());
	}
}

impl Serialize for Values {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
		where S: Serializer,
	{
		let len = self.bool_values.len() + self.i64_values.len() + self.u64_values.len() + self.string_values.len();
		let mut map = serializer.serialize_map(Some(len))?;
		for (k, v) in &self.bool_values {
			map.serialize_entry(k, v)?;
		}
		for (k, v) in &self.i64_values {
			map.serialize_entry(k, v)?;
		}
		for (k, v) in &self.u64_values {
			map.serialize_entry(k, v)?;
		}
		for (k, v) in &self.string_values {
			map.serialize_entry(k, v)?;
		}
		map.end()
	}
}

impl fmt::Display for Values {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let bool_iter = self.bool_values.iter().map(|(k, v)| format!("{}={}", k, v));
		let i64_iter = self.i64_values.iter().map(|(k, v)| format!("{}={}", k, v));
		let u64_iter = self.u64_values.iter().map(|(k, v)| format!("{}={}", k, v));
		let string_iter = self.string_values.iter().map(|(k, v)| format!("{}=\"{}\"", k, v));
		let values = bool_iter.chain(i64_iter).chain(u64_iter).chain(string_iter).collect::<Vec<String>>().join(", ");
		write!(f, "{}", values)
	}
}

impl ProfilingLayer {
	/// Takes a `TracingReceiver` and a comma separated list of targets,
	/// either with a level: "pallet=trace,frame=debug"
	/// or without: "pallet,frame" in which case the level defaults to `trace`.
	/// wasm_tracing indicates whether to enable wasm traces
	pub fn new(receiver: TracingReceiver, targets: &str) -> Self {
		match receiver {
			TracingReceiver::Log => Self::new_with_handler(Box::new(LogTraceHandler), targets),
		}
	}

	/// Allows use of a custom TraceHandler to create a new instance of ProfilingSubscriber.
	/// Takes a comma separated list of targets,
	/// either with a level, eg: "pallet=trace"
	/// or without: "pallet" in which case the level defaults to `trace`.
	/// wasm_tracing indicates whether to enable wasm traces
	pub fn new_with_handler(trace_handler: Box<dyn TraceHandler>, targets: &str) -> Self {
		let targets: Vec<_> = targets.split(',').map(|s| parse_target(s)).collect();
		Self {
			targets,
			trace_handler,
			span_data: Mutex::new(FxHashMap::default()),
			current_span: Default::default(),
		}
	}

	fn check_target(&self, target: &str, level: &Level) -> bool {
		for t in &self.targets {
			if target.starts_with(t.0.as_str()) && level <= &t.1 {
				return true;
			}
		}
		false
	}
}

// Default to TRACE if no level given or unable to parse Level
// We do not support a global `Level` currently
fn parse_target(s: &str) -> (String, Level) {
	match s.find('=') {
		Some(i) => {
			let target = s[0..i].to_string();
			if s.len() > i {
				let level = s[i + 1..s.len()].parse::<Level>().unwrap_or(Level::TRACE);
				(target, level)
			} else {
				(target, Level::TRACE)
			}
		}
		None => (s.to_string(), Level::TRACE)
	}
}

impl<S: Subscriber> Layer<S> for ProfilingLayer {
	fn new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<S>) {
		let mut values = Values::default();
		attrs.record(&mut values);
		let span_datum = SpanDatum {
			id: id.clone(),
			parent_id: attrs.parent().cloned().or_else(|| self.current_span.id()),
			name: attrs.metadata().name().to_owned(),
			target: attrs.metadata().target().to_owned(),
			level: attrs.metadata().level().clone(),
			line: attrs.metadata().line().unwrap_or(0),
			start_time: Instant::now(),
			overall_time: ZERO_DURATION,
			values,
		};
		self.span_data.lock().insert(id.clone(), span_datum);
	}

	fn on_record(&self, span: &Id, values: &Record<'_>, _ctx: Context<S>) {
		let mut span_data = self.span_data.lock();
		if let Some(s) = span_data.get_mut(span) {
			values.record(&mut s.values);
		}
	}

	fn on_event(&self, event: &Event<'_>, _ctx: Context<S>) {
		let mut values = Values::default();
		event.record(&mut values);
		let trace_event = TraceEvent {
			name: event.metadata().name(),
			target: event.metadata().target().to_owned(),
			level: event.metadata().level().clone(),
			values,
			parent_id: event.parent().cloned().or_else(|| self.current_span.id()),
		};
		self.trace_handler.handle_event(trace_event);
	}

	fn on_enter(&self, span: &Id, _ctx: Context<S>) {
		self.current_span.enter(span.clone());
		let mut span_data = self.span_data.lock();
		let start_time = Instant::now();
		if let Some(mut s) = span_data.get_mut(&span) {
			s.start_time = start_time;
		}
	}

	fn on_exit(&self, span: &Id, _ctx: Context<S>) {
		self.current_span.exit();
		let end_time = Instant::now();
		let span_datum = {
			let mut span_data = self.span_data.lock();
			span_data.remove(&span)
		};

		if let Some(mut span_datum) = span_datum {
			span_datum.overall_time += end_time - span_datum.start_time;
			if span_datum.name == WASM_TRACE_IDENTIFIER {
				span_datum.values.bool_values.insert("wasm".to_owned(), true);
				if let Some(n) = span_datum.values.string_values.remove(WASM_NAME_KEY) {
					span_datum.name = n;
				}
				if let Some(t) = span_datum.values.string_values.remove(WASM_TARGET_KEY) {
					span_datum.target = t;
				}
				if self.check_target(&span_datum.target, &span_datum.level) {
					self.trace_handler.handle_span(span_datum);
				}
			} else {
				self.trace_handler.handle_span(span_datum);
			}
		};
	}

	fn on_close(&self, span: Id, ctx: Context<S>) {
		self.on_exit(&span, ctx)
	}
}

/// TraceHandler for sending span data to the logger
pub struct LogTraceHandler;

fn log_level(level: Level) -> log::Level {
	match level {
		Level::TRACE => log::Level::Trace,
		Level::DEBUG => log::Level::Debug,
		Level::INFO => log::Level::Info,
		Level::WARN => log::Level::Warn,
		Level::ERROR => log::Level::Error,
	}
}

impl TraceHandler for LogTraceHandler {
	fn handle_span(&self, span_datum: SpanDatum) {
		if span_datum.values.is_empty() {
			log::log!(
				log_level(span_datum.level),
				"{}: {}, time: {}, id: {}, parent_id: {:?}",
				span_datum.target,
				span_datum.name,
				span_datum.overall_time.as_nanos(),
				span_datum.id.into_u64(),
				span_datum.parent_id.map(|s| s.into_u64()),
			);
		} else {
			log::log!(
				log_level(span_datum.level),
				"{}: {}, time: {}, id: {}, parent_id: {:?}, values: {}",
				span_datum.target,
				span_datum.name,
				span_datum.overall_time.as_nanos(),
				span_datum.id.into_u64(),
				span_datum.parent_id.map(|s| s.into_u64()),
				span_datum.values,
			);
		}
	}

	fn handle_event(&self, event: TraceEvent) {
		log::log!(
			log_level(event.level),
			"{}, parent_id: {:?}, {}",
			event.target,
			event.parent_id.map(|s| s.into_u64()),
			event.values,
		);
	}
}

