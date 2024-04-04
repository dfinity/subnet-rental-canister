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
        governance_canister::{self, GOVERNANCE_CANISTER_PRINCIPAL_STR},
    },
    external_types::{
        CmcInitPayload, FeatureFlags, NnsLedgerCanisterInitPayload, NnsLedgerCanisterPayload,
    },
    ExecuteProposalError, RentalConditionId, RentalConditions, SubnetRentalProposalPayload, E8S,
};

const SRC_WASM: &str = "../../subnet_rental_canister.wasm";
const LEDGER_WASM: &str = "./tests/ledger-canister.wasm.gz";
const CMC_WASM: &str = "./tests/cycles-minting-canister.wasm.gz";
const XRC_WASM: &str = "./tests/exchange-rate-canister.wasm.gz";
const SRC_ID: Principal =
    Principal::from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xE0, 0x00, 0x00, 0x01, 0x01]); // lxzze-o7777-77777-aaaaa-cai
const _SUBNET_FOR_RENT: &str = "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae";
const USER_1: Principal = Principal::from_slice(b"user1");
const USER_1_INITIAL_BALANCE: Tokens = Tokens::from_e8s(1_000_000 * E8S);
const USER_2: Principal = Principal::from_slice(b"user2");
const USER_2_INITIAL_BALANCE: Tokens = Tokens::from_e8s(DEFAULT_FEE.e8s() * 2);

fn install_cmc(pic: &PocketIc) {
    pic.create_canister_with_id(None, None, MAINNET_CYCLES_MINTING_CANISTER_ID)
        .unwrap();
    let cmc_wasm = fs::read(CMC_WASM).expect("Could not find the patched CMC wasm");
    let minter = AccountIdentifier::new(&MAINNET_CYCLES_MINTING_CANISTER_ID, &DEFAULT_SUBACCOUNT);
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

#[test]
fn dummy() {
    setup();
}

#[test]
fn test_initial_proposal() {
    let (pic, src_principal) = setup();

    let user_principal = USER_1;

    let res = query::<Vec<(RentalConditionId, RentalConditions)>>(
        &pic,
        SRC_ID,
        None,
        "list_rental_conditions",
        encode_one(()).unwrap(),
    );
    let (
        _,
        RentalConditions {
            description: _,
            subnet_id: _,
            daily_cost_cycles,
            initial_rental_period_days,
            billing_period_days: _,
        },
    ) = res[0];
    // same calculation as in SRC; assuming an exchange rate
    let needed_cycles = daily_cost_cycles.saturating_mul(initial_rental_period_days as u128);
    let exchange_rate_icp_per_xdr = 12.6;
    let e8s = (needed_cycles as f64 * exchange_rate_icp_per_xdr) as u64 / 10_000;
    let needed_icp = Tokens::from_e8s(e8s);

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
        &pic,
        MAINNET_LEDGER_CANISTER_ID,
        Some(user_principal),
        "transfer",
        transfer_args,
    )
    .unwrap();

    // create proposal
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_type: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time: now,
    };
    let _res = update::<Result<(), ExecuteProposalError>>(
        &pic,
        src_principal,
        Some(Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap()),
        "execute_rental_request_proposal",
        payload,
    )
    .unwrap();
}

// #[test]
// fn _test_authorization() {
//     // This test is incomplete because with PocketIC, we cannot create negative whitelist tests.
//     let pic = PocketIcBuilder::new()
//         .with_nns_subnet()
//         .with_application_subnet()
//         .with_application_subnet()
//         .build();
//     let _subnet_nns = pic.topology().get_nns().unwrap();
//     let subnet_1 = pic.topology().get_app_subnets()[0];
//     let _subnet_2 = pic.topology().get_app_subnets()[1];

//     install_cmc(&pic);
//     let user1 = Principal::from_slice(b"user1");
//     let _user2 = Principal::from_slice(b"user2");

//     #[derive(candid::CandidType)]
//     struct Arg {
//         pub who: Option<candid::Principal>,
//         pub subnets: Vec<candid::Principal>,
//     }
//     let arg = Arg {
//         who: Some(user1),
//         subnets: vec![subnet_1],
//     };

//     pic.update_call(
//         MAINNET_CYCLES_MINTING_CANISTER_ID,
//         MAINNET_GOVERNANCE_CANISTER_ID,
//         "set_authorized_subnetwork_list",
//         encode_one(arg).unwrap(),
//     )
//     .unwrap();
// }

// #[test]
// fn test_list_rental_conditions() {
//     let (pic, canister_id) = setup();

//     let WasmResult::Reply(res) = pic
//         .query_call(
//             canister_id,
//             Principal::anonymous(),
//             "list_rental_conditions",
//             encode_one(()).unwrap(),
//         )
//         .unwrap()
//     else {
//         panic!("Expected a reply")
//     };

