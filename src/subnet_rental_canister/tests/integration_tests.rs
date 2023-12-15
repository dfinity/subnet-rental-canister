use candid::{decode_one, encode_args, encode_one};
use pocket_ic::{PocketIc, WasmResult};
use sha2::{Digest, Sha256};
use std::fs;
use subnet_rental_canister::{
    ExecuteProposalError, RentalConditions, ValidatedSubnetRentalProposal,
};

const WASM: &str = "../../subnet_rental_canister.wasm";

fn setup() -> (PocketIc, candid::Principal) {
    let pic = PocketIc::new();
    let canister_id = pic.create_canister();
    let wasm = fs::read(WASM).expect("Please build the wasm with ./scripts/build.sh");
    pic.add_cycles(canister_id, 2_000_000_000_000);
    pic.install_canister(canister_id, wasm, vec![], None);
    (pic, canister_id)
}

#[test]
fn test_get_sub_account() {
    let (pic, canister_id) = setup();

    let subnet_id = candid::Principal::from_text(
        "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
    )
    .unwrap();
    let user = candid::Principal::from_slice(b"user1");

    let WasmResult::Reply(res) = pic
        .query_call(
            canister_id,
            candid::Principal::anonymous(),
            "get_sub_account",
            encode_args((user, subnet_id)).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };

    let actual = decode_one::<[u8; 32]>(&res).unwrap();

    let mut hasher = Sha256::new();
    hasher.update(user.as_slice());
    hasher.update(subnet_id.as_slice());
    let should: [u8; 32] = hasher.finalize().into();

    assert_eq!(actual, should);
}

#[test]
fn test_list_rental_conditions() {
    let (pic, canister_id) = setup();

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

    let conditions = decode_one::<Vec<(candid::Principal, RentalConditions)>>(&res).unwrap();
    assert!(!conditions.is_empty());
}

fn add_test_rental_agreement(
    pic: &PocketIc,
    canister_id: &candid::Principal,
    subnet_id_str: &str,
) -> WasmResult {
    let arg = ValidatedSubnetRentalProposal {
        subnet_id: candid::Principal::from_text(subnet_id_str).unwrap().into(),
        user: candid::Principal::from_slice(b"user1").into(),
        principals: vec![],
    };

    pic.update_call(
        *canister_id,
        candid::Principal::from_text(subnet_rental_canister::GOVERNANCE_CANISTER_ID).unwrap(),
        "accept_rental_agreement",
        encode_one(arg.clone()).unwrap(),
    )
    .unwrap()
}

#[test]
fn test_proposal_accepted() {
    let (pic, canister_id) = setup();

    // the first time must succeed
    let wasm_res = add_test_rental_agreement(
        &pic,
        &canister_id,
        "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
    );

    let WasmResult::Reply(res) = wasm_res else {
        panic!("Expected a reply");
    };

    let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
    assert!(res.is_ok());

    // using the same subnet again must fail
    let wasm_res = add_test_rental_agreement(
        &pic,
        &canister_id,
        "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
    );
    let WasmResult::Reply(res) = wasm_res else {
        panic!("Expected a reply");
    };

    let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
    assert!(matches!(
        res,
        Err(ExecuteProposalError::SubnetAlreadyRented)
    ));
}

#[test]
fn test_accept_rental_agreement_cannot_be_called_by_non_governance() {
    let (pic, canister_id) = setup();

    let arg = ValidatedSubnetRentalProposal {
        subnet_id: candid::Principal::from_text(
            "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
        )
        .unwrap()
        .into(),
        user: candid::Principal::from_slice(b"user1").into(),
        principals: vec![],
    };

    let WasmResult::Reply(res) = pic
        .update_call(
            canister_id,
            candid::Principal::anonymous(),
            "accept_rental_agreement",
            encode_one(arg.clone()).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply");
    };
    let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
    println!("res {:?}", res);
    assert!(matches!(res, Err(ExecuteProposalError::UnauthorizedCaller)));
}
