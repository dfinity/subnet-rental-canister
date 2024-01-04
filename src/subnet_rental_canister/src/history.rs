use std::borrow::Cow;

use candid::{CandidType, Decode, Encode};
use ic_stable_structures::{storable::Bound, Storable};
use serde::Deserialize;

use crate::{Principal, RentalAgreement};

/// The 
#[derive(Debug, Clone, CandidType, Deserialize)]
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
pub(crate) struct Event {
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
pub(crate) enum EventType {
    Created { rental_agreement: RentalAgreement },
    Rejected { user: Principal, subnet: Principal },
    PaymentSuccess { amount: u64 },
    PaymentFailure { reason: String },
    Degraded,
    Undegraded,
    Other(String),
}
