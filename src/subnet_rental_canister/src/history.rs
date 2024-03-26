use std::borrow::Cow;

<<<<<<< HEAD
use crate::{
    ExecuteProposalError, Principal, RentalConditionId, RentalConditions, RentalRequest,
    SubnetSpecification,
};
=======
use crate::{ExecuteProposalError, Principal, RentalConditionId, RentalConditions, RentalRequest};
>>>>>>> origin
use candid::{CandidType, Decode, Encode};
use ic_ledger_types::Tokens;
use ic_stable_structures::{storable::Bound, Storable};
use serde::Deserialize;

/// Important events are persisted for auditing by the community.
/// History struct instances are values in a Map<SubnetId, History>, so the
/// corresponding subnet_id is always implied.
<<<<<<< HEAD
/// Events that are not associated with a subnet are collected under 'None'.
=======
/// Events on rental conditions changes are collected under 'None'.  
>>>>>>> origin
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
/// Prefer creating events via EventType::SomeVariant.into()
/// so that system time is captured automatically.
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
    /// Changed via code upgrade, which should create this event in the post-upgrade hook.
    /// A None value means that the entry has been removed from the map.
    RentalConditionsChanged {
        rental_condition_id: RentalConditionId,
        rental_conditions: Option<RentalConditions>,
    },
    /// A successful SubnetRentalRequest proposal execution leads to a RentalRequest
    RentalRequestCreated {
        rental_request: RentalRequest,
    },
    /// An unsuccessful SubnetRentalRequest proposal execution
    RentalRequestFailed {
        user: Principal,
        proposal_id: u64,
        reason: ExecuteProposalError,
    },
    /// When the user calls get_refund and the effort is abandoned.
    RentalRequestCancelled {
        rental_request: RentalRequest,
        refund_amount: Tokens,
    },
    /// After successfull polling for a CreateSubnet proposal, a RentalAgreement is created
    RentalAgreementCreated {
        user: Principal,
        initial_proposal_id: u64,
        subnet_creation_proposal_id: Option<u64>,
        rental_condition_type: RentalConditionId,
    },
    // TODO: How to even get this?
    RentalAgreementTerminated {
        user: Principal,
        initial_proposal_id: u64,
        subnet_creation_proposal_id: Option<u64>,
        rental_condition_type: RentalConditionId,
    },
    PaymentSuccess {
        amount: Tokens,
        cycles: u128,
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
