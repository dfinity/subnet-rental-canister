use candid::CandidType;
use serde::Deserialize;

use crate::RentalAgreement;

#[derive(Debug, Clone, CandidType, Deserialize)]
struct Event {
    event: EventType,
    date: u64,
}

#[derive(Debug, Clone, CandidType, Deserialize)]
enum EventType {
    Created(RentalAgreement),
    PaymentSuccess,
    PaymentFailure,
}
