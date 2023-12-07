use candid::{CandidType, Deserialize, Principal};
use ic_cdk::{api::call::CallResult, call, init, query};
use std::{cell::RefCell, collections::HashMap};

type SubnetId = Principal;

const E8S: u64 = 100_000_000;
thread_local! {
    static RENTAL_CONDITIONS: RefCell<HashMap<SubnetId, RentalConditions>> = RefCell::new(HashMap::new());
}

#[derive(Debug, Clone, Copy, CandidType, Deserialize)]
pub struct RentalConditions {
    daily_cost_e8s: u64,
    minimal_rental_period_days: u64,
}
pub enum ExecuteProposalError {
    Failure(String),
}

/// Argument will be ValidatedSRProposal, created by government canister via
/// SRProposal::validate().
async fn on_proposal_accept(
    subnet_id: SubnetId,
    principals: Vec<Principal>,
    block_index: usize,
    refund_address: String,
) -> Result<(), ExecuteProposalError> {
    // assumptions:
    // - a single deposit transaction exists and covers amount

    // whitelist principal
    let result: CallResult<()> = call(
        Principal::from_text("cmc_id").unwrap(),
        "set_authorized_subnetwork_list",
        (),
    )
    .await;

    // collect rental information
    let RentalConditions {
        daily_cost_e8s,
        minimal_rental_period_days,
    } = RENTAL_CONDITIONS.with(|map| map.borrow().get(&subnet_id).unwrap().clone());

    // cost of initial period:
    let initial_cost_e8s = daily_cost_e8s * minimal_rental_period_days; // what about overflows
                                                                        // turn this amount of ICP into cycles and burn them.

    Ok(())
}

#[init]
fn init() {
    RENTAL_CONDITIONS.with(|map| {
        map.borrow_mut().insert(
            Principal::from_text("fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae")
                .unwrap(),
            RentalConditions {
                daily_cost_e8s: 100 * E8S,
                minimal_rental_period_days: 183,
            },
        );
    });
    ic_cdk::println!("Subnet rental canister initialized");
}

#[query]
fn list_rental_conditions() -> HashMap<SubnetId, RentalConditions> {
    RENTAL_CONDITIONS.with(|map| map.borrow().clone())
}
