use candid::{CandidType, Decode, Deserialize, Encode, Principal};
use history::Event;
use ic_ledger_types::{Memo, Tokens};
use ic_stable_structures::{storable::Bound, Storable};
use std::borrow::Cow;

pub mod canister;
pub mod canister_state;
pub mod external_calls;
pub mod external_canister_interfaces;
pub mod external_types;
pub mod history;

pub const BILLION: u64 = 1_000_000_000;
pub const TRILLION: u128 = 1_000_000_000_000;
pub const E8S: u64 = 100_000_000;
// const BILLING_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);
const MEMO_TOP_UP_CANISTER: Memo = Memo(0x50555054); // == 'TPUP'

// ============================================================================
// Types

/// Rental conditions are kept in a global HashMap and only changed via code upgrades.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, CandidType, Deserialize, Hash)]
pub enum RentalConditionId {
    App13CH,
}

/// Set of conditions for a subnet up for rent.
/// Rental conditions are kept in a global HashMap and only changed via code upgrades.
/// Once the subnet_id is known, it is added as Some().
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, CandidType, Deserialize)]
pub struct RentalConditions {
    /// A description of the topology of this subnet.
    pub description: String,
    /// Initially None, this field is filled when a new rental subnet
    /// is created with the given topology.
    pub subnet_id: Option<Principal>,
    pub daily_cost_cycles: u128,
    pub initial_rental_period_days: u64,
    pub billing_period_days: u64,
}

impl Storable for RentalConditions {
    const BOUND: Bound = Bound::Unbounded;
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

/// The governance canister calls the SRC's proposal execution method
/// with this argument in case the proposal was valid and adopted.
#[derive(Clone, CandidType, Deserialize)]
pub struct SubnetRentalProposalPayload {
    // The user who makes the payments and enters an agreement.
    pub user: Principal,
    /// A key into the global RENTAL_CONDITIONS HashMap.
    pub rental_condition_id: RentalConditionId,
    pub proposal_id: u64,
    pub proposal_creation_time: u64,
}

/// Successful proposal execution leads to a RentalRequest.
#[derive(Clone, CandidType, PartialEq, Eq, PartialOrd, Ord, Debug, Deserialize)]
pub struct RentalRequest {
    pub user: Principal,
    /// The cost in ICP, calculated from the ICP/XDR exchange rate
    /// at UTC midnight before proposal creation time.
    pub initial_cost_icp: Tokens,
    /// The amount of ICP in SRC's main account which remain refundable.
    pub refundable_icp: Tokens,
    /// The amount that is currently locked and available only as cycles.
    pub locked_amount_icp: Tokens,
    /// The amount of cycles that are no longer refundable.
    pub locked_amount_cycles: u128,
    /// The initial proposal id will be mentioned in the subnet
    /// creation proposal. When this is found on the governance
    /// canister, polling can stop.
    pub initial_proposal_id: u64,
    /// Rental request creation date in nanoseconds since epoch.
    pub creation_date: u64,
    /// A key into the global RENTAL_CONDITIONS HashMap.
    pub rental_condition_id: RentalConditionId,
    /// ===== Data for the ICP-locking timer. =====
    /// The last time ICP were successfully locked. If this is
    /// 30d in the past, a new locking event should trigger.
    pub last_locking_time: u64,
}

impl Storable for RentalRequest {
    const BOUND: Bound = Bound::Unbounded;
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, CandidType, Deserialize)]
pub struct RentalAgreement {
    // ===== Immutable data =====
    /// The principal which paid the deposit and will be whitelisted.
    pub user: Principal,
    /// The id of the SubnetRentalRequest proposal.
    pub initial_proposal_id: u64,
    /// The id of the proposal that created the subnet. Optional in case
    /// the subnet already existed at initial proposal time.
    pub subnet_creation_proposal_id: Option<u64>,
    /// A key into the global RENTAL_CONDITIONS HashMap.
    pub rental_condition_id: RentalConditionId,
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

#[derive(CandidType, Debug, PartialEq, Eq, Clone, Deserialize)]
pub enum ExecuteProposalError {
    CallGovernanceFailed,
    CallXRCFailed(String),
    PriceCalculationError(PriceCalculationData),
    UserAlreadyRequestingSubnetRental,
    UserAlreadyHasAgreement,
    SubnetAlreadyRented,
    SubnetAlreadyRequested,
    UnauthorizedCaller,
    InsufficientFunds { have: Tokens, need: Tokens },
    TransferSrcToCmcError(String),
    NotifyTopUpError(String),
    SubnetNotRented,
}

/// The data in this struct was used in a failed attempt to calculate an ICP/XDR
/// exchange rate for the subnet rental canister.
#[derive(CandidType, Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Deserialize)]
pub struct PriceCalculationData {
    /// From the rental conditions.
    daily_cost_cycles: u128,
    /// From the rental conditions.
    initial_rental_period_days: u64,
    /// The exchange rate is a positive integer scaled by 10^decimals.
    scaled_exchange_rate_xdr_per_icp: u64,
    /// Scale factor for the exchange rate.
    decimals: u32,
}

/// The return type of the query methods `get_history_page` and
/// `get_rental_conditions_history_page`.
#[derive(CandidType, Debug, Clone, Deserialize)]
pub struct EventPage {
    /// Up to a page of events (20).
    pub events: Vec<Event>,
    /// The event number of the oldest event in the page.
    /// Used to continue with the next page by calling
    /// `get_history_page(principal, Some(continuation))` or
    /// `get_rental_conditions_history_page(Some(continuation))
    pub continuation: u64,
}
