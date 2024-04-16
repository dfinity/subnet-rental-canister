use candid::{decode_one, encode_args, encode_one, CandidType, Principal};
use ic_ledger_types::{
    AccountBalanceArgs, AccountIdentifier, Memo, Subaccount, Tokens, TransferArgs, TransferResult,
    DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_CYCLES_MINTING_CANISTER_ID,
    MAINNET_GOVERNANCE_CANISTER_ID, MAINNET_LEDGER_CANISTER_ID,
};

use pocket_ic::{PocketIc, PocketIcBuilder, WasmResult};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    time::UNIX_EPOCH,
};
use subnet_rental_canister::{
    external_canister_interfaces::{
        exchange_rate_canister::EXCHANGE_RATE_CANISTER_PRINCIPAL_STR,
        governance_canister::GOVERNANCE_CANISTER_PRINCIPAL_STR,
    },
    external_types::{
        CmcInitPayload, FeatureFlags, NnsLedgerCanisterInitPayload, NnsLedgerCanisterPayload,
    },
    history::Event,
    ExecuteProposalError, RentalConditionId, RentalConditions, RentalRequest,
    SubnetRentalProposalPayload, E8S,
};

const SRC_WASM: &str = "../../subnet_rental_canister.wasm";
const LEDGER_WASM: &str = "./tests/ledger-canister.wasm.gz";
const CMC_WASM: &str = "./tests/cycles-minting-canister.wasm.gz";
const XRC_WASM: &str = "./tests/exchange-rate-canister.wasm.gz";
const SRC_ID: Principal =
    Principal::from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xE0, 0x00, 0x00, 0x01, 0x01]); // lxzze-o7777-77777-aaaaa-cai
const _SUBNET_FOR_RENT: &str = "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae";
const USER_1: Principal = Principal::from_slice(b"user1");
const USER_1_INITIAL_BALANCE: Tokens = Tokens::from_e8s(1_000_000_000 * E8S);
const USER_2: Principal = Principal::from_slice(b"user2");
const USER_2_INITIAL_BALANCE: Tokens = Tokens::from_e8s(DEFAULT_FEE.e8s() * 2);

fn install_cmc(pic: &PocketIc) {
    pic.create_canister_with_id(None, None, MAINNET_CYCLES_MINTING_CANISTER_ID)
        .unwrap();
    let cmc_wasm = fs::read(CMC_WASM).expect("Could not find the patched CMC wasm");
    let minter = AccountIdentifier::new(&MAINNET_GOVERNANCE_CANISTER_ID, &DEFAULT_SUBACCOUNT);
    let init_arg = CmcInitPayload {
        governance_canister_id: Some(MAINNET_GOVERNANCE_CANISTER_ID),
        minting_account_id: minter.to_string(),
        ledger_canister_id: Some(MAINNET_LEDGER_CANISTER_ID),
        last_purged_notification: None,
        exchange_rate_canister: None,
        cycles_ledger_canister_id: None,
    };

    pic.install_canister(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        cmc_wasm,
        encode_args((Some(init_arg),)).unwrap(),
        None,
    );
}

fn install_xrc(pic: &PocketIc) {
    let xrc_principal = Principal::from_text(EXCHANGE_RATE_CANISTER_PRINCIPAL_STR).unwrap();
    pic.create_canister_with_id(None, None, xrc_principal)
        .unwrap();
    let xrc_wasm = fs::read(XRC_WASM).expect("Failed to read XRC wasm");
    pic.install_canister(xrc_principal, xrc_wasm, vec![], None);
}

fn install_ledger(pic: &PocketIc) {
    pic.create_canister_with_id(None, None, MAINNET_LEDGER_CANISTER_ID)
        .unwrap();
    let icp_ledger_canister_wasm = fs::read(LEDGER_WASM)
        .expect("Download the test wasm files with ./scripts/download_wasms.sh");

    let minter = AccountIdentifier::new(&MAINNET_GOVERNANCE_CANISTER_ID, &DEFAULT_SUBACCOUNT);
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
        feature_flags: Some(FeatureFlags { icrc2: true }),
    });
    pic.install_canister(
        MAINNET_LEDGER_CANISTER_ID,
        icp_ledger_canister_wasm,
        encode_one(&icp_ledger_init_args).unwrap(),
        None,
    );
}

fn setup() -> (PocketIc, Principal) {
    let pic = PocketIcBuilder::new()
        .with_nns_subnet()
        // needed for XRC
        .with_ii_subnet()
        .build();

    install_ledger(&pic);
    install_cmc(&pic);
    install_xrc(&pic);

    // Install subnet rental canister.
    let subnet_rental_canister = pic.create_canister_with_id(None, None, SRC_ID).unwrap();
    let src_wasm = fs::read(SRC_WASM).expect("Build the wasm with ./scripts/build.sh");
    pic.install_canister(subnet_rental_canister, src_wasm, vec![], None);
    pic.add_cycles(subnet_rental_canister, 1_000 * 1_000_000_000_000);
    (pic, subnet_rental_canister)
}

