use std::borrow::Cow;

use candid::CandidType;
use ic_stable_structures::{storable::Bound, Storable};
use serde::{Deserialize, Serialize};

const MAX_PRINCIPAL_SIZE: u32 = 29;

#[derive(
    Debug, Clone, Copy, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, CandidType, Hash,
)]
pub struct Principal(pub candid::Principal);

impl From<candid::Principal> for Principal {
    fn from(value: candid::Principal) -> Self {
        Self(value)
    }
}

impl Storable for Principal {
    const BOUND: Bound = Bound::Bounded {
        max_size: MAX_PRINCIPAL_SIZE,
        is_fixed_size: false,
    };
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(self.0.as_slice().to_vec())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Self(candid::Principal::try_from_slice(bytes.as_ref()).unwrap())
    }
}

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
