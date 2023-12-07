use candid::{decode_one, encode_one, CandidType, Principal};
use pocket_ic::{PocketIc, WasmResult};
use serde::Deserialize;
use std::{collections::HashMap, fs};
const WASM: &str = "../../subnet_rental_canister.wasm";

#[derive(Debug, Clone, Copy, CandidType, Deserialize)]
pub struct RentalConditions {
    daily_cost_e8s: u64,
    minimal_rental_period_days: u64,
}

#[test]
fn test_list_rental_conditions() {
    let pic = PocketIc::new();
    let canister_id = pic.create_canister();
    let wasm = fs::read(WASM).expect("Please build the wasm first");

    pic.add_cycles(canister_id, 2_000_000_000_000);
    pic.install_canister(canister_id, wasm, vec![], None);

    let WasmResult::Reply(res) = pic
        .query_call(
            canister_id,
            Principal::anonymous(),
            "list_rental_conditions",
            encode_one(()).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };

    let conditions = decode_one::<HashMap<Principal, RentalConditions>>(&res).unwrap();
    println!("Reply: {:?}", conditions);
}
