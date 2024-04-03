#![allow(dead_code)]

use ic_cdk::update;

#[update]
pub fn get_exchange_rate(request: GetExchangeRateRequest) -> GetExchangeRateResult {
    let GetExchangeRateRequest {
        timestamp,
        quote_asset,
        base_asset,
    } = request;

    let timestamp = if let Some(timestamp) = timestamp {
        timestamp
    } else {
        let ts = ic_cdk::api::time();
        ts - ts % 86400
    };

    // for now, hardcode result
    let metadata = ExchangeRateMetadata {
        decimals: 9,
        forex_timestamp: None,
        quote_asset_num_received_rates: 0,
        base_asset_num_received_rates: 0,
        base_asset_num_queried_sources: 0,
        standard_deviation: 0,
        quote_asset_num_queried_sources: 0,
    };

    let exchange_rate = ExchangeRate {
        metadata,
        rate: 12_503_823_284,
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
