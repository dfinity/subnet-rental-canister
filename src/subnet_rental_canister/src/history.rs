use crate::{Principal, RentalConditionId, RentalConditions, RentalRequest};
use candid::{CandidType, Decode, Encode};
use ic_ledger_types::Tokens;
use ic_stable_structures::{storable::Bound, Storable};
use serde::Deserialize;
use std::borrow::Cow;

/// Important events are persisted for auditing by the community.
/// Create events via EventType::SomeVariant.into()
/// so that system time is captured automatically.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, CandidType, Deserialize)]
pub struct Event {
    time_nanos: u64,
    event: EventType,
}

impl Storable for Event {
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }

    const BOUND: Bound = Bound::Unbounded;
}

impl Event {
    pub fn event(&self) -> EventType {
        self.event.clone()
    }

    pub fn time_nanos(&self) -> u64 {
        self.time_nanos
    }

    #[cfg(test)]
    pub fn _mk_event(time_nanos: u64, event: EventType) -> Self {
        Self { time_nanos, event }
    }
}

impl From<EventType> for Event {
    fn from(value: EventType) -> Self {
        Event {
            event: value,
            time_nanos: ic_cdk::api::time(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, CandidType, Deserialize)]
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
        // Convert dependencies' error types to string in order to keep candid interface minimal.
        reason: String,
    },
    /// When the user calls get_refund and the effort is abandoned.
    RentalRequestCancelled {
        rental_request: RentalRequest,
    },
    /// After successfull polling for a CreateSubnet proposal, a RentalAgreement is created
    RentalAgreementCreated {
        user: Principal,
        rental_request_proposal_id: u64,
        subnet_creation_proposal_id: Option<u64>,
        rental_condition_id: RentalConditionId,
    },
    RentalAgreementTerminated {
        user: Principal,
        initial_proposal_id: u64,
        subnet_creation_proposal_id: Option<u64>,
        rental_condition_id: RentalConditionId,
    },
    /// A successful transfer from the SRC/user subaccount to the SRC main account.
    TransferSuccess {
        amount: Tokens,
        block_index: u64,
    },
    PaymentSuccess {
        amount: Tokens,
        cycles: u128,
        covered_until_nanos: u64,
    },
    PaymentFailure {
        reason: String,
    },
    /// A successfull locking of 10% during the wait until subnet creation.
    LockingSuccess {
        user: Principal,
        amount: Tokens,
        cycles: u128,
    },
    /// A failure to lock 10% during the wait until subnet creation.
    LockingFailure {
        user: Principal,
        reason: String,
    },
    Degraded,
    Undegraded,
    Other {
        message: String,
    },
}
