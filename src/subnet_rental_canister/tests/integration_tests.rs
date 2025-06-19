use candid::{decode_one, encode_args, encode_one, utils::ArgumentEncoder, CandidType, Principal};
use ic_ledger_types::{
    AccountBalanceArgs, AccountIdentifier, Memo, Subaccount, Tokens, TransferArgs, TransferResult,
    DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_CYCLES_MINTING_CANISTER_ID,
    MAINNET_GOVERNANCE_CANISTER_ID, MAINNET_LEDGER_CANISTER_ID,
};
use pocket_ic::{PocketIc, PocketIcBuilder};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    time::Duration,
};
use subnet_rental_canister::{
    external_canister_interfaces::{
        exchange_rate_canister::EXCHANGE_RATE_CANISTER_PRINCIPAL_STR,
        governance_canister::GOVERNANCE_CANISTER_PRINCIPAL_STR,
    },
    external_types::{
        CmcInitPayload, FeatureFlags, NnsLedgerCanisterInitPayload, NnsLedgerCanisterPayload,
    },
    EventPage, ExecuteProposalError, RentalConditionId, RentalConditions, RentalRequest,
    SubnetRentalProposalPayload, E8S,
};

const SRC_WASM: &str = "../../subnet_rental_canister.wasm";
const LEDGER_WASM: &str = "./tests/ledger-canister.wasm.gz";
const CMC_WASM: &str = "./tests/cycles-minting-canister.wasm.gz";
const XRC_WASM: &str = "./tests/exchange-rate-canister.wasm.gz";
const SRC_ID: Principal = Principal::from_slice(b"\xFF\xFF\xFF\xFF\xFF\xE0\x00\x00\x01\x01"); // lxzze-o7777-77777-aaaaa-cai
const NANOS_IN_SECOND: u64 = 1_000_000_000;
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

fn get_todays_price(pic: &PocketIc, src_principal: Principal) -> Tokens {
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
    update::<Result<Tokens, String>>(
        pic,
        src_principal,
        None,
        "get_todays_price",
        rental_condition_id,
    )
    .unwrap()
    .unwrap()
}

// transfers a fraction of the initial payment (`fraction = 1` to transfer the full initial payment)
fn make_initial_transfer(
    pic: &PocketIc,
    src_principal: Principal,
    user_principal: Principal,
    fraction: u64,
) -> Tokens {
    let needed_icp = get_todays_price(pic, src_principal);
    // user finds the correct subaccount via SRC
    let account_hex = update::<String>(
        pic,
        src_principal,
        Some(user_principal),
        "get_payment_account",
        user_principal,
    )
    .unwrap();
    let target_account = AccountIdentifier::from_hex(&account_hex).unwrap();
    // user transfers some ICP to SRC
    let amount = Tokens::from_e8s(needed_icp.e8s() / fraction);
    let transfer_args = TransferArgs {
        memo: Memo(0),
        amount,
        fee: DEFAULT_FEE,
        from_subaccount: None,
        to: target_account,
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
    amount
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
    )
    .unwrap();
}