//     let conditions = decode_one::<Vec<(Principal, RentalConditions)>>(&res).unwrap();
//     assert!(!conditions.is_empty());
// }

// #[test]
// fn test_proposal_accept() {
//     let (pic, canister_id) = setup();

//     let time_now = pic
//         .get_time()
//         .duration_since(UNIX_EPOCH)
//         .unwrap()
//         .as_nanos();

//     // User approves a sufficient amount of ICP.
//     let _block_index_approve = icrc2_approve(&pic, USER_1, 5_000 * E8S);

//     // Proposal is accepted and the governance canister calls accept_rental_agreement.
//     let wasm_res = accept_test_rental_agreement(&pic, &USER_1, &canister_id, SUBNET_FOR_RENT);
//     let WasmResult::Reply(res) = wasm_res else {
//         panic!("Expected a reply");
//     };
//     let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();

//     // The proposal is executed successfully.
//     assert!(res.is_ok());

//     // The user's balance is reduced by the rental fee and one transaction fee.
//     let user_balance = check_balance(&pic, USER_1, DEFAULT_SUBACCOUNT);
//     let historical_exchange_rate_cycles_per_e8s = 1_000_000; // this is what the CMC returns atm
//     assert_eq!(
//         user_balance,
//         USER_1_INITIAL_BALANCE
//             - DEFAULT_FEE // icrc-2 approval fee
//             - Tokens::from_e8s(historical_exchange_rate_cycles_per_e8s * 183 * 2_000) // 2_000 XDR for 183 days
//     );

//     // The rental account is stored in the canister state.
//     let billing_records: Vec<(Principal, BillingRecord)> =
//         query(&pic, canister_id, "list_billing_records", ());

//     assert_eq!(billing_records.len(), 1);
//     assert_eq!(billing_records[0].0.to_string(), SUBNET_FOR_RENT);
//     assert_eq!(
//         billing_records[0].1.cycles_balance,
//         2_000 * TRILLION * 183
//             - (2 * DEFAULT_FEE.e8s() as u128 * historical_exchange_rate_cycles_per_e8s as u128) // two transaction fees
//     );
//     assert_eq!(
//         billing_records[0].1.covered_until,
//         (time_now + 183 * 24 * 60 * 60 * 1_000_000_000) as u64
//     );

//     // The rental agreement is stored in the canister state.
//     let rental_agreements: Vec<RentalAgreement> =
//         query(&pic, canister_id, "list_rental_agreements", ());

//     assert_eq!(rental_agreements.len(), 1);
//     assert_eq!(rental_agreements[0].user.0, USER_1);
//     assert_eq!(
//         rental_agreements[0].subnet_id.0.to_string(),
//         SUBNET_FOR_RENT
//     );
//     assert!(rental_agreements[0]
//         .principals
//         .iter()
//         .map(|p| p.0.to_string())
//         .collect_vec()
//         .contains(&USER_1.to_string()));
// }

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
// fn test_proposal_rejected_if_icrc2_approval_too_low() {
//     let (pic, canister_id) = setup();

//     let _block_index_approve = icrc2_approve(&pic, USER_1, 2 * E8S);

//     // User 1 has approved too little funds.
//     let wasm_res = accept_test_rental_agreement(&pic, &USER_1, &canister_id, SUBNET_FOR_RENT);
//     let WasmResult::Reply(res) = wasm_res else {
//         panic!("Expected a reply");
//     };
//     let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
//     assert!(matches!(
//         res,
//         Err(ExecuteProposalError::TransferUserToSrcError(
//             TransferFromError::InsufficientAllowance { .. }
//         ))
//     ));
// }

// #[test]
// fn test_history() {
//     let (pic, canister_id) = setup();
//     let _wasm_res = accept_test_rental_agreement(&pic, &USER_1, &canister_id, SUBNET_FOR_RENT);
//     let subnet = Principal::from_text(SUBNET_FOR_RENT).unwrap();

//     let events: Option<Vec<Event>> = query(&pic, canister_id, "get_history", subnet);
//     assert!(events.is_some());
//     assert_eq!(events.unwrap().len(), 2);
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

// #[test]
// fn test_accept_rental_agreement_cannot_be_called_by_non_governance() {
//     let (pic, canister_id) = setup();

//     let arg = SubnetRentalProposalPayload {
//         subnet_id: Principal::from_text(SUBNET_FOR_RENT).unwrap(),
//         user: USER_1,
//         principals: vec![USER_1],
//         proposal_creation_time: 0,
//     };

//     let WasmResult::Reply(res) = pic
//         .update_call(
//             canister_id,
//             Principal::anonymous(),
//             "accept_rental_agreement",
//             encode_one(arg.clone()).unwrap(),
//         )
//         .unwrap()
//     else {
//         panic!("Expected a reply");
//     };
//     let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
//     assert!(matches!(res, Err(ExecuteProposalError::UnauthorizedCaller)));
// }

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
