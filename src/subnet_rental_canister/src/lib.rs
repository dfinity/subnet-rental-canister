use candid::{CandidType, Decode, Deserialize, Encode, Nat, Principal};
use canister_state::{set_rental_conditions, RENTAL_CONDITIONS};
use external_types::NotifyError;
use history::{Event, EventType, History};
use ic_cdk::println;
use ic_ledger_types::{
    transfer, AccountIdentifier, Memo, Subaccount, Tokens, TransferArgs, TransferError,
    DEFAULT_FEE, MAINNET_CYCLES_MINTING_CANISTER_ID, MAINNET_GOVERNANCE_CANISTER_ID,
    MAINNET_LEDGER_CANISTER_ID,
};
use ic_stable_structures::Memory;
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    DefaultMemoryImpl, StableBTreeMap, Storable,
};
use icrc_ledger_types::icrc1::account::Account;
use icrc_ledger_types::icrc2::transfer_from::{TransferFromArgs, TransferFromError};

use serde::Serialize;
use std::{borrow::Cow, cell::RefCell, time::Duration};

use crate::external_types::{
    IcpXdrConversionRate, IcpXdrConversionRateResponse, NotifyTopUpArg,
    SetAuthorizedSubnetworkListArgs,
};

pub mod canister;
pub mod canister_state;
pub mod external_types;
pub mod history;
mod http_request;

pub const TRILLION: u128 = 1_000_000_000_000;
pub const E8S: u64 = 100_000_000;
const MAX_PRINCIPAL_SIZE: u32 = 29;
const BILLING_INTERVAL: Duration = Duration::from_secs(60 * 60); // hourly
const MEMO_TOP_UP_CANISTER: Memo = Memo(0x50555054); // == 'TPUP'

// ============================================================================
// Types

/// Rental conditions are kept in a global HashMap and only changed via code upgrades.
#[derive(Debug, Clone, Copy, CandidType, Deserialize, PartialEq, Eq, Hash)]
pub enum RentalConditionType {
    App13,
}

const APP13: RentalConditions = RentalConditions {
    daily_cost_cycles: 835 * TRILLION,
    initial_rental_period_days: 180,
    billing_period_days: 30,
};

/// Set of conditions for a specific subnet up for rent.
/// The subnet_id is the associated key in the StableBTreeMap.
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

#[derive(Clone, CandidType, Deserialize)]
pub enum SubnetInfo {
    /// A description of the desired topology.
    TopologyDescription(String),
    /// If this is used, the SRC attempts to make the given subnet
    /// available for rent immediately.
    ExistingSubnetId(Principal),
}

/// The governance canister calls the SRC with this argument twice.
/// First in the validation method, which may either reject the proposal
/// or return a 'Rendering' which will be shown to the voter.
/// The second time in the proposal execution method, in case the proposal
/// was valid and adopted.
#[derive(Clone, CandidType, Deserialize)]
pub struct SubnetRentalProposalPayload {
    // The tenant, who makes the payments
    pub user: Principal,
    /// Either a description of the desired topology
    /// or an existing subnet id.
    pub subnet_info: SubnetInfo,
}

/// Successful proposal execution leads to a RentalRequest.
#[derive(Clone, CandidType, Deserialize)]
pub struct RentalRequest {
    user: Principal,
    locked_amount_cycles: u128,
    initial_proposal_id: u64,
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

/// Immutable rental agreement. A rental agreement is
/// uniquely identified by the (subnet_id, creation_date) 'composite key'.
#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct RentalAgreement {
    // ===== Immutable data =====
    /// The principal which paid the deposit and will be whitelisted.
    pub user: Principal,
    /// The subnet to be rented.
    pub subnet_id: Principal,
    /// Other principals to be whitelisted.
    pub principals: Vec<Principal>,
    /// The ICP ledger subaccount to which the user makes payments.
    pub user_subaccount: Subaccount,
    /// The id of the SubnetRentalRequest proposal.
    pub initial_proposal_id: u64,
    /// The id of the proposal that created the subnet. Optional in case
    /// the subnet already existed at initial proposal time.
    pub subnet_creation_proposal_id: Option<u64>,
    /// Rental agreement creation date in nanoseconds since epoch.
    pub creation_date: u64,
    // ===== Mutable data =====
    /// The date in nanos since epoch until which the rental agreement is paid for.
    pub covered_until: u64,
    /// This subnet's share of cycles among the SRC's cycles.
    /// Increased by the payment process (via timer).
    /// Decreased by the burning process (via heartbeat).
    pub cycles_balance: u128,
    /// The last point in time in nanos since epoch when cycles were burned in a heartbeat.
    pub last_burned: u64,
}

impl RentalAgreement {
    pub fn get_rental_conditions(&self) -> RentalConditions {
        // unwrap justified because no rental agreement can exist without rental conditions
        RENTAL_CONDITIONS.with(|map| map.borrow().get(&self.subnet_id).unwrap())
    }
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
    TransferUserToSrcError(TransferFromError),
    TransferSrcToCmcError(TransferError),
    NotifyTopUpError(NotifyError),
    SubnetNotRented,
}

// ============================================================================
// Outcalls

