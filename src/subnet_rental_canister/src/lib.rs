use std::{cell::RefCell, collections::HashMap};

use candid::{CandidType, Principal};
use ic_cdk::{init, query};

type SubnetId = Principal;

const E8S: u64 = 100_000_000;
thread_local! {
    static RENTAL_CONDITIONS: RefCell<HashMap<SubnetId, RentalConditions>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy, CandidType)]
struct RentalConditions {
    daily_cost_e8s: u64,
    minimal_rental_period_days: u64,
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
fn list_rental_conditions() -> Vec<(SubnetId, RentalConditions)> {
    RENTAL_CONDITIONS.with(|map| map.borrow().iter().map(|(k, v)| (*k, *v)).collect())
}
