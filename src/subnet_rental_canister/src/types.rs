use candid::CandidType;
use serde::Deserialize;

#[derive(CandidType, Deserialize)]
pub struct Tokens {
    pub e8s: u64,
}

#[derive(CandidType, Deserialize)]
pub struct TimeStamp {
    pub timestamp_nanos: u64,
}

#[derive(CandidType, Deserialize)]
pub struct TransferArgs {
    pub to: serde_bytes::ByteBuf,
    pub fee: Tokens,
    pub memo: u64,
    pub from_subaccount: Option<serde_bytes::ByteBuf>,
    pub created_at_time: Option<TimeStamp>,
    pub amount: Tokens,
}

#[derive(CandidType, Deserialize)]
pub enum TransferError {
    TxTooOld { allowed_window_nanos: u64 },
    BadFee { expected_fee: Tokens },
    TxDuplicate { duplicate_of: u64 },
    TxCreatedInFuture,
    InsufficientFunds { balance: Tokens },
}
