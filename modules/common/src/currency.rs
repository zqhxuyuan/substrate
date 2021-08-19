use codec::{Codec, FullCodec, MaxEncodedLen};
pub use frame_support::{
	traits::{BalanceStatus, LockIdentifier},
	transactional,
};
use sp_runtime::{
	traits::{AtLeast32BitUnsigned, MaybeSerializeDeserialize},
	DispatchError, DispatchResult,
};
use sp_std::{
	cmp::{Eq, PartialEq},
	convert::{TryFrom, TryInto},
	fmt::Debug,
	result,
};
use frame_support::traits::{
	WithdrawReasons, ExistenceRequirement,
	tokens::Balance
};

pub trait VerticCurrency<AccountId> {
	/// The balance of an account.
	type Balance: Balance + MaybeSerializeDeserialize + Debug + MaxEncodedLen;

	// PUBLIC IMMUTABLES

	/// The combined balance of `who`.
	fn total_balance(who: &AccountId) -> Self::Balance;

	/// The total amount of issuance in the system.
	fn total_issuance() -> Self::Balance;

	/// The minimum balance any single account may have. This is equivalent to the `Balances` module's
	/// `ExistentialDeposit`.
	fn minimum_balance() -> Self::Balance;

	/// The 'free' balance of a given account.
	fn free_balance(who: &AccountId) -> Self::Balance;

	/// Returns `Ok` iff the account is able to make a withdrawal of the given amount
	/// for the given reason. Basically, it's just a dry-run of `withdraw`.
	///
	/// `Err(...)` with the reason why not otherwise.
	fn ensure_can_withdraw(
		who: &AccountId,
		_amount: Self::Balance,
		reasons: WithdrawReasons,
		new_balance: Self::Balance,
	) -> DispatchResult;

	// PUBLIC MUTABLES (DANGEROUS)

	/// Transfer some liquid free balance to another staker.
	///
	/// This is a very high-level function. It will ensure all appropriate fees are paid
	/// and no imbalance in the system remains.
	fn transfer(
		source: &AccountId,
		dest: &AccountId,
		value: Self::Balance,
		existence_requirement: ExistenceRequirement,
	) -> DispatchResult;

}
