use candid::{CandidType, Deserialize, Principal};
use ic_ledger_types::Tokens;
use std::collections::{HashMap, HashSet};

#[derive(CandidType, Debug)]
pub struct NotifyTopUpArg {
    pub block_index: u64,
    pub canister_id: Principal,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum NotifyError {
    Refunded {
        block_index: Option<u64>,
        reason: String,
    },
    InvalidTransaction(String),
    Other {
        error_message: String,
        error_code: u64,
    },
    Processing,
    TransactionTooOld(u64),
}

#[derive(CandidType, Deserialize, Debug)]
pub struct IcpXdrConversionRate {
    pub xdr_permyriad_per_icp: u64,
    pub timestamp_seconds: u64,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct IcpXdrConversionRateResponse {
    pub certificate: serde_bytes::ByteBuf,
    pub data: IcpXdrConversionRate,
    pub hash_tree: serde_bytes::ByteBuf,
}

#[derive(CandidType, Debug)]
pub struct SetAuthorizedSubnetworkListArgs {
    pub who: Option<Principal>,
    pub subnets: Vec<Principal>,
}

#[derive(CandidType, Debug)]
pub struct CmcInitPayload {
    pub ledger_canister_id: Option<Principal>,
    pub governance_canister_id: Option<Principal>,
    pub minting_account_id: String,
    pub last_purged_notification: Option<u64>,
    pub exchange_rate_canister: Option<ExchangeRateCanister>,
    pub cycles_ledger_canister_id: Option<Principal>,
}

#[derive(CandidType, Debug)]
pub enum ExchangeRateCanister {
    /// Enables the exchange rate canister with the given canister ID.
    Set(Principal),
    /// Disable the exchange rate canister.
    Unset,
}

#[derive(CandidType, Debug)]
pub enum NnsLedgerCanisterPayload {
    Init(NnsLedgerCanisterInitPayload),
}

#[derive(CandidType, Debug)]
pub struct NnsLedgerCanisterInitPayload {
    pub minting_account: String,
    pub initial_values: HashMap<String, Tokens>,
    pub send_whitelist: HashSet<Principal>,
    pub transfer_fee: Option<Tokens>,
    pub token_symbol: Option<String>,
    pub token_name: Option<String>,
    pub feature_flags: Option<FeatureFlags>,
}

#[derive(CandidType, Debug)]
pub struct FeatureFlags {
    pub icrc2: bool,
}
