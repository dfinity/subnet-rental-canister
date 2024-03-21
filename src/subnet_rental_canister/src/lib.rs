use candid::{CandidType, Decode, Deserialize, Encode, Principal};
use external_types::NotifyError;
use ic_cdk::println;

use ic_ledger_types::{Memo, TransferError, MAINNET_GOVERNANCE_CANISTER_ID};
use ic_stable_structures::{storable::Bound, Storable};

use std::borrow::Cow;

pub mod canister;
pub mod canister_state;
pub mod external_calls;
pub mod external_types;
pub mod history;
mod http_request;

pub const TRILLION: u128 = 1_000_000_000_000;
pub const E8S: u64 = 100_000_000;
// const BILLING_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);
const MEMO_TOP_UP_CANISTER: Memo = Memo(0x50555054); // == 'TPUP'

// ============================================================================
// Types

/// Rental conditions are kept in a global HashMap and only changed via code upgrades.
#[derive(Debug, Clone, Copy, CandidType, Deserialize, PartialEq, Eq, Hash)]
pub enum RentalConditionType {
    App13Switzerland,
}

const APP13SWITZERLAND: RentalConditions = RentalConditions {
    daily_cost_cycles: 835 * TRILLION,
    initial_rental_period_days: 180,
    billing_period_days: 30,
};

/// Set of conditions for a subnet up for rent.
/// Rental conditions are kept in a global HashMap and only changed via code upgrades.
#[derive(Debug, Clone, Copy, CandidType, Deserialize)]
pub struct RentalConditions {
    daily_cost_cycles: u128,
    initial_rental_period_days: u64,
    billing_period_days: u64,
}

impl Storable for RentalConditions {
    // TODO: find max size and bound
    const BOUND: Bound = Bound::Unbounded;
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

#[derive(Clone, CandidType, Debug, Deserialize)]
pub enum SubnetSpecification {
    /// A description of the desired topology.
    TopologyDescription(String),
    /// If this is used, the SRC attempts to make the given subnet
    /// available for rent immediately.
    ExistingSubnetId(Principal),
}

/// The governance canister calls the SRC's proposal execution method
/// with this argument in case the proposal was valid and adopted.
#[derive(Clone, CandidType, Deserialize)]
pub struct SubnetRentalProposalPayload {
    // The tenant, who makes the payments
    pub user: Principal,
    /// Either a description of the desired topology
    /// or an existing subnet id.
    pub subnet_spec: SubnetSpecification,
    /// A key into the global RENTAL_CONDITIONS HashMap.
    pub rental_condition_type: RentalConditionType,
}

/// Successful proposal execution leads to a RentalRequest.
#[derive(Clone, CandidType, Debug, Deserialize)]
pub struct RentalRequest {
    pub user: Principal,
    /// The amount of cycles that are no longer refundable.
    pub locked_amount_cycles: u128,
    /// The initial proposal id will be mentioned in the subnet
    /// creation proposal. When this is found on the governance
    /// canister, polling can stop.
    pub initial_proposal_id: u64,
    /// Rental request creation date in nanoseconds since epoch.
    pub creation_date: u64,
    // ===== Some fields from the proposal payload for the rental agreement =====
    /// Either a description of the desired topology
    /// or an existing subnet id.
    pub subnet_spec: SubnetSpecification,
    /// A key into the global RENTAL_CONDITIONS HashMap.
    pub rental_condition_type: RentalConditionType,
}

impl Storable for RentalRequest {
    // TODO: find max size and bound
    const BOUND: Bound = Bound::Unbounded;
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct RentalAgreement {
    // ===== Immutable data =====
    /// The principal which paid the deposit and will be whitelisted.
    pub user: Principal,
    /// The id of the SubnetRentalRequest proposal.
    pub initial_proposal_id: u64,
    /// The id of the proposal that created the subnet. Optional in case
    /// the subnet already existed at initial proposal time.
    pub subnet_creation_proposal_id: Option<u64>,
    /// Either a description of the desired topology
    /// or an existing subnet id. Kept in the rental agreement so that
    /// UI can easily serve this associated information.
    pub subnet_spec: SubnetSpecification,
    /// A key into the global RENTAL_CONDITIONS HashMap.
    pub rental_condition_type: RentalConditionType,
    /// Rental agreement creation date in nanoseconds since epoch.
    pub creation_date: u64,
    // ===== Mutable data =====
    /// The date in nanos since epoch until which the rental agreement is paid for.
    pub covered_until: u64,
    /// This subnet's share of cycles among the SRC's cycles.
    /// Increased by the locking mechanism, monthly.
    /// Increased by the payment process (via timer).
    /// Decreased by the burning process (via heartbeat).
    pub cycles_balance: u128,
    /// The last point in time in nanos since epoch when cycles were burned in a heartbeat.
    pub last_burned: u64,
}

impl Storable for RentalAgreement {
    // should be bounded once we replace string with real type
    const BOUND: Bound = Bound::Unbounded;
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

#[derive(CandidType, Debug, Clone, Deserialize)]
pub enum ExecuteProposalError {
    SubnetNotRentable,
    SubnetAlreadyRented,
    UnauthorizedCaller,
    InsufficientFunds,
    TransferUserToSrcError(TransferError),
    TransferSrcToCmcError(TransferError),
    NotifyTopUpError(NotifyError),
    SubnetNotRented,
}

// ============================================================================
// Misc

fn verify_caller_is_governance() -> Result<(), ExecuteProposalError> {
    if ic_cdk::caller() != MAINNET_GOVERNANCE_CANISTER_ID {
        println!("Caller is not the governance canister");
        return Err(ExecuteProposalError::UnauthorizedCaller);
    }
    Ok(())
}