async fn whitelist_principals(subnet_id: candid::Principal, principals: &Vec<Principal>) {
    for user in principals {
        ic_cdk::call::<_, ()>(
            MAINNET_CYCLES_MINTING_CANISTER_ID,
            "set_authorized_subnetwork_list",
            (SetAuthorizedSubnetworkListArgs {
                who: Some(user.clone()),
                subnets: vec![subnet_id], // TODO: Add to the current list, don't overwrite
            },),
        )
        .await
        .expect("Failed to call CMC"); // TODO: handle error
    }
}

async fn delist_principals(_subnet_id: candid::Principal, principals: &Vec<candid::Principal>) {
    // TODO: if we allow multiple subnets per user:
    // first read the current list,
    // remove this subnet from the list and then
    // re-whitelist the principal for the remaining list
    for user in principals {
        ic_cdk::call::<_, ()>(
            MAINNET_CYCLES_MINTING_CANISTER_ID,
            "set_authorized_subnetwork_list",
            (SetAuthorizedSubnetworkListArgs {
                who: Some(*user),
                subnets: vec![],
            },),
        )
        .await
        .expect("Failed to call CMC"); // TODO: handle error
    }
}

async fn notify_top_up(block_index: u64) -> Result<u128, NotifyError> {
    ic_cdk::call::<_, (Result<u128, NotifyError>,)>(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        "notify_top_up",
        (NotifyTopUpArg {
            block_index,
            canister_id: ic_cdk::id(),
        },),
    )
    .await
    .expect("Failed to call CMC") // TODO: handle error
    .0
    // TODO: In the canister logs, the CMC claims that the burning of ICPs failed, but the cycles are minted anyway.
    // It states that the "transfer fee should be 0.00010000 Token", but that fee is hardcoded to
    // (ZERO)[https://sourcegraph.com/github.com/dfinity/ic@8126ad2fab0196908d9456a65914a3e05179ac4b/-/blob/rs/nns/cmc/src/main.rs?L1835]
    // in the CMC, and cannot be changed from outside. What's going on here?
}

async fn transfer_to_cmc(amount: Tokens) -> Result<u64, TransferError> {
    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        TransferArgs {
            to: AccountIdentifier::new(
                &MAINNET_CYCLES_MINTING_CANISTER_ID,
                &Subaccount::from(ic_cdk::id()),
            ),
            fee: DEFAULT_FEE,
            from_subaccount: None,
            amount,
            memo: MEMO_TOP_UP_CANISTER,
            created_at_time: None,
        },
    )
    .await
    .expect("Failed to call ledger canister") // TODO: handle error
}

async fn icrc2_transfer_to_src(
    user: candid::Principal,
    amount: Tokens,
) -> Result<u128, TransferFromError> {
    ic_cdk::call::<_, (Result<u128, TransferFromError>,)>(
        MAINNET_LEDGER_CANISTER_ID,
        "icrc2_transfer_from",
        (TransferFromArgs {
            to: Account {
                owner: ic_cdk::id(),
                subaccount: None,
            },
            fee: None,
            spender_subaccount: None,
            from: Account {
                owner: user,
                subaccount: None,
            },
            memo: None,
            created_at_time: None,
            amount: Nat::from(amount.e8s()),
        },),
    )
    .await
    .expect("Failed to call ledger canister") // TODO: handle error
    .0
}

async fn get_historical_avg_exchange_rate_cycles_per_e8s(timestamp: u64) -> u64 {
    // TODO: implement
    println!(
        "Getting historical average exchange rate for timestamp {}",
        timestamp
    );
    get_exchange_rate_cycles_per_e8s().await
}

async fn get_current_avg_exchange_rate_cycles_per_e8s() -> u64 {
    // TODO: implement
    get_exchange_rate_cycles_per_e8s().await
}

async fn get_exchange_rate_cycles_per_e8s() -> u64 {
    let IcpXdrConversionRateResponse {
        data: IcpXdrConversionRate {
            xdr_permyriad_per_icp,
            ..
        },
        ..
    } = ic_cdk::call::<_, (IcpXdrConversionRateResponse,)>(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        "get_icp_xdr_conversion_rate",
        (),
    )
    .await
    .expect("Failed to call CMC") // TODO: handle error
    .0;

    xdr_permyriad_per_icp
}

fn verify_caller_is_governance() -> Result<(), ExecuteProposalError> {
    if ic_cdk::caller() != MAINNET_GOVERNANCE_CANISTER_ID {
        println!("Caller is not the governance canister");
        return Err(ExecuteProposalError::UnauthorizedCaller);
    }
    Ok(())
}

fn days_to_nanos(days: u64) -> u64 {
    days * 24 * 60 * 60 * 1_000_000_000
}

// ============================================================================
// Misc

/// Called in canister_init
pub fn set_initial_rental_conditions() {
    set_rental_conditions(
        candid::Principal::from_text(
            "bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe",
        )
        .unwrap(),
        1_000 * TRILLION,
        365,
        30,
    );
    set_rental_conditions(
        candid::Principal::from_text(
            "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
        )
        .unwrap(),
        2_000 * TRILLION,
        183,
        30,
    );
}
