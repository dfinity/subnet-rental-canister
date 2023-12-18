use candid::CandidType;
use ic_ledger_types::{AccountIdentifier, Tokens};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(CandidType, Deserialize)]
pub enum ExchangeRateCanister {
    Set(candid::Principal),
    Unset,
}
#[derive(CandidType, Deserialize, Default)]
pub struct CyclesCanisterInitPayload {
    pub exchange_rate_canister: Option<ExchangeRateCanister>,
    pub last_purged_notification: Option<u64>,
    pub governance_canister_id: Option<candid::Principal>,
    pub minting_account_id: Option<AccountIdentifier>,
    pub ledger_canister_id: Option<candid::Principal>,
}

#[derive(CandidType)]
pub enum NnsLedgerCanisterPayload {
    Init(NnsLedgerCanisterInitPayload),
}

#[derive(CandidType)]
pub struct NnsLedgerCanisterInitPayload {
    pub minting_account: String,
    pub initial_values: HashMap<String, Tokens>,
    pub send_whitelist: HashSet<candid::Principal>,
    pub transfer_fee: Option<Tokens>,
    pub token_symbol: Option<String>,
    pub token_name: Option<String>,
}
