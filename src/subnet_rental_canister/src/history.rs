use std::borrow::Cow;

use candid::{CandidType, Decode, Encode};
use ic_stable_structures::{storable::Bound, Storable};
use serde::Deserialize;

use crate::{ExecuteProposalError, Principal, RentalAgreement};

/// The
#[derive(Debug, Default, Clone, CandidType, Deserialize)]
pub struct History {
    pub events: Vec<Event>,
}

impl Storable for History {
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
    const BOUND: Bound = Bound::Unbounded;
}

#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct Event {
    event: EventType,
    date: u64,
}

impl Event {
    pub fn new(event: EventType) -> Self {
        event.into()
    }
}

impl From<EventType> for Event {
    fn from(value: EventType) -> Self {
        Event {
            event: value,
            date: ic_cdk::api::time(),
        }
    }
}

#[derive(Debug, Clone, CandidType, Deserialize)]
pub enum EventType {
    Created {
        // proposal_id: u64,
        rental_agreement: RentalAgreement,
    },
    Rejected {
        // proposal_id: u64,
        user: Principal,
    },
    Failed {
        // proposal_id: u64,
        user: Principal,
        reason: ExecuteProposalError,
    },
    Terminated,
    PaymentSuccess {
        amount: u64,
        covered_until: u64,
    },
    PaymentFailure {
        reason: String,
    },
    Degraded,
    Undegraded,
    RentalConditionsChanged, // TODO: Create this in canister init
    Other {
        message: String,
    },
}
