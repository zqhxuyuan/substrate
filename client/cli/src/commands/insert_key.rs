// This file is part of Substrate.

// Copyright (C) 2020-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Implementation of the `insert` subcommand

use crate::{
	Error, KeystoreParams, CryptoSchemeFlag, SharedParams, utils, with_crypto_scheme,
	SubstrateCli,
};
use std::{sync::Arc, convert::TryFrom};
use structopt::StructOpt;
use sp_core::{crypto::KeyTypeId, crypto::SecretString};
use sp_keystore::{SyncCryptoStorePtr, SyncCryptoStore};
use sc_keystore::LocalKeystore;
use sc_service::config::{KeystoreConfig, BasePath};

/// The `insert` command
#[derive(Debug, StructOpt)]
#[structopt(
	name = "insert",
	about = "Insert a key to the keystore of a node."
)]
pub struct InsertKeyCmd {
	/// The secret key URI.
	/// If the value is a file, the file content is used as URI.
	/// If not given, you will be prompted for the URI.
	#[structopt(long)]
	suri: Option<String>,

	/// Key type, examples: "gran", or "imon"
	#[structopt(long)]
	key_type: String,

	#[allow(missing_docs)]
	#[structopt(flatten)]
	pub shared_params: SharedParams,

	#[allow(missing_docs)]
	#[structopt(flatten)]
	pub keystore_params: KeystoreParams,

	#[allow(missing_docs)]
	#[structopt(flatten)]
	pub crypto_scheme: CryptoSchemeFlag,
}

impl InsertKeyCmd {
	/// Run the command
	pub fn run<C: SubstrateCli>(&self, cli: &C) -> Result<(), Error> {
		let suri = utils::read_uri(self.suri.as_ref())?;
		let base_path = self.shared_params
			.base_path()
			.unwrap_or_else(|| BasePath::from_project("", "", &C::executable_name()));
		let chain_id = self.shared_params.chain_id(self.shared_params.is_dev());
		let chain_spec = cli.load_spec(&chain_id)?;
		let config_dir = base_path.config_dir(chain_spec.id());

		let (keystore, public) = match self.keystore_params.keystore_config(&config_dir)? {
			(_, KeystoreConfig::Path { path, password }) => {
				let public = with_crypto_scheme!(
					self.crypto_scheme.scheme,
					to_vec(&suri, password.clone())
				)?;
				let keystore: SyncCryptoStorePtr = Arc::new(LocalKeystore::open(path, password)?);
				(keystore, public)
			},
			_ => unreachable!("keystore_config always returns path and password; qed")
		};

		let key_type = KeyTypeId::try_from(self.key_type.as_str()).map_err(|_| Error::KeyTypeInvalid)?;

		SyncCryptoStore::insert_unknown(&*keystore, key_type, &suri, &public[..])
			.map_err(|_| Error::KeyStoreOperation)?;

		Ok(())
	}
}

fn to_vec<P: sp_core::Pair>(uri: &str, pass: Option<SecretString>) -> Result<Vec<u8>, Error> {
	let p = utils::pair_from_suri::<P>(uri, pass)?;
	Ok(p.public().as_ref().to_vec())
}

