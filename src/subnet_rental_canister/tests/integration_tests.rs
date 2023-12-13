use candid::{decode_one, encode_one};
use pocket_ic::{PocketIc, WasmResult};
use std::fs;
use subnet_rental_canister::{Principal, RentalConditions};

const WASM: &str = "../../subnet_rental_canister.wasm";

#[test]
fn test_list_rental_conditions() {
    let pic = PocketIc::new();
    let canister_id = pic.create_canister();
    let wasm = fs::read(WASM).expect("Please build the wasm with ./scripts/build.sh");

    pic.add_cycles(canister_id, 2_000_000_000_000);
    pic.install_canister(canister_id, wasm, vec![], None);

    let WasmResult::Reply(res) = pic
        .query_call(
            canister_id,
            candid::Principal::anonymous(),
            "list_subnet_conditions",
            encode_one(()).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };

    let conditions = decode_one::<Vec<(Principal, RentalConditions)>>(&res).unwrap();
    assert!(!conditions.is_empty());
}
