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
        exchange_rate_canister::EXCHANGE_RATE_CANISTER_ID,
        governance_canister::GOVERNANCE_CANISTER_ID,
    },
    external_types::{
        CmcInitPayload, FeatureFlags, NnsLedgerCanisterInitPayload, NnsLedgerCanisterPayload,
    },
    EventPage, ExecuteProposalError, RentalConditionId, RentalConditions, RentalRequest,
    SubnetRentalProposalPayload, E8S, TRILLION,
};

const SRC_WASM: &str = "../../subnet_rental_canister.wasm.gz";
const LEDGER_WASM: &str = "./tests/ledger-canister.wasm.gz";
const CMC_WASM: &str = "./tests/cycles-minting-canister.wasm.gz";
const XRC_WASM: &str = "./tests/exchange-rate-canister.wasm";
const SRC_ID: Principal = Principal::from_slice(b"\x00\x00\x00\x00\x00\x00\x00\x0D\x01\x01"); // qvhpv-4qaaa-aaaaa-aaagq-cai
const NANOS_PER_SECOND: u64 = 1_000_000_000;
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
    pic.create_canister_with_id(None, None, EXCHANGE_RATE_CANISTER_ID)
        .unwrap();
    let xrc_wasm =
        fs::read(XRC_WASM).expect("Get the Wasm dependencies with ./scripts/get_wasms.sh");
    pic.install_canister(EXCHANGE_RATE_CANISTER_ID, xrc_wasm, vec![], None);
}

fn install_ledger(pic: &PocketIc) {
    pic.create_canister_with_id(None, None, MAINNET_LEDGER_CANISTER_ID)
        .unwrap();
    let icp_ledger_canister_wasm =
        fs::read(LEDGER_WASM).expect("Get the Wasm dependencies with ./scripts/get_wasms.sh");

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

fn setup() -> PocketIc {
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
    pic.add_cycles(subnet_rental_canister, 1_000 * TRILLION);
    pic
}

fn get_todays_price(pic: &PocketIc) -> Tokens {
    // user finds rental conditions
    let res = query::<Vec<(RentalConditionId, RentalConditions)>>(
        pic,
        SRC_ID,
        None,
        "list_rental_conditions",
        encode_one(()).unwrap(),
    );
    let (rental_condition_id, ref _rental_conditions) = res[0];
    // user finds current price by consulting SRC
    update::<Result<Tokens, String>>(pic, SRC_ID, None, "get_todays_price", rental_condition_id)
        .unwrap()
        .unwrap()
}

/// Transfers `amount` to the SRC subaccount of `user_principal`.
fn pay_src(pic: &PocketIc, user_principal: Principal, amount: Tokens) -> Tokens {
    // user finds the correct subaccount via SRC
    let account_hex = update::<String>(
        pic,
        SRC_ID,
        Some(user_principal),
        "get_payment_account",
        user_principal,
    )
    .unwrap();
    let target_account = AccountIdentifier::from_hex(&account_hex).unwrap();
    // user transfers some ICP to SRC
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
        EXCHANGE_RATE_CANISTER_ID,
        None,
        "set_exchange_rate_data",
        arg,
    )
    .unwrap();
}

#[test]
fn test_initial_proposal() {
    let pic = setup();

    let user_principal = USER_1;

    let initial_user_balance = check_balance(&pic, user_principal, DEFAULT_SUBACCOUNT);

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    set_mock_exchange_rate(&pic, now, 5_000_000_000, 9);
    let price1 = get_todays_price(&pic);

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    set_mock_exchange_rate(&pic, now, 10_000_000_000, 9);
    let price2 = get_todays_price(&pic);

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);
    let final_price = get_todays_price(&pic);

    // price should keep declining
    assert!(price1 > price2);
    assert!(price2 > final_price);

    let extra_amount = Tokens::from_e8s(200 * E8S); // Users might send more than is actually needed
    let total_amount_sent_to_src = final_price + extra_amount;
    // transfer the initial payment plus some extra amount
    pay_src(&pic, user_principal, total_amount_sent_to_src);

    // user creates proposal
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: now,
    };

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    set_mock_exchange_rate(&pic, now, 10_000_000_000, 9);
    let price4 = get_todays_price(&pic);

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    set_mock_exchange_rate(&pic, now, 5_000_000_000, 9);
    let price5 = get_todays_price(&pic);

    // price should keep increasing
    assert!(final_price < price4);
    assert!(price4 < price5);

    // the proposal has only been voted two days after its creation
    update::<()>(
        &pic,
        SRC_ID,
        Some(GOVERNANCE_CANISTER_ID),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();

    // assert state is as expected
    let src_history = query_multi_arg::<EventPage>(
        &pic,
        SRC_ID,
        None,
        "get_rental_conditions_history_page",
        (None::<Option<u64>>,),
    );
    let user_history = query_multi_arg::<EventPage>(
        &pic,
        SRC_ID,
        None,
        "get_history_page",
        (user_principal, None::<Option<u64>>),
    );
    // think of a better test than length
    assert_eq!(src_history.events.len(), 1);
    assert_eq!(user_history.events.len(), 1);

    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ());
    assert_eq!(rental_requests.len(), 1);
    assert_eq!(rental_requests[0].user, user_principal);
    assert_eq!(
        rental_requests[0].rental_condition_id,
        RentalConditionId::App13CH
    );
    assert_eq!(rental_requests[0].initial_cost_icp, final_price);

    // get refund as anonymous principal (should fail)
    let res = update::<Result<u64, String>>(&pic, SRC_ID, None, "refund", ());
    assert!(res.unwrap().is_err());

    // transfer some more funds to the SRC subaccount
    let additional_src_payment = Tokens::from_e8s(100 * E8S);
    pay_src(&pic, user_principal, additional_src_payment);

    // get refund on behalf of the actual renter
    let balance_before_refund = check_balance(&pic, user_principal, DEFAULT_SUBACCOUNT);
    let res = update::<Result<u64, String>>(&pic, SRC_ID, Some(user_principal), "refund", ());
    assert!(res.unwrap().is_ok());

    let immediately_locked_amount = Tokens::from_e8s(final_price.e8s() / 10); // 10% of ICP are locked immediately

    // check that transfer has succeeded and has correct amount
    let refundable_icp =
        total_amount_sent_to_src + additional_src_payment - immediately_locked_amount - DEFAULT_FEE; // withdraw cost
    let balance_after_refund = check_balance(&pic, user_principal, DEFAULT_SUBACCOUNT);
    assert_eq!(balance_after_refund, balance_before_refund + refundable_icp);

    // check total costs
    assert_eq!(
        initial_user_balance
        - immediately_locked_amount
        - DEFAULT_FEE // initial transfer to SRC
        - DEFAULT_FEE // additional transfer to SRC
        - DEFAULT_FEE, // withdraw cost
        balance_after_refund
    );

    // check that no funds are left on SRC subaccount
    let src_balance = check_balance(&pic, SRC_ID, Subaccount::from(user_principal));
    assert_eq!(src_balance, Tokens::from_e8s(0));

    // afterwards there should be no more rental requests remaining
    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ());
    assert!(rental_requests.is_empty());
}

