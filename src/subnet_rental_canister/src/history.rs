use std::borrow::Cow;

use candid::{CandidType, Decode, Encode};
use ic_ledger_types::Tokens;
use ic_stable_structures::{storable::Bound, Storable};
use serde::Deserialize;

use crate::{ExecuteProposalError, Principal, RentalAgreement, RentalConditions};

/// Important events are persisted for auditing by the community.
/// History struct instances are values in a Map<SubnetId, History>, so the
/// corresponding subnet_id is always implied. The first event in a History
/// should be a RentalConditionsChanged variant, created in canister_init.
/// Events belonging to a valid rental agreement are then bracketed by the variants
/// Created and Terminated.
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

/// A rental agreement state change.
/// Prefer creating events via EventType::SomeVariant.into().
#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct Event {
    event: EventType,
    date: u64,
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
    // TODO: Create this in canister init
    RentalConditionsChanged {
        rental_conditions: RentalConditions,
    },
    RentalConditionsRemoved {
        rental_conditions: RentalConditions,
    },
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
        amount: Tokens,
        covered_until: u64,
    },
    PaymentFailure {
        reason: String,
    },
    Degraded,
    Undegraded,
    Other {
        message: String,
    },
}
