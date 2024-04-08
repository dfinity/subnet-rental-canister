use std::{cell::RefCell, collections::HashMap};

use ic_cdk::{println, update};

pub const XRC_REQUEST_CYCLES_COST: u128 = 1_000_000_000;

thread_local! {
    static DATA: RefCell<HashMap<u64, (u64, u64)>> = RefCell::new(HashMap::new());
}

/// Set test data: [(time in seconds since epoch, (rate, decimals))]
#[update]
pub fn set_exchange_rate_data(data: Vec<(u64, (u64, u64))>) {
    DATA.with_borrow_mut(|map| {
        for (k, v) in data.into_iter() {
            map.insert((k / 60) * 60, v);
        }
    })
}

#[update]
pub fn get_exchange_rate(request: GetExchangeRateRequest) -> GetExchangeRateResult {
    let payment = ic_cdk::api::call::msg_cycles_accept128(XRC_REQUEST_CYCLES_COST);
    if payment < XRC_REQUEST_CYCLES_COST {
        return GetExchangeRateResult::Err(ExchangeRateError::NotEnoughCycles);
    }

    let GetExchangeRateRequest {
        timestamp,
        quote_asset,
        base_asset,
    } = request;

    let timestamp =
        (timestamp.unwrap_or_else(|| (ic_cdk::api::time().saturating_sub(30))) / 60) * 60;

    let Some((rate, decimals)) = DATA.with_borrow(|map| map.get(&timestamp).map(|x| x.clone()))
    else {
        println!("requested timestamp: {}", timestamp);
        println!(
            "known times: {:?}",
            DATA.with_borrow(|map| map.clone().into_iter())
        );
        return GetExchangeRateResult::Err(ExchangeRateError::ForexInvalidTimestamp);
    };

    let metadata = ExchangeRateMetadata {
        decimals: decimals as u32,
        forex_timestamp: None,
        quote_asset_num_received_rates: 0,
        base_asset_num_received_rates: 0,
        base_asset_num_queried_sources: 0,
        standard_deviation: 0,
        quote_asset_num_queried_sources: 0,
    };

    let exchange_rate = ExchangeRate {
        metadata,
        rate,
        timestamp,
        quote_asset,
        base_asset,
    };
    GetExchangeRateResult::Ok(exchange_rate)
}

// ============================================================================

// Exchange rate canister https://dashboard.internetcomputer.org/canister/uf6dk-hyaaa-aaaaq-qaaaq-cai
use candid::{self, CandidType, Deserialize};

pub static EXCHANGE_RATE_CANISTER_PRINCIPAL_STR: &str = "uf6dk-hyaaa-aaaaq-qaaaq-cai";

#[derive(CandidType, Deserialize, Debug)]
pub enum AssetClass {
    Cryptocurrency,
    FiatCurrency,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct Asset {
    pub class: AssetClass,
    pub symbol: String,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct GetExchangeRateRequest {
    pub timestamp: Option<u64>,
    pub quote_asset: Asset,
    pub base_asset: Asset,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct ExchangeRateMetadata {
    pub decimals: u32,
    pub forex_timestamp: Option<u64>,
    pub quote_asset_num_received_rates: u64,
    pub base_asset_num_received_rates: u64,
    pub base_asset_num_queried_sources: u64,
    pub standard_deviation: u64,
    pub quote_asset_num_queried_sources: u64,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct ExchangeRate {
    pub metadata: ExchangeRateMetadata,
    pub rate: u64,
    pub timestamp: u64,
    pub quote_asset: Asset,
    pub base_asset: Asset,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum ExchangeRateError {
    AnonymousPrincipalNotAllowed,
    CryptoQuoteAssetNotFound,
    FailedToAcceptCycles,
    ForexBaseAssetNotFound,
    CryptoBaseAssetNotFound,
    StablecoinRateTooFewRates,
    ForexAssetsNotFound,
    InconsistentRatesReceived,
    RateLimited,
    StablecoinRateZeroRate,
    Other { code: u32, description: String },
    ForexInvalidTimestamp,
    NotEnoughCycles,
    ForexQuoteAssetNotFound,
    StablecoinRateNotFound,
    Pending,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum GetExchangeRateResult {
    Ok(ExchangeRate),
    Err(ExchangeRateError),
}