#[test]
fn test_failed_initial_proposal() {
    let pic = setup();

    let user_principal = USER_1;

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);

    // transfer a bit too little to trigger a failure
    let initial_payment = get_todays_price(&pic);
    pay_src(&pic, user_principal, initial_payment - DEFAULT_FEE);

    // check user history before running proposal
    let user_history = query_multi_arg::<EventPage>(
        &pic,
        SRC_ID,
        None,
        "get_history_page",
        (user_principal, None::<Option<u64>>),
    );
    assert_eq!(user_history.events.len(), 0);

    // user creates proposal
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: now,
    };
    // run proposal
    update::<()>(
        &pic,
        SRC_ID,
        Some(GOVERNANCE_CANISTER_ID),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap_err();

    // the history is updated
    let user_history = query_multi_arg::<EventPage>(
        &pic,
        SRC_ID,
        None,
        "get_history_page",
        (user_principal, None::<Option<u64>>),
    );
    assert_eq!(user_history.events.len(), 1);

    // check that there are no rental requests
    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ());
    assert!(rental_requests.is_empty());
}

#[test]
fn test_duplicate_request_fails() {
    let pic = setup();

    let user_principal = USER_1;

    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);

    // user performs preparations
    let initial_payment = get_todays_price(&pic);
    pay_src(&pic, user_principal, initial_payment);

    // user creates proposal
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: now,
    };
    // run proposal
    update::<()>(
        &pic,
        SRC_ID,
        Some(GOVERNANCE_CANISTER_ID),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();

    // it must only work the first time for the same user
    let res = update::<()>(
        &pic,
        SRC_ID,
        Some(GOVERNANCE_CANISTER_ID),
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
    let pic = setup();
    let user_principal = USER_1;
    // set an exchange rate for the current time on the XRC mock
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    set_mock_exchange_rate(&pic, now, 12_503_823_284, 9);

    // transfer the initial payment
    let initial_payment = get_todays_price(&pic);
    let payment_on_top = Tokens::from_e8s(100 * E8S); // Users might send more than is actually needed
    pay_src(&pic, user_principal, initial_payment + payment_on_top);

    // user creates proposal
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: now,
    };

    // run proposal
    update::<()>(
        &pic,
        SRC_ID,
        Some(GOVERNANCE_CANISTER_ID),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();

    // check that 10% of the initial amount has been locked
    let rental_request =
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ())
            .pop()
            .unwrap();

    let should_lock_amount_icp = Tokens::from_e8s(initial_payment.e8s() / 10);

    assert_eq!(rental_request.locked_amount_icp, should_lock_amount_icp);

    // let some time pass. nothing should happen
    pic.advance_time(Duration::from_secs(60 * 60 * 24 * 29));
    for _ in 0..3 {
        pic.tick();
    }
    let updated_rental_request =
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ())
            .pop()
            .unwrap();
    assert_eq!(rental_request, updated_rental_request);
    // let some time pass. this time, we want a locking event to occur
    pic.advance_time(Duration::from_secs(60 * 60 * 24 * 2));
    for _ in 0..3 {
        pic.tick();
    }
    let updated_rental_request =
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ())
            .pop()
            .unwrap();
    assert_eq!(
        updated_rental_request.locked_amount_icp,
        should_lock_amount_icp + should_lock_amount_icp
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
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ())
            .pop()
            .unwrap();

    // in total, we might have an error of strictly less than 10 e8s
    assert!(updated_rental_request.locked_amount_icp >= initial_payment - Tokens::from_e8s(10));
    // there should be 1 rental request created event + 9 locking events (the last call should not have caused an event)
    let user_history = query_multi_arg::<EventPage>(
        &pic,
        SRC_ID,
        None,
        "get_history_page",
        (user_principal, None::<Option<u64>>),
    );
    assert_eq!(user_history.events.len(), 10);
}

#[test]
fn test_accept_rental_agreement_cannot_be_called_by_non_governance() {
    let pic = setup();
    let user_principal = USER_1;
    let payload = SubnetRentalProposalPayload {
        user: user_principal,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 999,
        proposal_creation_time_seconds: 999,
    };
    let res = update::<()>(
        &pic,
        SRC_ID,
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
