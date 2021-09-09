use node_template_runtime::{
	AccountId, AuraConfig, BalancesConfig, GenesisConfig, GrandpaConfig, Signature, SudoConfig,
	SystemConfig, WASM_BINARY,
};
use sc_service::ChainType;
use sp_consensus_aura::sr25519::AuthorityId as AuraId;
use sp_core::{sr25519, Pair, Public, ed25519, ecdsa2, ecdsa};
use sp_finality_grandpa::AuthorityId as GrandpaId;
use sp_runtime::traits::{IdentifyAccount, Verify};

// The URL for the telemetry server.
// const STAGING_TELEMETRY_URL: &str = "wss://telemetry.polkadot.io/submit/";

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec<GenesisConfig>;

/// Generate a crypto pair from seed.
pub fn get_from_seed<TPublic: Public>(seed: &str) -> <TPublic::Pair as Pair>::Public {
	TPublic::Pair::from_string(&format!("//{}", seed), None)
		.expect("static values are valid; qed")
		.public()
}

/// Generate a crypto pair from seed.
pub fn get_from_seed_pair<TPublic: Public>(seed: &str) -> TPublic::Pair {
	TPublic::Pair::from_string(&format!("//{}", seed), None)
		.expect("static values are valid; qed")
}

type AccountPublic = <Signature as Verify>::Signer;

/// Generate an Aura authority key.
pub fn authority_keys_from_seed(s: &str) -> (AuraId, GrandpaId) {
	(get_from_seed::<AuraId>(s), get_from_seed::<GrandpaId>(s))
}

/// Generate an account ID from seed.
pub fn get_account_id_from_seed<TPublic: Public>(seed: &str) -> AccountId
where
	AccountPublic: From<<TPublic::Pair as Pair>::Public>,
{
	let account = AccountPublic::from(get_from_seed::<TPublic>(seed)).into_account();
    log::info!("account: {}, {:?}", account, account);
	account
}

pub fn get_account_id_from_seeds<TPublic: Public>(seeds: Vec<&str>) -> Vec<AccountId>
	where
		AccountPublic: From<<TPublic::Pair as Pair>::Public>,
{
	seeds.iter().map(|seed|
		AccountPublic::from(get_from_seed::<TPublic>(*seed)).into_account()
	).collect::<Vec<AccountId>>()
}

pub fn authority_keys_from_seed_accountId<TPublic: Public>(s: &str, grandpa_weight: u64) -> (AuraId, GrandpaId, AccountId, u64)
	where
		AccountPublic: From<<TPublic::Pair as Pair>::Public>,{
	(
		get_from_seed::<AuraId>(s),
		get_from_seed::<GrandpaId>(s),
	 	get_account_id_from_seed::<TPublic>(s),
		grandpa_weight
	)
}

pub fn authority_keys_from_seed_accountIds<TPublic: Public>(vec: Vec<(&str, u64)>) -> Vec<(AuraId, GrandpaId, AccountId, u64)>
	where
		AccountPublic: From<<TPublic::Pair as Pair>::Public>,{
	vec.iter().map(|(s, grandpa_weight)|
		(
			get_from_seed::<AuraId>(s),
			get_from_seed::<GrandpaId>(s),
			get_account_id_from_seed::<TPublic>(s),
			grandpa_weight.clone()
		)
	).collect::<Vec<(AuraId, GrandpaId, AccountId, u64)>>()
}

pub fn development_config<TPublic: Public>() -> Result<ChainSpec, String>
	where
		AccountPublic: From<<TPublic::Pair as Pair>::Public>
{
	let wasm_binary = WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?;
	Ok(ChainSpec::from_genesis(
		"Development", "dev", ChainType::Development,
		move || {
			testnet_genesis(
				wasm_binary,
				authority_keys_from_seed_accountIds::<TPublic>(vec![("Alice", 1)]),
				get_account_id_from_seed::<TPublic>("Alice"),
				get_account_id_from_seeds::<TPublic>(
					vec!["Alice", "Bob", "Alice//stash", "Bob//stash"]),
				true,
			)
		},
		vec![], None, None, None, None,
	))
}

// chain=local
pub fn local_testnet_config() -> Result<ChainSpec, String> {
	let wasm_binary = WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?;
	Ok(ChainSpec::from_genesis(
		"Local Testnet", "local_testnet", ChainType::Local,
		move || {
			testnet_genesis(
				wasm_binary,
				authority_keys_from_seed_accountIds::<sr25519::Public>(vec![
					("Alice", 1), // ("Bob", 2), ("Charlie", 4), ("Dave", 8), ("Eve", 16)
				]),
				get_account_id_from_seed::<sr25519::Public>("Alice"),
				get_account_id_from_seeds::<sr25519::Public>(
					vec!["Alice", "Bob", "Charlie", "Dave", "Eve", "Ferdie",
						 "Alice//stash", "Bob//stash", "Charlie//stash",
						 "Dave//stash", "Eve//stash", "Ferdie//stash"]),
				true,
			)
		},
		// Bootnodes
		vec![],
		// Telemetry
		None,
		// Protocol ID
		None,
		// Properties
		None,
		// Extensions
		None,
	))
}

/// Configure initial storage state for FRAME modules.
fn testnet_genesis(
	wasm_binary: &[u8],
	initial_authorities: Vec<(AuraId, GrandpaId, AccountId, u64)>,
	root_key: AccountId,
	endowed_accounts: Vec<AccountId>,
	_enable_println: bool,
) -> GenesisConfig {
	GenesisConfig {
		system: SystemConfig {
			// Add Wasm runtime to storage.
			code: wasm_binary.to_vec(),
			changes_trie_config: Default::default(),
		},
		balances: BalancesConfig {
			// Configure endowed accounts with initial balance of 1 << 60.
			balances: endowed_accounts.iter().cloned().map(|k| (k, 1 << 60)).collect(),
		},
		aura: AuraConfig {
			authorities: initial_authorities.iter().map(|x| (x.0.clone())).collect(),
			accounts: initial_authorities.iter().map(|x| (x.2.clone())).collect(),
		},
		grandpa: GrandpaConfig {
			authorities: initial_authorities.iter().map(|x| (x.1.clone(), x.3)).collect(),
		},
		sudo: SudoConfig {
			// Assign network admin rights.
			key: root_key,
		},
	}
}
