use candid::{decode_one, encode_args, encode_one};
use ic_ledger_types::{
    AccountIdentifier, Tokens, DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_GOVERNANCE_CANISTER_ID,
    MAINNET_LEDGER_CANISTER_ID,
};
use pocket_ic::{PocketIc, PocketIcBuilder, WasmResult};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
};
use subnet_rental_canister::{
    external_types::{NnsLedgerCanisterInitPayload, NnsLedgerCanisterPayload},
    ExecuteProposalError, RentalConditions, ValidatedSubnetRentalProposal,
};

const SRC_WASM: &str = "../../subnet_rental_canister.wasm";
const LEDGER_WASM: &str = "./tests/ledger-canister_notify-method.wasm.gz";

fn setup() -> (PocketIc, candid::Principal) {
    let pic = PocketIcBuilder::new().with_nns_subnet().build();

    // Install subnet rental canister.
    let subnet_rental_canister = pic.create_canister();
    let src_wasm = fs::read(SRC_WASM).expect("Please build the wasm with ./scripts/build.sh");
    pic.install_canister(subnet_rental_canister, src_wasm, vec![], None);

    // Install ICP ledger canister.
    pic.create_canister_with_id(
        Some(MAINNET_LEDGER_CANISTER_ID),
        None,
        MAINNET_LEDGER_CANISTER_ID,
    )
    .unwrap();
    let icp_ledger_canister_wasm = fs::read(LEDGER_WASM).expect("Ledger canister wasm not found");

    let controller_and_minter =
        AccountIdentifier::new(&MAINNET_LEDGER_CANISTER_ID, &DEFAULT_SUBACCOUNT);

    let icp_ledger_init_args = NnsLedgerCanisterPayload::Init(NnsLedgerCanisterInitPayload {
        minting_account: controller_and_minter.to_string(),
        initial_values: HashMap::from([(
            controller_and_minter.to_string(),
            Tokens::from_e8s(1_000_000_000_000),
        )]),
        send_whitelist: HashSet::new(),
        transfer_fee: Some(DEFAULT_FEE),
        token_symbol: Some("ICP".to_string()),
        token_name: Some("Internet Computer".to_string()),
    });
    pic.install_canister(
        MAINNET_LEDGER_CANISTER_ID,
        icp_ledger_canister_wasm,
        encode_one(&icp_ledger_init_args).unwrap(),
        Some(MAINNET_LEDGER_CANISTER_ID),
    );

    (pic, subnet_rental_canister)
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
        MAINNET_GOVERNANCE_CANISTER_ID,
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
