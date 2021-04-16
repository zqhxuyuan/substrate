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

//! Chain Spec extensions helpers.

use std::fmt::Debug;
use std::any::{TypeId, Any};

use std::collections::BTreeMap;

use serde::{Serialize, Deserialize, de::DeserializeOwned};

/// A `ChainSpec` extension.
///
/// This trait is implemented automatically by `ChainSpecGroup` macro.
pub trait Group: Clone + Sized {
	/// An associated type containing fork definition.
	type Fork: Fork<Base=Self>;

	/// Convert to fork type.
	fn to_fork(self) -> Self::Fork;
}

/// A `ChainSpec` extension fork definition.
///
/// Basically should look the same as `Group`, but
/// all parameters are optional. This allows changing
/// only one parameter as part of the fork.
/// The forks can be combined (summed up) to specify
/// a complete set of parameters
pub trait Fork: Serialize + DeserializeOwned + Clone + Sized {
	/// A base `Group` type.
	type Base: Group<Fork=Self>;

	/// Combine with another struct.
	///
	/// All parameters set in `other` should override the
	/// ones in the current struct.
	fn combine_with(&mut self, other: Self);

	/// Attempt to convert to the base type if all parameters are set.
	fn to_base(self) -> Option<Self::Base>;
}

macro_rules! impl_trivial {
	() => {};
	($A : ty) => {
		impl_trivial!($A ,);
	};
	($A : ty , $( $B : ty ),*) => {
		impl_trivial!($( $B ),*);

		impl Group for $A {
			type Fork = $A;

			fn to_fork(self) -> Self::Fork {
				self
			}
		}

		impl Fork for $A {
			type Base = $A;

			fn combine_with(&mut self, other: Self) {
				*self = other;
			}

			fn to_base(self) -> Option<Self::Base> {
				Some(self)
			}
		}
	}
}

impl_trivial!((), u8, u16, u32, u64, usize, String, Vec<u8>);

impl<T: Group> Group for Option<T> {
	type Fork = Option<T::Fork>;

	fn to_fork(self) -> Self::Fork {
		self.map(|a| a.to_fork())
	}
}

impl<T: Fork> Fork for Option<T> {
	type Base = Option<T::Base>;

	fn combine_with(&mut self, other: Self) {
		*self = match (self.take(), other) {
			(Some(mut a), Some(b)) => {
				a.combine_with(b);
				Some(a)
			},
			(a, b) => a.or(b),
		};
	}

	fn to_base(self) -> Option<Self::Base> {
		self.map(|x| x.to_base())
	}
}

/// A collection of `ChainSpec` extensions.
///
/// This type can be passed around and allows the core
/// modules to request a strongly-typed, but optional configuration.
pub trait Extension: Serialize + DeserializeOwned + Clone {
	type Forks: IsForks;

	/// Get an extension of specific type.
	fn get<T: 'static>(&self) -> Option<&T>;
	/// Get an extension of specific type as refernce to `Any`
	fn get_any(&self, t: TypeId) -> &dyn Any;

	/// Get forkable extensions of specific type.
	fn forks<BlockNumber, T>(&self) -> Option<Forks<BlockNumber, T>> where
		BlockNumber: Ord + Clone + 'static,
		T: Group + 'static,
		<Self::Forks as IsForks>::Extension: Extension,
		<<Self::Forks as IsForks>::Extension as Group>::Fork: Extension,
	{
		self.get::<Forks<BlockNumber, <Self::Forks as IsForks>::Extension>>()?
			.for_type()
	}
}

impl Extension for crate::NoExtension {
	type Forks = Self;

	fn get<T: 'static>(&self) -> Option<&T> { None }
	fn get_any(&self, _t: TypeId) -> &dyn Any { self }
}

pub trait IsForks {
	type BlockNumber: Ord + 'static;
	type Extension: Group + 'static;
}

impl IsForks for Option<()> {
	type BlockNumber = u64;
	type Extension = Self;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Forks<BlockNumber: Ord, T: Group> {
	forks: BTreeMap<BlockNumber, T::Fork>,
	#[serde(flatten)]
	base: T,
}

impl<B: Ord, T: Group + Default> Default for Forks<B, T> {
	fn default() -> Self {
		Self {
			base: Default::default(),
			forks: Default::default(),
		}
	}
}

impl<B: Ord, T: Group> Forks<B, T> where
	T::Fork: Debug,
{
	/// Create new fork definition given the base and the forks.
	pub fn new(base: T, forks: BTreeMap<B, T::Fork>) -> Self {
		Self { base, forks }
	}

	/// Return a set of parameters for `Group` including all forks up to `block` (inclusive).
	pub fn at_block(&self, block: B) -> T {
		let mut start = self.base.clone().to_fork();

		for (_, fork) in self.forks.range(..=block) {
			start.combine_with(fork.clone());
		}

		start
			.to_base()
			.expect("We start from the `base` object, so it's always fully initialized; qed")
	}
}

impl<B, T> IsForks for Forks<B, T> where
	B: Ord + 'static,
	T: Group + 'static,
{
	type BlockNumber = B;
	type Extension = T;
}

impl<B: Ord + Clone, T: Group + Extension> Forks<B, T> where
	T::Fork: Extension,
{
	/// Get forks definition for a subset of this extension.
	///
	/// Returns the `Forks` struct, but limited to a particular type
	/// within the extension.
	pub fn for_type<X>(&self) -> Option<Forks<B, X>> where
		X: Group + 'static,
	{
		let base = self.base.get::<X>()?.clone();
		let forks = self.forks.iter().filter_map(|(k, v)| {
			Some((k.clone(), v.get::<Option<X::Fork>>()?.clone()?))
		}).collect();

		Some(Forks {
			base,
			forks,
		})
	}
}

impl<B, E> Extension for Forks<B, E> where
	B: Serialize + DeserializeOwned + Ord + Clone + 'static,
	E: Extension + Group + 'static,
{
	type Forks = Self;

	fn get<T: 'static>(&self) -> Option<&T> {
		match TypeId::of::<T>() {
			x if x == TypeId::of::<E>() => Any::downcast_ref(&self.base),
			_ => self.base.get(),
		}
	}

	fn get_any(&self, t: TypeId) -> &dyn Any {
		match t {
			x if x == TypeId::of::<E>() => &self.base,
			_ => self.base.get_any(t),
		}
	}

	fn forks<BlockNumber, T>(&self) -> Option<Forks<BlockNumber, T>> where
		BlockNumber: Ord + Clone + 'static,
		T: Group + 'static,
		<Self::Forks as IsForks>::Extension: Extension,
		<<Self::Forks as IsForks>::Extension as Group>::Fork: Extension,
	{
		if TypeId::of::<BlockNumber>() == TypeId::of::<B>() {
			Any::downcast_ref(&self.for_type::<T>()?).cloned()
		} else {
			self.get::<Forks<BlockNumber, <Self::Forks as IsForks>::Extension>>()?
				.for_type()
		}
	}
}

/// A subset if the `Extension` trait that only allows for quering extensions.
pub trait GetExtension {
	/// Get an extension of specific type.
	fn get_any(&self, t: TypeId) -> &dyn Any;
}

impl <E: Extension> GetExtension for E {
	fn get_any(&self, t: TypeId) -> &dyn Any {
		Extension::get_any(self, t)
	}
}

/// Helper function that queries an extension by type from `GetExtension`
/// trait object.
pub fn get_extension<T: 'static>(e: &dyn GetExtension) -> Option<&T> {
	Any::downcast_ref(GetExtension::get_any(e, TypeId::of::<T>()))
}