#[test]
fn test_initial_proposal() {
    let (pic, src_principal) = setup();

    let user_principal = USER_1;

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    set_mock_exchange_rate(&pic, now, 5_000_000_000, 9);
    let price1 = get_todays_price(&pic, src_principal);

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    set_mock_exchange_rate(&pic, now, 10_000_000_000, 9);
    let price2 = get_todays_price(&pic, src_principal);

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);
    let price3 = get_todays_price(&pic, src_principal);

    // price should keep declining
    assert!(price1 > price2);
    assert!(price2 > price3);

    // transfer the initial payment
    make_initial_transfer(&pic, src_principal, user_principal, 1);

    // user creates proposal
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: now,
    };

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    set_mock_exchange_rate(&pic, now, 10_000_000_000, 9);
    let price4 = get_todays_price(&pic, src_principal);

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    set_mock_exchange_rate(&pic, now, 5_000_000_000, 9);
    let price5 = get_todays_price(&pic, src_principal);

    // price should keep increasing
    assert!(price3 < price4);
    assert!(price4 < price5);

    // the proposal has only been voted two days after its creation
    update::<()>(
        &pic,
        src_principal,
        Some(Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap()),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();

    // assert state is as expected
    let src_history = query_multi_arg::<EventPage>(
        &pic,
        src_principal,
        None,
        "get_rental_conditions_history_page",
        (None::<Option<u64>>,),
    );
    let user_history = query_multi_arg::<EventPage>(
        &pic,
        src_principal,
        None,
        "get_history_page",
        (user_principal, None::<Option<u64>>),
    );
    // think of a better test than length
    assert_eq!(src_history.events.len(), 1);
    assert_eq!(user_history.events.len(), 1);

    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, src_principal, None, "list_rental_requests", ());
    assert_eq!(rental_requests.len(), 1);
    let RentalRequest {
        user,
        initial_cost_icp: _,
        refundable_icp,
        locked_amount_icp: _,
        locked_amount_cycles: _,
        initial_proposal_id: _,
        creation_time_nanos: _,
        rental_condition_id,
        last_locking_time_nanos: _,
    } = rental_requests[0];
    assert_eq!(user, user_principal);
    assert_eq!(rental_condition_id, RentalConditionId::App13CH);

    // get refund as anonymous principal (should fail)
    let balance_before = check_balance(&pic, user_principal, DEFAULT_SUBACCOUNT);
    let res = update::<Result<u64, String>>(&pic, src_principal, None, "refund", ());
    assert!(res.unwrap().is_err());

    // get refund on behalf of the actual renter
    let res =
        update::<Result<u64, String>>(&pic, src_principal, Some(user_principal), "refund", ());
    assert!(res.unwrap().is_ok());

    // check that transfer has succeeded
    let balance_after = check_balance(&pic, user_principal, DEFAULT_SUBACCOUNT);
    assert_eq!(balance_after - balance_before, refundable_icp - DEFAULT_FEE);

    // check that no funds are left on SRC subaccount
    let src_balance = check_balance(&pic, src_principal, Subaccount::from(user_principal));
    assert_eq!(src_balance, Tokens::from_e8s(0));

    // afterwards there should be no more rental requests remaining
    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, src_principal, None, "list_rental_requests", ());
    assert!(rental_requests.is_empty());
}

#[test]
fn test_failed_initial_proposal() {
    let (pic, src_principal) = setup();

    let user_principal = USER_1;

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);

    // transfer only half of the initial payment
    make_initial_transfer(&pic, src_principal, user_principal, 2);

    // check user history before running proposal
    let user_history = query_multi_arg::<EventPage>(
        &pic,
        src_principal,
        None,
        "get_history_page",
        (user_principal, None::<Option<u64>>),
    );
    assert_eq!(user_history.events.len(), 0);

    // user creates proposal
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: now,
    };
    // run proposal
    update::<()>(
        &pic,
        src_principal,
        Some(Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap()),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap_err();

    // the history is updated
    let user_history = query_multi_arg::<EventPage>(
        &pic,
        src_principal,
        None,
        "get_history_page",
        (user_principal, None::<Option<u64>>),
    );
    assert_eq!(user_history.events.len(), 1);

    // check that there are no rental requests
    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, src_principal, None, "list_rental_requests", ());
    assert!(rental_requests.is_empty());
}

#[test]
fn test_duplicate_request_fails() {
    let (pic, src_principal) = setup();

    let user_principal = USER_1;

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);

    // user performs preparations
    make_initial_transfer(&pic, src_principal, user_principal, 1);

    // user creates proposal
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: now,
    };
    // run proposal
    update::<()>(
        &pic,
        src_principal,
        Some(Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap()),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();

    // it must only work the first time for the same user
    let res = update::<()>(
        &pic,
        src_principal,
        Some(Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap()),
        "execute_rental_request_proposal",
        payload,
    );
    assert!(res.unwrap_err().contains(&format!(
        "{:?}",
        ExecuteProposalError::UserAlreadyRequestingSubnetRental
    )));
}

