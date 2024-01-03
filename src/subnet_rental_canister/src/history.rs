use candid::CandidType;
use ic_stable_structures::{storable::Bound, Storable};
use serde::Deserialize;

use crate::RentalAgreement;

#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct History {
    events: Vec<Event>,
}

impl Storable for History {
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        todo!()
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        todo!()
    }

    const BOUND: Bound = Bound::Unbounded;
}

#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct Event {
    event: EventType,
    date: u64,
}

impl Event {
    fn new() {}
}

#[derive(Debug, Clone, CandidType, Deserialize)]
pub enum EventType {
    Created(RentalAgreement),
    PaymentSuccess,
    PaymentFailure,
}

impl Storable for Event {
    const BOUND: Bound = Bound::Unbounded;

    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        todo!()
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        todo!()
    }
}