fn make_initial_transfer(pic: &PocketIc, src_principal: Principal, user_principal: Principal) {
    // user finds rental conditions
    let res = query::<Vec<(RentalConditionId, RentalConditions)>>(
        pic,
        src_principal,
        None,
        "list_rental_conditions",
        encode_one(()).unwrap(),
    );
    let (rental_condition_id, ref _rental_conditions) = res[0];
    // user finds current price by consulting SRC
    let needed_icp = update::<Result<Tokens, String>>(
        pic,
        src_principal,
        None,
        "get_todays_price",
        rental_condition_id,
    )
    .unwrap();
    // user transfers some ICP to SRC
    let transfer_args = TransferArgs {
        memo: Memo(0),
        amount: needed_icp,
        fee: DEFAULT_FEE,
        from_subaccount: None,
        to: AccountIdentifier::new(&src_principal, &Subaccount::from(user_principal)),
        created_at_time: None,
    };
    let _res = update::<TransferResult>(
        pic,
        MAINNET_LEDGER_CANISTER_ID,
        Some(user_principal),
        "transfer",
        transfer_args,
    )
    .unwrap();
}

fn set_mock_exchange_rate(
    pic: &PocketIc,
    time_secs: u64,
    exchange_rate_icp_per_xdr: u64,
    decimals: u32,
) {
    let midnight = time_secs - time_secs % 86400;
    let arg: Vec<(u64, (u64, u32))> = vec![(midnight, (exchange_rate_icp_per_xdr, decimals))];
    update::<()>(
        pic,
        Principal::from_text(EXCHANGE_RATE_CANISTER_PRINCIPAL_STR).unwrap(),
        None,
        "set_exchange_rate_data",
        arg,
    );
}

#[test]
fn test_initial_proposal() {
    let (pic, src_principal) = setup();

    let user_principal = USER_1;

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().duration_since(UNIX_EPOCH).unwrap().as_secs();
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);

    // user performs preparations
    make_initial_transfer(&pic, src_principal, user_principal);

    // user creates proposal
    let now = now * 1_000_000_000;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time: now,
    };
    // run proposal
    update::<Result<(), ExecuteProposalError>>(
        &pic,
        src_principal,
        Some(Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap()),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();

    // assert state is as expected
    let src_history = query::<Vec<Event>>(
        &pic,
        src_principal,
        None,
        "get_history",
        None::<Option<Principal>>,
    );
    let user_history = query::<Vec<Event>>(
        &pic,
        src_principal,
        None,
        "get_history",
        Some(user_principal),
    );
    // think of a better test than length
    assert_eq!(src_history.len(), 1);
    assert_eq!(user_history.len(), 2);

    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, src_principal, None, "list_rental_requests", ());
    assert_eq!(rental_requests.len(), 1);
    let RentalRequest {
        user,
        refundable_icp,
        locked_amount_cycles: _,
        initial_proposal_id: _,
        creation_date: _,
        rental_condition_id,
        last_locking_time: _,
        lock_amount_icp: _,
    } = rental_requests[0];
    assert_eq!(user, user_principal);
    assert_eq!(rental_condition_id, RentalConditionId::App13CH);

    // get refund
    let balance_before = check_balance(&pic, user_principal, DEFAULT_SUBACCOUNT);
    let res = update::<Result<u64, String>>(&pic, src_principal, None, "refund", ());
    // anonymous principal should fail
    assert!(res.is_err());
    let res =
        update::<Result<u64, String>>(&pic, src_principal, Some(user_principal), "refund", ());
    assert!(res.is_ok());
    // check that transfer has succeeded
    let balance_after = check_balance(&pic, user_principal, DEFAULT_SUBACCOUNT);
    assert_eq!(balance_after - balance_before, refundable_icp - DEFAULT_FEE);

    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, src_principal, None, "list_rental_requests", ());
    assert!(rental_requests.is_empty());
}

#[test]
fn test_duplicate_request_fails() {
    let (pic, src_principal) = setup();

    let user_principal = USER_1;

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().duration_since(UNIX_EPOCH).unwrap().as_secs();
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);

    // user performs preparations
    make_initial_transfer(&pic, src_principal, user_principal);

    // user creates proposal
    let now = now * 1_000_000_000;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time: now,
    };
    // run proposal
    update::<Result<(), ExecuteProposalError>>(
        &pic,
        src_principal,
        Some(Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap()),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();

    // it must only work the first time for the same user
    let res = update::<Result<(), ExecuteProposalError>>(
        &pic,
        src_principal,
        Some(Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap()),
        "execute_rental_request_proposal",
        payload,
    );
    assert!(res.unwrap_err() == ExecuteProposalError::UserAlreadyRequestingSubnetRental);
}

// #[test]
// fn test_proposal_rejected_if_already_rented() {
//     let (pic, canister_id) = setup();

//     let _block_index_approve = icrc2_approve(&pic, USER_1, 5_000 * E8S);

//     // The first time must succeed.
//     let wasm_res = accept_test_rental_agreement(&pic, &USER_1, &canister_id, SUBNET_FOR_RENT);
//     let WasmResult::Reply(res) = wasm_res else {
//         panic!("Expected a reply");
//     };
//     let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
//     assert!(res.is_ok());

