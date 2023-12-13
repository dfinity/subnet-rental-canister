use candid::Principal as PrincipalImpl;
use candid::{decode_one, encode_one};
use pocket_ic::{PocketIc, WasmResult};
use std::fs;
use subnet_rental_canister::{Principal, RentalConditions, ValidatedSubnetRentalProposal};

const WASM: &str = "../../subnet_rental_canister.wasm";

fn setup() -> (PocketIc, PrincipalImpl) {
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

    let arg = ValidatedSubnetRentalProposal {
        subnet_id: candid::Principal::from_text(
            "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
        )
        .unwrap()
        .into(),
        user: candid::Principal::from_slice(b"user1").into(),
        principals: vec![],
        block_index: 0,
        refund_address: "ok".to_string(),
    };

    assert!(pic
        .update_call(
            canister_id,
            candid::Principal::anonymous(),
            "on_proposal_accept",
            encode_one(arg).unwrap(),
        )
        .is_ok())
}
