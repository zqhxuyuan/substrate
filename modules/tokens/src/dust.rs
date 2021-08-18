use orml_traits::OnDust;
use crate::{Config};
use sp_std::marker;

// pub struct TransferDust<T, GetAccountId>(marker::PhantomData<(T, GetAccountId)>);
// impl<T, GetAccountId> OnDust<T::AccountId, T::CurrencyId, T::Balance> for TransferDust<T, GetAccountId>
// where
// 	T: Config,
// 	GetAccountId: Get<T::AccountId>,
// {
// 	fn on_dust(who: &T::AccountId, currency_id: T::CurrencyId, amount: T::Balance) {
// 		// transfer the dust to treasury account, ignore the result,
// 		// if failed will leave some dust which still could be recycled.
// 		let _ = <Pallet<T> as MultiCurrency<T::AccountId>>::transfer(currency_id, who, &GetAccountId::get(), amount);
// 	}
// }
//
// pub struct BurnDust<T>(marker::PhantomData<T>);
// impl<T: Config> OnDust<T::AccountId, T::CurrencyId, T::Balance> for BurnDust<T> {
// 	fn on_dust(who: &T::AccountId, currency_id: T::CurrencyId, amount: T::Balance) {
// 		// burn the dust, ignore the result,
// 		// if failed will leave some dust which still could be recycled.
// 		let _ = Pallet::<T>::withdraw(currency_id, who, amount);
// 	}
// }