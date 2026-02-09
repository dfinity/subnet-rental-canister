//! A mock implementation of the exchange rate canister, which allows us to get
//! exchange rates for testing purposes.
//!
//! Ideally, the real canister would be used, however, since the real canister
//! performs HTTPS outcalls, it creates a complication in PocketIC tests as
//! mocking these would be more demanding as one would need to understand the
//! exact format of the outcalls the XRC performs and provide mock responses
//! in their tests accordingly.
//!
//! For now and until another better solution is found (e.g. PocketIC providing a way
//! to mock calls to a certain canister or the XRC providing a dedicated mock which
//! does not perform HTTPS outcalls), this mock is used to provide exchange rates
//! to the subnet rental canister in PocketIC tests.

use ic_xrc_types::{
    Asset, AssetClass, ExchangeRate, ExchangeRateMetadata, GetExchangeRateRequest,
    GetExchangeRateResult,
};
use std::cell::RefCell;

thread_local! {
    static RATES: RefCell<Vec<(u64, u64)>> = const { RefCell::new(vec![]) } // (timestamp, rate) where rate is 1 ICP = X XDR (10^9 precision)
}

// See https://github.com/dfinity/exchange-rate-canister/blob/2f2a08f36fa6d043da9751d61d77952b36a59006/src/xrc/src/lib.rs#L56
// for this constant.
const CALL_CYCLES_COST: u128 = 1_000_000_000;

#[ic_cdk::update]
async fn get_exchange_rate(request: GetExchangeRateRequest) -> GetExchangeRateResult {
    ic_cdk::api::msg_cycles_accept(CALL_CYCLES_COST);
    // Find the closest rate to the request timestamp
    let timestamp = request
        .timestamp
        .unwrap_or_else(|| (ic_cdk::api::time() / 1_000_000_000) + 6); // Add a slight perterbation; there is nothing particularly magical about 6 seconds; it's just a "small" amount of time.
    let rates = RATES.with(|rates| rates.borrow().clone());
    let closest_rate = rates.iter().min_by_key(|(t, _)| t.abs_diff(timestamp));
    let default_rate = (timestamp, 3_497_900_000); // 1 ICP = 3.4979 XDR
    let (_, rate) = closest_rate.unwrap_or(&default_rate);
    GetExchangeRateResult::Ok(ExchangeRate {
        base_asset: Asset {
            symbol: "ICP".to_string(),
            class: AssetClass::Cryptocurrency,
        },
        quote_asset: Asset {
            symbol: "CXDR".to_string(), // "C" stands for "computed" (and "XDR" is the standard symbol for Special Drawing Rights).
            class: AssetClass::FiatCurrency,
        },
        timestamp,
        rate: *rate,
        metadata: ExchangeRateMetadata {
            decimals: 9,
            base_asset_num_queried_sources: 7,
            base_asset_num_received_rates: 5,
            quote_asset_num_queried_sources: 10,
            quote_asset_num_received_rates: 4,
            standard_deviation: 0,
            forex_timestamp: None,
        },
    })
}

#[ic_cdk::update]
fn set_exchange_rate_data(data: Vec<(u64, u64)>) {
    RATES.with_borrow_mut(|rates| {
        rates.extend(data);
    });
}
