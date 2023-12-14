use candid::{CandidType, Decode, Deserialize, Encode};
use ic_cdk::{api::cycles_burn, init, query, update};
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    DefaultMemoryImpl, StableBTreeMap, Storable,
};
use std::{borrow::Cow, cell::RefCell, collections::HashMap};
use types::Principal;

mod api;
mod types;

const LEDGER_ID: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
const CMC_ID: &str = "rkp4c-7iaaa-aaaaa-aaaca-cai";
// The canister_id of the SRC
const _SRC_PRINCIPAL: &str = "src_principal";
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
    // Hardcoded rental agreement for testing
    let subnet_id = Principal(
        candid::Principal::from_text(
            "bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe",
        )
        .unwrap(),
    );
    let renter = Principal(candid::Principal::from_slice(b"user1"));
    let user = Principal(candid::Principal::from_slice(b"user2"));
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(
            subnet_id,
            RentalAgreement {
                user: renter,
                subnet_id,
                principals: vec![renter, user],
                refund_address: "my-wallet-address".to_owned(),
                initial_period_days: 365,
                initial_period_cost_e8s: 333 * 365 * E8S,
                creation_date: 1702394252000000000,
            },
        )
    });
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
    pub block_index: u64,
    pub refund_address: String,
}

#[derive(CandidType, Deserialize)]
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
    } = SUBNETS.with(|rc| *rc.borrow().get(&subnet_id).unwrap());

    // nanoseconds since epoch.
    let creation_date = ic_cdk::api::time();
    let _initial_period_end = creation_date + (minimal_rental_period_days * 86400 * 1_000_000_000);

    // cost of initial period: TODO: overflows?
    let initial_period_cost_e8s = daily_cost_e8s * minimal_rental_period_days;
    // turn this amount of ICP into cycles and burn them.

    let _cmc_canister = candid::Principal::from_text(CMC_ID).unwrap();
    let _ledger_canister = candid::Principal::from_text(LEDGER_ID).unwrap();

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

    // 7. add it to the rental agreement map
    if RENTAL_AGREEMENTS.with(|map| map.borrow().contains_key(&subnet_id)) {
        ic_cdk::println!(
            "Subnet is already in an active rental agreement: {:?}",
            &subnet_id
        );
        return Err(ExecuteProposalError::Failure(
            "Subnet is already in an active rental agreement".to_string(),
        ));
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
