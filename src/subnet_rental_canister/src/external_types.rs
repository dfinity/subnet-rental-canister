use candid::{CandidType, Principal};
use ic_ledger_types::{AccountIdentifier, Tokens};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

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

#[derive(CandidType)]
pub struct SetAuthorizedSubnetworkListArgs {
    pub who: Option<Principal>,
    pub subnets: Vec<Principal>,
}

#[derive(CandidType, Deserialize)]
pub enum ExchangeRateCanister {
    Set(Principal),
    Unset,
}

#[derive(CandidType, Deserialize, Default)]
pub struct CyclesCanisterInitPayload {
    pub exchange_rate_canister: Option<ExchangeRateCanister>,
    pub last_purged_notification: Option<u64>,
    pub governance_canister_id: Option<Principal>,
    pub minting_account_id: Option<AccountIdentifier>,
    pub ledger_canister_id: Option<Principal>,
}

#[derive(CandidType)]
pub enum NnsLedgerCanisterPayload {
    Init(NnsLedgerCanisterInitPayload),
}

#[derive(CandidType)]
pub struct NnsLedgerCanisterInitPayload {
    pub minting_account: String,
    pub initial_values: HashMap<String, Tokens>,
    pub send_whitelist: HashSet<Principal>,
    pub transfer_fee: Option<Tokens>,
    pub token_symbol: Option<String>,
    pub token_name: Option<String>,
}
