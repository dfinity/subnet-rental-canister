use candid::{CandidType, Deserialize, Principal};
use ic_ledger_types::{Memo, Subaccount, Tokens};
use std::collections::{HashMap, HashSet};

#[derive(CandidType, Deserialize, Debug)]
pub struct TransferFromArgs {
    pub to: Account,
    pub fee: Option<u128>,
    pub spender_subaccount: Option<Subaccount>,
    pub from: Account,
    pub memo: Option<Memo>,
    pub created_at_time: Option<u64>,
    pub amount: u128,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum TransferFromError {
    GenericError { message: String, error_code: u128 },
    TemporarilyUnavailable,
    InsufficientAllowance { allowance: u128 },
    BadBurn { min_burn_amount: u128 },
    Duplicate { duplicate_of: u128 },
    BadFee { expected_fee: u128 },
    CreatedInFuture { ledger_time: u64 },
    TooOld,
    InsufficientFunds { balance: u128 },
}

#[derive(CandidType, Deserialize, Debug)]
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
pub struct Account {
    pub owner: Principal,
    pub subaccount: Option<Subaccount>,
}

#[derive(CandidType, Deserialize, Debug)]
pub struct ApproveArgs {
    pub fee: Option<u128>,
    pub memo: Option<Memo>,
    pub from_subaccount: Option<Subaccount>,
    pub created_at_time: Option<u64>,
    pub amount: u128,
    pub expected_allowance: Option<u128>,
    pub expires_at: Option<u64>,
    pub spender: Account,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum ApproveError {
    GenericError { message: String, error_code: u128 },
    TemporarilyUnavailable,
    Duplicate { duplicate_of: u128 },
    BadFee { expected_fee: u128 },
    AllowanceChanged { current_allowance: u128 },
    CreatedInFuture { ledger_time: u64 },
    TooOld,
    Expired { ledger_time: u64 },
    InsufficientFunds { balance: u128 },
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

#[derive(CandidType)]
pub struct SetAuthorizedSubnetworkListArgs {
    pub who: Option<Principal>,
    pub subnets: Vec<Principal>,
}

#[derive(Deserialize, CandidType, Debug)]
pub struct CyclesCanisterInitPayload {
    pub ledger_canister_id: Option<Principal>,
    pub governance_canister_id: Option<Principal>,
    pub minting_account_id: String,
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
    pub feature_flags: Option<FeatureFlags>,
}

#[derive(CandidType, Deserialize)]
pub struct FeatureFlags {
    pub icrc2: bool,
}
