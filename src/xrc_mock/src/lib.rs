use candid::candid_method;
use ic_xrc_types::{
    Asset, AssetClass, ExchangeRate, ExchangeRateMetadata, GetExchangeRateRequest,
    GetExchangeRateResult,
};
use std::cell::RefCell;

thread_local! {
    static RATES: RefCell<Vec<(u64, u64)>> = RefCell::new(vec![]); // (timestamp, rate)
}

const CALL_CYCLES_COST: u128 = 1_000_000_000;

#[ic_cdk::update]
#[candid_method(update)]
async fn get_exchange_rate(request: GetExchangeRateRequest) -> GetExchangeRateResult {
    ic_cdk::api::msg_cycles_accept(CALL_CYCLES_COST);
    // Find the closest rate to the request timestamp
    let timestamp = request
        .timestamp
        .unwrap_or_else(|| (ic_cdk::api::time() / 1_000_000_000) + 6);
    let rates = RATES.with(|rates| rates.borrow().clone());
    let closest_rate = rates.iter().min_by_key(|(t, _)| t.abs_diff(timestamp));
    let default_rate = (timestamp, 3_497_900_000);
    let (_, rate) = closest_rate.unwrap_or(&default_rate);
    GetExchangeRateResult::Ok(ExchangeRate {
        base_asset: Asset {
            symbol: "ICP".to_string(),
            class: AssetClass::Cryptocurrency,
        },
        quote_asset: Asset {
            symbol: "CXDR".to_string(),
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
#[candid_method(update)]
fn set_exchange_rate_data(data: Vec<(u64, u64)>) {
    RATES.with_borrow_mut(|rates| {
        rates.extend(data);
    });
}
