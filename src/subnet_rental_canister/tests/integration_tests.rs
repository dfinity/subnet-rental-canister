use candid::{decode_one, encode_args, encode_one, Principal};
use ic_ledger_types::{
    AccountIdentifier, Tokens, DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_CYCLES_MINTING_CANISTER_ID,
    MAINNET_GOVERNANCE_CANISTER_ID, MAINNET_LEDGER_CANISTER_ID,
};
use pocket_ic::{PocketIc, PocketIcBuilder, WasmResult};
use std::{
    collections::{HashMap, HashSet},
    fs,
};
use subnet_rental_canister::{
    external_types::{
        CyclesCanisterInitPayload, NnsLedgerCanisterInitPayload, NnsLedgerCanisterPayload,
    },
    ExecuteProposalError, RentalConditions, ValidatedSubnetRentalProposal,
};

const SRC_WASM: &str = "../../subnet_rental_canister.wasm";
const LEDGER_WASM: &str = "./tests/ledger-canister.wasm.gz";
const CMC_WASM: &str = "./tests/cycles-minting-canister.wasm.gz";

const SUBNET_FOR_RENT: &str = "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae";
const E8S: u64 = 100_000_000;
const USER_1: Principal = Principal::from_slice(b"user1");
const USER_1_INITIAL_BALANCE: Tokens = Tokens::from_e8s(1_000 * E8S);
const USER_2: Principal = Principal::from_slice(b"user2");
const USER_2_INITIAL_BALANCE: Tokens = Tokens::from_e8s(DEFAULT_FEE.e8s() * 2);

fn install_cmc(pic: &PocketIc) {
    pic.create_canister_with_id(None, None, MAINNET_CYCLES_MINTING_CANISTER_ID)
        .unwrap();
    let cmc_wasm = fs::read(CMC_WASM).expect("Could not find the patched CMC wasm");

    let init_arg: Option<CyclesCanisterInitPayload> = Some(CyclesCanisterInitPayload {
        exchange_rate_canister: None,
        last_purged_notification: None,
        governance_canister_id: Some(MAINNET_GOVERNANCE_CANISTER_ID),
        minting_account_id: None,
        ledger_canister_id: Some(MAINNET_LEDGER_CANISTER_ID),
    });

    pic.install_canister(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        cmc_wasm,
        encode_args((init_arg,)).unwrap(),
        None,
    );
}

fn install_ledger(pic: &PocketIc) {
    pic.create_canister_with_id(None, None, MAINNET_LEDGER_CANISTER_ID)
        .unwrap();
    let icp_ledger_canister_wasm = fs::read(LEDGER_WASM)
        .expect("Download the test wasm files with ./scripts/download_wasms.sh");

    let minter = AccountIdentifier::new(&MAINNET_LEDGER_CANISTER_ID, &DEFAULT_SUBACCOUNT);
    let user_1 = AccountIdentifier::new(&USER_1, &DEFAULT_SUBACCOUNT);
    let user_2 = AccountIdentifier::new(&USER_2, &DEFAULT_SUBACCOUNT);

    let icp_ledger_init_args = NnsLedgerCanisterPayload::Init(NnsLedgerCanisterInitPayload {
        minting_account: minter.to_string(),
        initial_values: HashMap::from([
            (minter.to_string(), Tokens::from_e8s(1_000_000_000 * E8S)),
            (user_1.to_string(), USER_1_INITIAL_BALANCE),
            (user_2.to_string(), USER_2_INITIAL_BALANCE),
        ]),
        send_whitelist: HashSet::new(),
        transfer_fee: Some(DEFAULT_FEE),
        token_symbol: Some("ICP".to_string()),
        token_name: Some("Internet Computer".to_string()),
    });
    pic.install_canister(
        MAINNET_LEDGER_CANISTER_ID,
        icp_ledger_canister_wasm,
        encode_one(&icp_ledger_init_args).unwrap(),
        None,
    );
}

fn setup() -> (PocketIc, Principal) {
    let pic = PocketIcBuilder::new().with_nns_subnet().build();

    install_ledger(&pic);
    install_cmc(&pic);

    // Install subnet rental canister.
    let subnet_rental_canister = pic.create_canister();
    let src_wasm = fs::read(SRC_WASM).expect("Build the wasm with ./scripts/build.sh");
    pic.install_canister(subnet_rental_canister, src_wasm, vec![], None);

    (pic, subnet_rental_canister)
}

// #[test]
fn _test_authorization() {
    // This test is incomplete because with PocketIC, we cannot create negative whitelist tests.
    let pic = PocketIcBuilder::new()
        .with_nns_subnet()
        .with_application_subnet()
        .with_application_subnet()
        .build();
    let _subnet_nns = pic.topology().get_nns().unwrap();
    let subnet_1 = pic.topology().get_app_subnets()[0];
    let _subnet_2 = pic.topology().get_app_subnets()[1];

    install_cmc(&pic);
    let user1 = Principal::from_slice(b"user1");
    let _user2 = Principal::from_slice(b"user2");

    #[derive(candid::CandidType)]
    struct Arg {
        pub who: Option<candid::Principal>,
        pub subnets: Vec<candid::Principal>,
    }
    let arg = Arg {
        who: Some(user1),
        subnets: vec![subnet_1],
    };

    pic.update_call(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        MAINNET_GOVERNANCE_CANISTER_ID,
        "set_authorized_subnetwork_list",
        encode_one(arg).unwrap(),
    )
    .unwrap();
}

#[test]
fn test_list_rental_conditions() {
    let (pic, canister_id) = setup();

    let WasmResult::Reply(res) = pic
        .query_call(
            canister_id,
            Principal::anonymous(),
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

fn add_test_rental_agreement(
    pic: &PocketIc,
    canister_id: &Principal,
    subnet_id_str: &str,
) -> WasmResult {
    let user = Principal::from_text(subnet_id_str).unwrap();
    let arg = ValidatedSubnetRentalProposal {
        subnet_id: user,
        user: USER_1,
        principals: vec![user],
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
    let wasm_res = add_test_rental_agreement(&pic, &canister_id, SUBNET_FOR_RENT);

    let WasmResult::Reply(res) = wasm_res else {
        panic!("Expected a reply");
    };

    let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
    assert!(res.is_ok());
    println!("res {:?}", res);

    // using the same subnet again must fail
    let wasm_res = add_test_rental_agreement(&pic, &canister_id, SUBNET_FOR_RENT);
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
        subnet_id: Principal::from_text(SUBNET_FOR_RENT).unwrap(),
        user: USER_1,
        principals: vec![],
    };

    let WasmResult::Reply(res) = pic
        .update_call(
            canister_id,
            Principal::anonymous(),
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
