// Exchange rate canister https://dashboard.internetcomputer.org/canister/uf6dk-hyaaa-aaaaq-qaaaq-cai
use candid::{self, CandidType, Deserialize, Principal};

pub const EXCHANGE_RATE_CANISTER_ID: Principal =
    Principal::from_slice(b"\x00\x00\x00\x00\x02\x10\x00\x01\x01\x01"); // uf6dk-hyaaa-aaaaq-qaaaq-cai

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

#[derive(CandidType, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Debug)]
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
