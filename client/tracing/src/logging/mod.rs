// This file is part of Substrate.

// Copyright (C) 2021 Parity Technologies (UK) Ltd.
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

//! Substrate logging library.
//!
//! This crate uses tokio's [tracing](https://github.com/tokio-rs/tracing/) library for logging.

#![warn(missing_docs)]

mod directives;
mod event_format;
mod layers;

pub use directives::*;
pub use sc_tracing_proc_macro::*;

use std::io;
use tracing::Subscriber;
use tracing_subscriber::{
	filter::LevelFilter,
	fmt::time::ChronoLocal,
	fmt::{
		format, FormatEvent, FormatFields, Formatter, Layer as FmtLayer, MakeWriter,
		SubscriberBuilder,
	},
	layer::{self, SubscriberExt},
	registry::LookupSpan,
	EnvFilter, FmtSubscriber, Layer, Registry,
};

pub use event_format::*;
pub use layers::*;

/// Logging Result typedef.
pub type Result<T> = std::result::Result<T, Error>;

/// Logging errors.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
#[non_exhaustive]
#[error(transparent)]
pub enum Error {
	IoError(#[from] io::Error),
	SetGlobalDefaultError(#[from] tracing::subscriber::SetGlobalDefaultError),
	DirectiveParseError(#[from] tracing_subscriber::filter::ParseError),
	SetLoggerError(#[from] tracing_log::log_tracer::SetLoggerError),
}

macro_rules! enable_log_reloading {
	($builder:expr) => {{
		let builder = $builder.with_filter_reloading();
		let handle = builder.reload_handle();
		set_reload_handle(handle);
		builder
	}};
}

/// Common implementation to get the subscriber.
fn prepare_subscriber<N, E, F, W>(
	directives: &str,
	profiling_targets: Option<&str>,
	force_colors: Option<bool>,
	builder_hook: impl Fn(
		SubscriberBuilder<
			format::DefaultFields,
			EventFormat<ChronoLocal>,
			EnvFilter,
			fn() -> std::io::Stderr,
		>,
	) -> SubscriberBuilder<N, E, F, W>,
) -> Result<impl Subscriber + for<'a> LookupSpan<'a>>
where
	N: for<'writer> FormatFields<'writer> + 'static,
	E: FormatEvent<Registry, N> + 'static,
	W: MakeWriter + 'static,
	F: layer::Layer<Formatter<N, E, W>> + Send + Sync + 'static,
	FmtLayer<Registry, N, E, W>: layer::Layer<Registry> + Send + Sync + 'static,
{
	// Accept all valid directives and print invalid ones
	fn parse_user_directives(mut env_filter: EnvFilter, dirs: &str) -> Result<EnvFilter> {
		for dir in dirs.split(',') {
			env_filter = env_filter.add_directive(parse_default_directive(&dir)?);
		}
		Ok(env_filter)
	}

	// Initialize filter - ensure to use `parse_default_directive` for any defaults to persist
	// after log filter reloading by RPC
	let mut env_filter = EnvFilter::default()
		// Enable info
		.add_directive(parse_default_directive("info").expect("provided directive is valid"))
		// Disable info logging by default for some modules.
		.add_directive(parse_default_directive("ws=off").expect("provided directive is valid"))
		.add_directive(parse_default_directive("yamux=off").expect("provided directive is valid"))
		.add_directive(
			parse_default_directive("cranelift_codegen=off").expect("provided directive is valid"),
		)
		// Set warn logging by default for some modules.
		.add_directive(
			parse_default_directive("cranelift_wasm=warn").expect("provided directive is valid"),
		)
		.add_directive(parse_default_directive("hyper=warn").expect("provided directive is valid"));

	if let Ok(lvl) = std::env::var("RUST_LOG") {
		if lvl != "" {
			env_filter = parse_user_directives(env_filter, &lvl)?;
		}
	}

	if directives != "" {
		env_filter = parse_user_directives(env_filter, directives)?;
	}

	if let Some(profiling_targets) = profiling_targets {
		env_filter = parse_user_directives(env_filter, profiling_targets)?;
		env_filter = env_filter.add_directive(
			parse_default_directive("sc_tracing=trace").expect("provided directive is valid"),
		);
	}

	let max_level_hint = Layer::<FmtSubscriber>::max_level_hint(&env_filter);

	let max_level = match max_level_hint {
		Some(LevelFilter::INFO) | None => log::LevelFilter::Info,
		Some(LevelFilter::TRACE) => log::LevelFilter::Trace,
		Some(LevelFilter::WARN) => log::LevelFilter::Warn,
		Some(LevelFilter::ERROR) => log::LevelFilter::Error,
		Some(LevelFilter::DEBUG) => log::LevelFilter::Debug,
		Some(LevelFilter::OFF) => log::LevelFilter::Off,
	};

	tracing_log::LogTracer::builder()
		.with_max_level(max_level)
		.init()?;

	// If we're only logging `INFO` entries then we'll use a simplified logging format.
	let simple = match max_level_hint {
		Some(level) if level <= tracing_subscriber::filter::LevelFilter::INFO => true,
		_ => false,
	};

	let enable_color = force_colors.unwrap_or_else(|| atty::is(atty::Stream::Stderr));
	let timer = ChronoLocal::with_format(if simple {
		"%Y-%m-%d %H:%M:%S".to_string()
	} else {
		"%Y-%m-%d %H:%M:%S%.3f".to_string()
	});

	let event_format = EventFormat {
		timer,
		display_target: !simple,
		display_level: !simple,
		display_thread_name: !simple,
		enable_color,
		dup_to_stdout: !atty::is(atty::Stream::Stderr) && atty::is(atty::Stream::Stdout),
	};
	let builder = FmtSubscriber::builder().with_env_filter(env_filter);

	#[cfg(not(target_os = "unknown"))]
	let builder = builder.with_writer(std::io::stderr as _);

	#[cfg(target_os = "unknown")]
	let builder = builder.with_writer(std::io::sink);

	#[cfg(not(target_os = "unknown"))]
	let builder = builder.event_format(event_format);

	#[cfg(not(target_os = "unknown"))]
	let builder = builder_hook(builder);

	let subscriber = builder.finish().with(PrefixLayer);

	#[cfg(target_os = "unknown")]
	let subscriber = subscriber.with(ConsoleLogLayer::new(event_format));

	Ok(subscriber)
}

/// A builder that is used to initialize the global logger.
pub struct LoggerBuilder {
	directives: String,
	profiling: Option<(crate::TracingReceiver, String)>,
	log_reloading: bool,
	force_colors: Option<bool>,
}

impl LoggerBuilder {
	/// Create a new [`LoggerBuilder`] which can be used to initialize the global logger.
	pub fn new<S: Into<String>>(directives: S) -> Self {
		Self {
			directives: directives.into(),
			profiling: None,
			log_reloading: true,
			force_colors: None,
		}
	}

	/// Set up the profiling.
	pub fn with_profiling<S: Into<String>>(
		&mut self,
		tracing_receiver: crate::TracingReceiver,
		profiling_targets: S,
	) -> &mut Self {
		self.profiling = Some((tracing_receiver, profiling_targets.into()));
		self
	}

	/// Wether or not to disable log reloading.
	pub fn with_log_reloading(&mut self, enabled: bool) -> &mut Self {
		self.log_reloading = enabled;
		self
	}

	/// Force enable/disable colors.
	pub fn with_colors(&mut self, enable: bool) -> &mut Self {
		self.force_colors = Some(enable);
		self
	}

	/// Initialize the global logger
	///
	/// This sets various global logging and tracing instances and thus may only be called once.
	pub fn init(self) -> Result<()> {
		if let Some((tracing_receiver, profiling_targets)) = self.profiling {
			if self.log_reloading {
				let subscriber = prepare_subscriber(
					&self.directives,
					Some(&profiling_targets),
					self.force_colors,
					|builder| enable_log_reloading!(builder),
				)?;
				let profiling = crate::ProfilingLayer::new(tracing_receiver, &profiling_targets);

				tracing::subscriber::set_global_default(subscriber.with(profiling))?;

				Ok(())
			} else {
				let subscriber = prepare_subscriber(
					&self.directives,
					Some(&profiling_targets),
					self.force_colors,
					|builder| builder,
				)?;
				let profiling = crate::ProfilingLayer::new(tracing_receiver, &profiling_targets);

				tracing::subscriber::set_global_default(subscriber.with(profiling))?;

				Ok(())
			}
		} else {
			if self.log_reloading {
				let subscriber = prepare_subscriber(
					&self.directives,
					None,
					self.force_colors,
					|builder| enable_log_reloading!(builder),
				)?;

				tracing::subscriber::set_global_default(subscriber)?;

				Ok(())
			} else {
				let subscriber = prepare_subscriber(
					&self.directives,
					None,
					self.force_colors,
					|builder| builder,
				)?;

				tracing::subscriber::set_global_default(subscriber)?;

				Ok(())
			}
		}
	}
}


