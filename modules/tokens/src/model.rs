use frame_support::{
    ensure, log,
    pallet_prelude::*,
    traits::{
        tokens::{fungible, fungibles, DepositConsequence, WithdrawConsequence},
        BalanceStatus as Status, Currency as PalletCurrency, ExistenceRequirement, Get, Imbalance,
        LockableCurrency as PalletLockableCurrency, ReservableCurrency as PalletReservableCurrency,
        SignedImbalance, WithdrawReasons,
    },
    transactional, BoundedVec,
};
use orml_traits::{
    arithmetic::{self, Signed},
    currency::TransferAll,
    BalanceStatus, GetByKey, LockIdentifier, BasicCurrency, BasicCurrencyExtended, BasicLockableCurrency,
    BasicReservableCurrency, OnDust,
};
use sp_runtime::{
    traits::{
        AtLeast32BitUnsigned, Bounded, CheckedAdd, CheckedSub, MaybeSerializeDeserialize, Member, Saturating,
        StaticLookup, Zero,
    },
    ArithmeticError, DispatchError, DispatchResult, RuntimeDebug,
};

/// A single lock on a balance. There can be many of these on an account and
/// they "overlap", so the same balance is frozen by multiple locks.
#[derive(Encode, Decode, Clone, PartialEq, Eq, MaxEncodedLen, RuntimeDebug)]
pub struct BalanceLock<Balance> {
    /// An identifier for this lock. Only one lock may be in existence for
    /// each identifier.
    pub id: LockIdentifier,
    /// The amount which the free balance may not drop below when this lock
    /// is in effect.
    pub amount: Balance,
}

/// balance information for an account.
#[derive(Encode, Decode, Clone, PartialEq, Eq, Default, MaxEncodedLen, RuntimeDebug)]
pub struct AccountData<Balance> {
    /// Non-reserved part of the balance. There may still be restrictions on
    /// this, but it is the total pool what may in principle be transferred,
    /// reserved.
    ///
    /// This is the only balance that matters in terms of most operations on
    /// tokens.
    pub free: Balance,
    /// Balance which is reserved and may not be used at all.
    ///
    /// This can still get slashed, but gets slashed last of all.
    ///
    /// This balance is a 'reserve' balance that other subsystems use in
    /// order to set aside tokens that are still 'owned' by the account
    /// holder, but which are suspendable.
    pub reserved: Balance,
    /// The amount that `free` may not drop below when withdrawing.
    pub frozen: Balance,
}

impl<Balance: Saturating + Copy + Ord> AccountData<Balance> {
    /// The amount that this account's free balance may not be reduced
    /// beyond.
    pub(crate) fn frozen(&self) -> Balance {
        self.frozen
    }
    /// The total balance in this account including any that is reserved and
    /// ignoring any frozen.
    pub(crate) fn total(&self) -> Balance {
        self.free.saturating_add(self.reserved)
    }
}