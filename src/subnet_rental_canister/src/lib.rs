use candid::{CandidType, Decode, Deserialize, Encode};
use ic_cdk::{api::cycles_burn, init, query, update};
use ic_ledger_types::{
    MAINNET_CYCLES_MINTING_CANISTER_ID, MAINNET_GOVERNANCE_CANISTER_ID, MAINNET_LEDGER_CANISTER_ID,
};
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    DefaultMemoryImpl, StableBTreeMap, Storable,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{borrow::Cow, cell::RefCell, collections::HashMap};

pub mod external_types;
mod http_request;

// During billing, the cost in cycles is fixed, but the cost in ICP depends on the exchange rate
const _XDR_COST_PER_DAY: u64 = 1;
const E8S: u64 = 100_000_000;

type SubnetId = Principal;

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    // Memory region 0
    static RENTAL_AGREEMENTS: RefCell<StableBTreeMap<Principal, RentalAgreement, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    /// Hardcoded subnets and their rental conditions.
    static SUBNETS: RefCell<HashMap<Principal, RentalConditions>> = HashMap::from([
        (candid::Principal::from_text("bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe").unwrap().into(),
            RentalConditions {
                daily_cost_e8s: 333 * E8S,
                minimal_rental_period_days: 365,
            },
        ),
        (candid::Principal::from_text("fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae").unwrap().into(),
            RentalConditions {
                daily_cost_e8s: 100 * E8S,
                minimal_rental_period_days: 183,
            },
        ),
    ]).into();
}

const MAX_PRINCIPAL_SIZE: u32 = 29;

#[derive(
    Debug, Clone, Copy, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, CandidType, Hash,
)]
pub struct Principal(pub candid::Principal);

impl From<candid::Principal> for Principal {
    fn from(value: candid::Principal) -> Self {
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
        Self(candid::Principal::try_from_slice(bytes.as_ref()).unwrap())
    }
}

/// Set of conditions for a specific subnet up for rent.
#[derive(Debug, Clone, Copy, CandidType, Deserialize)]
pub struct RentalConditions {
    daily_cost_e8s: u64,
    minimal_rental_period_days: u64,
}

/// Immutable rental agreement; mutabla data and log events should refer to it via the id.
#[derive(Debug, Clone, CandidType, Deserialize)]
struct RentalAgreement {
    user: Principal,
    subnet_id: SubnetId,
    principals: Vec<Principal>,
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
    ic_cdk::println!("Subnet rental canister initialized");
}

#[update]
fn demo_add_rental_agreement() {
    // TODO: remove this endpoint before release
    // Hardcoded rental agreement for testing
    let subnet_id = candid::Principal::from_text(
        "bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe",
    )
    .unwrap()
    .into();
    let renter = candid::Principal::from_slice(b"user1").into();
    let user = candid::Principal::from_slice(b"user2").into();
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(
            subnet_id,
            RentalAgreement {
                user: renter,
                subnet_id,
                principals: vec![renter, user],
                initial_period_days: 365,
                initial_period_cost_e8s: 333 * 365 * E8S,
                creation_date: 1702394252000000000,
            },
        )
    });
}

#[update]
fn attempt_refund(subnet_id: candid::Principal) -> Result<(), external_types::TransferError> {
    let caller = ic_cdk::caller();
    let _subaccount = get_sub_account(caller, subnet_id);
    let _icp_ledger_canister = candid::Principal::from_text(ICP_LEDGER_CANISTER_ID).unwrap();
    // TODO: try to withdraw all funds from the SRC's subaccount to the caller.
    // - fee is paid by the caller

    Ok(())
}

#[query]
fn get_sub_account(user: candid::Principal, subnet_id: candid::Principal) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(user.as_slice());
    hasher.update(subnet_id.as_slice());
    hasher.finalize().into()
}

#[query]
fn list_subnet_conditions() -> HashMap<SubnetId, RentalConditions> {
    SUBNETS.with(|map| map.borrow().clone())
}

