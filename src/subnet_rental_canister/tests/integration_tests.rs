use candid::{decode_one, encode_one};
use pocket_ic::{PocketIc, WasmResult};
use std::fs;
use subnet_rental_canister::{Principal, RentalConditions};

const WASM: &str = "../../subnet_rental_canister.wasm";

fn setup() -> (PocketIc, Principal) {
    let pic = PocketIc::new();
    let canister_id = pic.create_canister();
    let wasm = fs::read(WASM).expect("Please build the wasm with ./scripts/build.sh");

    pic.add_cycles(canister_id, 2_000_000_000_000);
    pic.install_canister(canister_id, wasm, vec![], None);
    (pic, canister_id)
}

#[test]
fn test_list_rental_conditions() {
    let (pic, canister_id) = setup();

    let WasmResult::Reply(res) = pic
        .query_call(
            canister_id,
            candid::Principal::anonymous(),
            "list_rental_conditions",
            encode_one(()).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };

    let conditions = decode_one::<Vec<(Principal, RentalConditions)>>(&res).unwrap();
    println!("Reply: {:?}", conditions);
}

#[test]
fn test_proposal_accepted() {
    let (pic, canister_id) = setup();

    assert!(pic
        .update_call(
            canister_id,
            candid::Principal::anonymous(),
            "on_proposal_accept",
            encode_one(()).unwrap(),
        )
        .is_ok())

}