#[test]
fn test_locking() {
    let (pic, src_principal) = setup();
    let user_principal = USER_1;
    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);
    // transfer the initial payment
    let initial_amount_icp = make_initial_transfer(&pic, src_principal, user_principal, 1);
    // user creates proposal
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_IN_SECOND;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: now,
    };
    // run proposal
    update::<()>(
        &pic,
        src_principal,
        Some(Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap()),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();
    // check that 10% of the initial amount has been locked
    let rental_request =
        query::<Vec<RentalRequest>>(&pic, src_principal, None, "list_rental_requests", ())
            .pop()
            .unwrap();
    let lock_amount_icp = Tokens::from_e8s(initial_amount_icp.e8s() / 10);
    assert_eq!(rental_request.locked_amount_icp, lock_amount_icp);
    // this one might fail due to rounding errors
    assert_eq!(
        rental_request.refundable_icp,
        initial_amount_icp - lock_amount_icp
    );
    // let some time pass. nothing should happen
    pic.advance_time(Duration::from_secs(60 * 60 * 24 * 29));
    for _ in 0..3 {
        pic.tick();
    }
    let updated_rental_request =
        query::<Vec<RentalRequest>>(&pic, src_principal, None, "list_rental_requests", ())
            .pop()
            .unwrap();
    assert_eq!(rental_request, updated_rental_request);
    // let some time pass. this time, we want a locking event to occur
    pic.advance_time(Duration::from_secs(60 * 60 * 24 * 2));
    for _ in 0..3 {
        pic.tick();
    }
    let updated_rental_request =
        query::<Vec<RentalRequest>>(&pic, src_principal, None, "list_rental_requests", ())
            .pop()
            .unwrap();
    assert_eq!(
        updated_rental_request.locked_amount_icp,
        lock_amount_icp + lock_amount_icp
    );
    assert_eq!(
        updated_rental_request.refundable_icp,
        initial_amount_icp - lock_amount_icp - lock_amount_icp
    );
    assert!(
        rental_request.last_locking_time_nanos < updated_rental_request.last_locking_time_nanos
    );
    // repeat 9 times (once more than necessary to lock everything; should silently succeed)
    for _ in 0..9 {
        pic.advance_time(Duration::from_secs(60 * 60 * 24 * 30 + 1));
        for _ in 0..3 {
            pic.tick();
        }
    }
    let updated_rental_request =
        query::<Vec<RentalRequest>>(&pic, src_principal, None, "list_rental_requests", ())
            .pop()
            .unwrap();
    // in total, we might have an error of strictly less than 10 e8s
    assert!(updated_rental_request.locked_amount_icp >= initial_amount_icp - Tokens::from_e8s(10));
    assert!(updated_rental_request.refundable_icp < Tokens::from_e8s(10));
    // there should be 1 rental request created event + 9 locking events (the last call should not have caused an event)
    let user_history = query_multi_arg::<EventPage>(
        &pic,
        src_principal,
        None,
        "get_history_page",
        (user_principal, None::<Option<u64>>),
    );
    assert_eq!(user_history.events.len(), 10);
}

#[test]
fn test_accept_rental_agreement_cannot_be_called_by_non_governance() {
    let (pic, src_principal) = setup();
    let user_principal = USER_1;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: 999,
    };
    let res = update::<()>(
        &pic,
        src_principal,
        None,
        "execute_rental_request_proposal",
        payload,
    );
    assert!(res
        .unwrap_err()
        .contains(&format!("{:?}", ExecuteProposalError::UnauthorizedCaller)));
}

// TODO
// fn test_proposal_rejected_if_already_rented() {
// fn test_burning() {
// fn accept_test_rental_agreement(

// ====================================================================================================================
// Helpers
fn query<T: for<'a> Deserialize<'a> + candid::CandidType>(
    pic: &PocketIc,
    canister_id: Principal,
    sender: Option<Principal>,
    method: &str,
    args: impl CandidType,
) -> T {
    match pic.query_call(
        canister_id,
        sender.unwrap_or(Principal::anonymous()),
        method,
        encode_one(args).unwrap(),
    ) {
        Ok(res) => decode_one::<T>(&res).unwrap(),
        Err(message) => panic!("Query expected Reply, got Reject: \n{}", message),
    }
}

fn query_multi_arg<T: for<'a> Deserialize<'a> + candid::CandidType>(
    pic: &PocketIc,
    canister_id: Principal,
    sender: Option<Principal>,
    method: &str,
    args: impl CandidType + ArgumentEncoder,
) -> T {
    match pic.query_call(
        canister_id,
        sender.unwrap_or(Principal::anonymous()),
        method,
        encode_args(args).unwrap(),
    ) {
        Ok(res) => decode_one::<T>(&res).unwrap(),
        Err(message) => panic!("Query expected Reply, got Reject: \n{}", message),
    }
}

fn update<T: CandidType + for<'a> Deserialize<'a>>(
    pic: &PocketIc,
    canister_id: Principal,
    sender: Option<Principal>,
    method: &str,
    args: impl CandidType,
) -> Result<T, String> {
    match pic.update_call(
        canister_id,
        sender.unwrap_or(Principal::anonymous()),
        method,
        encode_one(args).unwrap(),
    ) {
        Ok(res) => Ok(decode_one::<T>(&res).unwrap()),
        Err(message) => Err(message.to_string()),
    }
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
