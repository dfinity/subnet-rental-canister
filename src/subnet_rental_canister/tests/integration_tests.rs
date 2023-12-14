use candid::{decode_one, encode_one};
use pocket_ic::{PocketIc, WasmResult};
use std::fs;
use subnet_rental_canister::{
    ExecuteProposalError, Principal, RejectedSubnetRentalProposal, RentalConditions,
    ValidatedSubnetRentalProposal,
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

    let conditions = decode_one::<Vec<(Principal, RentalConditions)>>(&res).unwrap();
    assert!(!conditions.is_empty());
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

    let WasmResult::Reply(res) = pic
        .update_call(
            canister_id,
            candid::Principal::from_text(subnet_rental_canister::GOVERNANCE_CANISTER_ID).unwrap(),
            "on_proposal_accept",
            encode_one(arg.clone()).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply");
    };
    let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
    assert!(res.is_ok());

    // using the same subnet again must fail
    let WasmResult::Reply(res) = pic
        .update_call(
            canister_id,
            candid::Principal::from_text(subnet_rental_canister::GOVERNANCE_CANISTER_ID).unwrap(),
            "on_proposal_accept",
            encode_one(arg).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply");
    };

    let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
    assert!(matches!(
        res,
        Err(ExecuteProposalError::SubnetAlreadyRented)
    ));
}

#[test]
fn test_on_proposal_accept_cannot_be_called_by_non_governance() {
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

    let WasmResult::Reply(res) = pic
        .update_call(
            canister_id,
            candid::Principal::anonymous(),
            "on_proposal_accept",
            encode_one(arg.clone()).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply");
    };
    let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
    assert!(matches!(res, Err(ExecuteProposalError::UnauthorizedCaller)));
}

#[test]
fn test_on_proposal_reject_cannot_be_called_by_non_governance() {
    let (pic, canister_id) = setup();

    let arg = RejectedSubnetRentalProposal {
        nns_proposal_id: 11111111,
        refund_address: [0u8; 32],
    };

    let WasmResult::Reply(res) = pic
        .update_call(
            canister_id,
            candid::Principal::anonymous(),
            "on_proposal_reject",
            encode_one(arg.clone()).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply");
    };
    let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
    assert!(matches!(res, Err(ExecuteProposalError::UnauthorizedCaller)));
}
