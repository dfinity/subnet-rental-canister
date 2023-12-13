#![allow(non_snake_case)]

use candid::{CandidType, Decode, Deserialize, Encode, Principal as PrincipalImpl};
use ic_cdk::{api::cycles_burn, init, query, update};
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    DefaultMemoryImpl, StableBTreeMap, Storable,
};
use serde::Serialize;
use std::{
    borrow::{BorrowMut, Cow},
    cell::RefCell,
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

mod types;

const LEDGER_ID: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
const CMC_ID: &str = "rkp4c-7iaaa-aaaaa-aaaca-cai";
// The canister_id of the SRC
const _SRC_PRINCIPAL: &str = "src_principal";
// During billing, the cost in cycles is fixed, but the cost in ICP depends on the exchange rate
const _XDR_COST_PER_DAY: u64 = 1;
const E8S: u64 = 100_000_000;
const MAX_PRINCIPAL_SIZE: u32 = 29;

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    // Memory region 0
    static RENTAL_AGREEMENTS: RefCell<StableBTreeMap<Principal, RentalAgreement, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    static RENTAL_CONDITIONS: HashMap<Principal, RentalConditions> = HashMap::new();
}

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

/// Immutable rental agreement; mutabla data and log events should refer to it via the id.
#[derive(Debug, CandidType, Deserialize)]
struct RentalAgreement {
    user: Principal,
    subnet_id: SubnetId,
    principals: Vec<Principal>,
    refund_address: String,
    initial_period_days: u64,
    initial_period_cost_e8s: u64,
    // nanoseconds since epoch
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
        map.clone().borrow_mut().insert(
            PrincipalImpl::from_text(
                "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
            )
            .unwrap()
            .into(),
            RentalConditions {
                daily_cost_e8s: 100 * E8S,
                minimal_rental_period_days: 183,
            },
        )
    });
    ic_cdk::println!("Subnet rental canister initialized");
}

#[query]
fn list_rental_conditions() -> HashMap<SubnetId, RentalConditions> {
    RENTAL_CONDITIONS.with(|rc| rc.clone())
}

#[derive(CandidType, Deserialize)]
pub struct ValidatedSubnetRentalProposal {
    pub subnet_id: Principal,
    pub user: Principal,
    pub principals: Vec<Principal>,
    pub block_index: u64,
    pub refund_address: String,
}

#[derive(CandidType)]
pub enum ExecuteProposalError {
    Failure(String),
}

/// TODO: Argument should be something like ValidatedSRProposal, created by government canister via
/// SRProposal::validate().
/// validate needs to ensure:
/// - subnet not currently rented
/// - A single deposit transaction exists and covers the necessary amount.
/// - The deposit was made to the <subnet_id>-subaccount of the SRC.
#[update]
async fn on_proposal_accept(
    ValidatedSubnetRentalProposal {
        subnet_id,
        user,
        principals,
        block_index: _block_index,
        refund_address,
    }: ValidatedSubnetRentalProposal,
) -> Result<(), ExecuteProposalError> {
    // TODO: need access control: only the governance canister may call this method.
    // Collect rental information
    // If the governance canister was able to validate, then this entry must exist, so we can unwrap.
    let RentalConditions {
        daily_cost_e8s,
        minimal_rental_period_days,
    } = RENTAL_CONDITIONS.with(|rc| *rc.get(&subnet_id).unwrap());

    // nanoseconds since epoch.
    let creation_date = ic_cdk::api::time();
    let _initial_period_end = creation_date + minimal_rental_period_days * 86400 * 1_000_000_000;

    // cost of initial period: TODO: overflows?
    let initial_period_cost_e8s = daily_cost_e8s * minimal_rental_period_days;
    // turn this amount of ICP into cycles and burn them.

    let _CMC = PrincipalImpl::from_text(CMC_ID).unwrap();
    let _LEDGER = PrincipalImpl::from_text(LEDGER_ID).unwrap();

    // 1. transfer the right amount of ICP to the CMC
    // let result: CallResult<> = call(LEDGER, "transfer", TransferArgs).await;
    // 2. create NotifyTopUpArg{ block_index, canister_id } from that transaction
    // 3. call CMC with the notify arg to get cycles
    // 4. burn the cycles with the system api. the amount depends on the current exchange rate.
    cycles_burn(0);
    // 5. set the end date of the initial period
    // 6. fill in the other rental agreement details
    let rental_agreement = RentalAgreement {
        user,
        subnet_id,
        principals,
        refund_address,
        initial_period_days: minimal_rental_period_days,
        initial_period_cost_e8s,
        creation_date,
    };
    // TODO: log this event in the persisted log
    ic_cdk::println!("Creating rental agreement: {:?}", &rental_agreement);

    // 7. add it to the rental agreement map
    // TODO: double check if there exists a rental agreement with this subnet_id.
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(subnet_id.into(), rental_agreement);
    });

    // 8. Whitelist the principal
    // let result: CallResult<()> = call(CMC, "set_authorized_subnetwork_list", (Some(user), vec![subnet_id])).await;

    Ok(())
}