//     // Using the same subnet again must fail.
//     let wasm_res = accept_test_rental_agreement(&pic, &USER_1, &canister_id, SUBNET_FOR_RENT);
//     let WasmResult::Reply(res) = wasm_res else {
//         panic!("Expected a reply");
//     };

//     let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
//     assert!(matches!(
//         res,
//         Err(ExecuteProposalError::SubnetAlreadyRented)
//     ));
// }

// #[test]
// fn test_proposal_rejected_if_too_low_funds() {
//     let (pic, canister_id) = setup();

//     let _block_index_approve = icrc2_approve(&pic, USER_2, 5_000 * E8S);

//     // User 2 has too low funds.
//     let wasm_res = accept_test_rental_agreement(&pic, &USER_2, &canister_id, SUBNET_FOR_RENT);
//     let WasmResult::Reply(res) = wasm_res else {
//         panic!("Expected a reply");
//     };
//     let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
//     assert!(matches!(
//         res,
//         Err(ExecuteProposalError::TransferUserToSrcError(
//             TransferFromError::InsufficientFunds { .. }
//         ))
//     ));
// }

// #[test]
// fn test_burning() {
//     let (pic, canister_id) = setup();
//     let _block_index_approve = icrc2_approve(&pic, USER_1, 5_000 * E8S);
//     accept_test_rental_agreement(&pic, &USER_1, &canister_id, SUBNET_FOR_RENT);

//     let billing_records: Vec<(Principal, BillingRecord)> =
//         query(&pic, canister_id, "list_billing_records", ());
//     let initial_balance = billing_records[0].1.cycles_balance;
//     pic.advance_time(Duration::from_secs(2));
//     pic.tick();
//     let billing_records: Vec<(Principal, BillingRecord)> =
//         query(&pic, canister_id, "list_billing_records", ());
//     let balance_1 = billing_records[0].1.cycles_balance;
//     assert!(balance_1 < initial_balance);

//     pic.advance_time(Duration::from_secs(4));
//     pic.tick();
//     let billing_records: Vec<(Principal, BillingRecord)> =
//         query(&pic, canister_id, "list_billing_records", ());
//     let balance_2 = billing_records[0].1.cycles_balance;
//     assert!(balance_2 < initial_balance);
//     assert!(balance_2 < balance_1);
// }

#[test]
fn test_accept_rental_agreement_cannot_be_called_by_non_governance() {
    let (pic, src_principal) = setup();
    let user_principal = USER_1;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time: 999,
    };
    let res = update::<Result<(), ExecuteProposalError>>(
        &pic,
        src_principal,
        None,
        "execute_rental_request_proposal",
        payload,
    );
    assert_eq!(res.unwrap_err(), ExecuteProposalError::UnauthorizedCaller);
}

// fn accept_test_rental_agreement(
//     pic: &PocketIc,
//     user: &Principal,
//     canister_id: &Principal,
//     subnet_id_str: &str,
// ) -> WasmResult {
//     let subnet_id = Principal::from_text(subnet_id_str).unwrap();
//     let arg = SubnetRentalProposalPayload {
//         subnet_id,
//         user: *user,
//         principals: vec![*user],
//         proposal_creation_time: 0,
//     };

//     pic.update_call(
//         *canister_id,
//         MAINNET_GOVERNANCE_CANISTER_ID,
//         "accept_rental_agreement",
//         encode_one(arg).unwrap(),
//     )
//     .unwrap()
// }

fn query<T: for<'a> Deserialize<'a> + candid::CandidType>(
    pic: &PocketIc,
    canister_id: Principal,
    sender: Option<Principal>,
    method: &str,
    args: impl CandidType,
) -> T {
    let res = pic
        .query_call(
            canister_id,
            sender.unwrap_or(Principal::anonymous()),
            method,
            encode_one(args).unwrap(),
        )
        .unwrap();
    let res = match res {
        WasmResult::Reply(res) => res,
        WasmResult::Reject(message) => panic!("Query expected Reply, got Reject: \n{}", message),
    };
    decode_one::<T>(&res).unwrap()
}

fn update<T: CandidType + for<'a> Deserialize<'a>>(
    pic: &PocketIc,
    canister_id: Principal,
    sender: Option<Principal>,
    method: &str,
    args: impl CandidType,
) -> T {
    let res = pic
        .update_call(
            canister_id,
            sender.unwrap_or(Principal::anonymous()),
            method,
            encode_one(args).unwrap(),
        )
        .unwrap();
    let res = match res {
        WasmResult::Reply(res) => res,
        WasmResult::Reject(message) => panic!("Update expected Reply, got Reject: \n{}", message),
    };
    decode_one::<T>(&res).unwrap()
}

fn check_balance(pic: &PocketIc, owner: Principal, subaccount: Subaccount) -> Tokens {
    query(
        pic,
        MAINNET_LEDGER_CANISTER_ID,
        Some(owner),
        "account_balance",
        AccountBalanceArgs {
            account: AccountIdentifier::new(&owner, &subaccount),
        },
    )
}
