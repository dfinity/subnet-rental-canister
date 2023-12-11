#![allow(dead_code)]

use candid::{CandidType, Decode, Deserialize, Encode, Principal as PrincipalImpl};
use ic_cdk::{api::call::CallResult, call, init, query, update};
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    DefaultMemoryImpl, StableBTreeMap, Storable,
};
use serde::Serialize;
use std::{borrow::Cow, cell::RefCell};

const LEDGER_ID: &str = "todo";
const CMC_ID: &str = "rkp4c-7iaaa-aaaaa-aaaca-cai";
// The canister_id of the SRC
const SRC_PRINCIPAL: &str = "src_principal";
// During billing, the cost in cycles is fixed, but the cost in ICP depends on the exchange rate
const XDR_COST_PER_DAY: u64 = 1;

const E8S: u64 = 100_000_000;

type Memory = VirtualMemory<DefaultMemoryImpl>;
thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    // Memory region 0
    static RENTAL_CONDITIONS: RefCell<StableBTreeMap<Principal, RentalConditions, Memory>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    // Memory region 1
    static RENTAL_AGREEMENTS: RefCell<StableBTreeMap<Principal, RentalAgreement, Memory>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1)))));
}

const MAX_PRINCIPAL_SIZE: u32 = 29;

#[derive(
    Debug, Clone, Copy, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, CandidType, Hash,
)]
pub struct Principal(PrincipalImpl);

impl From<PrincipalImpl> for Principal {
    fn from(value: PrincipalImpl) -> Self {
        Self(value)
    }
}

impl Storable for Principal {
    const BOUND: Bound = Bound::Bounded {
        max_size: MAX_PRINCIPAL_SIZE,
        is_fixed_size: false,
    };
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(self.0.as_slice().to_vec())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Self(PrincipalImpl::try_from_slice(bytes.as_ref()).unwrap())
    }
}

type SubnetId = Principal;

/// Set of conditions for a specific subnet up for rent.
#[derive(Debug, Clone, Copy, CandidType, Deserialize)]
pub struct RentalConditions {
    daily_cost_e8s: u64,
    minimal_rental_period_days: u64,
}

impl Storable for RentalConditions {
    const BOUND: Bound = Bound::Bounded {
        max_size: 16,
        is_fixed_size: true,
    };
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

/// Immutable rental agreement; mutabla data and log events should refer to it via the id.
#[derive(Debug, CandidType, Deserialize)]
struct RentalAgreement {
    id: usize,
    user: Principal,
    subnet: SubnetId,
    principals: Vec<Principal>,
    refund_subaccount: String,
    initial_period_days: u64,
    initial_period_cost_e8s: u64,
    // nanos since epoch?  TODO: figure out how times are handled in NNS canisters
    // creation_date: Date, // https://time-rs.github.io/book/how-to/create-dates.html date might be resolution enough, because we have no sub-day durations, so timezone offsets should be irrelevant.
    creation_date: u64,
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

#[init]
fn init() {
    RENTAL_CONDITIONS.with(|map| {
        map.borrow_mut().insert(
            PrincipalImpl::from_text(
                "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
            )
            .unwrap()
            .into(),
            RentalConditions {
                daily_cost_e8s: 100 * E8S,
                minimal_rental_period_days: 183,
            },
        );
    });
    ic_cdk::println!("Subnet rental canister initialized");
}

#[query]
fn list_rental_conditions() -> Vec<(SubnetId, RentalConditions)> {
    RENTAL_CONDITIONS.with(|map| {
        map.borrow()
            .iter()
            .collect::<Vec<(SubnetId, RentalConditions)>>()
    })
}

#[derive(CandidType)]
pub enum ExecuteProposalError {
    Failure(String),
}

/// TODO: Argument should be something like ValidatedSRProposal, created by government canister via
/// SRProposal::validate().
#[update]
async fn on_proposal_accept(
    subnet_id: SubnetId,
    user: Principal,
    _principals: Vec<Principal>,
    _block_index: usize,
    _refund_address: String,
) -> Result<(), ExecuteProposalError> {
    // Assumptions:
    // - A single deposit transaction exists and covers the necessary amount.
    // - The deposit was made to the <subnet_id>-subaccount of the SRC.

    // Collect rental information
    // If the governance canister was able to validate, then this entry must exist, so we can unwrap.
    let RentalConditions {
        daily_cost_e8s,
        minimal_rental_period_days,
    } = RENTAL_CONDITIONS.with(|map| map.borrow().get(&subnet_id).unwrap());

    // cost of initial period: TODO: overflows?
    let _initial_cost_e8s = daily_cost_e8s * minimal_rental_period_days;
    // turn this amount of ICP into cycles and burn them.
    // 1. transfer the right amount of ICP to the CMC
    // 2. create NotifyTopUpArg{ block_index, canister_id } from that transaction
    // 3. call CMC with the notify arg to get cycles
    // 4. burn the cycles with the system api
    // 5. set the end date of the initial period
    // 6. fill in the other rental agreement details
    // 7. add it to the rental agreement map

    // Whitelist the principal
    let result: CallResult<()> = call(
        PrincipalImpl::from_text(CMC_ID).unwrap(),
        "set_authorized_subnetwork_list",
        (Some(user), vec![subnet_id]), // TODO: figure out exact semantics of this method.
    )
    .await;
    match result {
        Ok(_) => {}
        // TODO: figure out failure modes of this method and consequences. can this call fail at all? the deposit is gone by now..
        Err((code, msg)) => {
            ic_cdk::println!("Call to CMC failed: {:?}, {}", code, msg);
            return Err(ExecuteProposalError::Failure(
                "Failed to call CMC".to_string(),
            ));
        }
    }

    Ok(())
}