#[query]
fn list_rental_agreements() -> Vec<RentalAgreement> {
    RENTAL_AGREEMENTS.with(|map| map.borrow().iter().map(|(_, v)| v).collect())
}

#[derive(Clone, CandidType, Deserialize)]
pub struct ValidatedSubnetRentalProposal {
    pub subnet_id: Principal,
    pub user: Principal,
    pub principals: Vec<Principal>,
}

#[derive(CandidType, Debug, Clone, Deserialize)]
pub enum ExecuteProposalError {
    Failure(String),
    SubnetAlreadyRented,
    UnauthorizedCaller,
    InsufficientFunds,
}

/// TODO: Argument should be something like ValidatedSRProposal, created by government canister via
/// SRProposal::validate().
/// validate needs to ensure:
/// - subnet not currently rented
/// - A single deposit transaction exists and covers the necessary amount.
/// - The deposit was made to the <subnet_id>-subaccount of the SRC.
#[update]
async fn accept_rental_agreement(
    ValidatedSubnetRentalProposal {
        subnet_id,
        user,
        principals,
    }: ValidatedSubnetRentalProposal,
) -> Result<(), ExecuteProposalError> {
    verify_caller_is_governance()?;

    // Collect rental information
    // If the governance canister was able to validate, then this entry must exist, so we can unwrap.
    let RentalConditions {
        daily_cost_e8s,
        minimal_rental_period_days,
    } = SUBNETS.with(|rc| *rc.borrow().get(&subnet_id).unwrap());

    // nanoseconds since epoch.
    let creation_date = ic_cdk::api::time();
    let _initial_period_end = creation_date + (minimal_rental_period_days * 86400 * 1_000_000_000);

    // cost of initial period: TODO: overflows?
    let initial_period_cost_e8s = daily_cost_e8s * minimal_rental_period_days;
    // turn this amount of ICP into cycles and burn them.

    let _cmc_canister = MAINNET_CYCLES_MINTING_CANISTER_ID;
    let _ledger_canister = MAINNET_LEDGER_CANISTER_ID;

    // 1. transfer the right amount of ICP to the CMC, if it fails, return an error
    let _sub_account = get_sub_account(user.0, subnet_id.0);
    // let result::CallResult<> = call(LEDGER, "transfer", TransferArgs).await;
    let txn_failed = false;
    if txn_failed {
        ic_cdk::println!("Balance is insufficient");
        return Err(ExecuteProposalError::InsufficientFunds);
    }
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
        initial_period_days: minimal_rental_period_days,
        initial_period_cost_e8s,
        creation_date,
    };

    // 7. add it to the rental agreement map
    if RENTAL_AGREEMENTS.with(|map| map.borrow().contains_key(&subnet_id)) {
        ic_cdk::println!(
            "Subnet is already in an active rental agreement: {:?}",
            &subnet_id
        );
        return Err(ExecuteProposalError::SubnetAlreadyRented);
    }
    // TODO: log this event in the persisted log
    ic_cdk::println!("Creating rental agreement: {:?}", &rental_agreement);
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(subnet_id, rental_agreement);
    });

    // 8. Whitelist the principal
    // let result: CallResult<()> = call(CMC, "set_authorized_subnetwork_list", (Some(user), vec![subnet_id])).await;

    Ok(())
}

#[derive(Clone, CandidType, Deserialize, Debug)]
pub struct RejectedSubnetRentalProposal {
    pub nns_proposal_id: u64,
    pub refund_address: [u8; 32],
}

fn verify_caller_is_governance() -> Result<(), ExecuteProposalError> {
    if ic_cdk::caller() != MAINNET_GOVERNANCE_CANISTER_ID {
        ic_cdk::println!("Caller is not the governance canister");
        return Err(ExecuteProposalError::UnauthorizedCaller);
    }
    Ok(())
}
