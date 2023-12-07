use candid::Principal;
use ic_cdk::api::call;
use std::cell::RefCell;
use std::collections::HashMap;

type SubnetId = Principal;

#[derive(Clone, Copy)]
struct RentalConditions {
    daily_cost_e8s: u64,
    minimal_rental_period_days: u64,
}

thread_local! {
    static RENTAL_CONDITIONS: RefCell<HashMap<SubnetId, RentalConditions>> = RefCell::new(HashMap::new());
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
    let result = call(
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
