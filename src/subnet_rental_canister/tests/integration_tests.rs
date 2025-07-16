use candid::{decode_one, encode_args, encode_one, utils::ArgumentEncoder, CandidType, Principal};
use ic_ledger_types::{
    AccountBalanceArgs, AccountIdentifier, Memo, Subaccount, Tokens, TransferArgs, TransferResult,
    DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_CYCLES_MINTING_CANISTER_ID,
    MAINNET_GOVERNANCE_CANISTER_ID, MAINNET_LEDGER_CANISTER_ID,
};
use pocket_ic::{PocketIc, PocketIcBuilder, Time};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    time::Duration,
};
use subnet_rental_canister::{
    external_calls::EXCHANGE_RATE_CANISTER_ID,
    external_types::{
        CmcInitPayload, ExchangeRateCanister, FeatureFlags, NnsLedgerCanisterInitPayload,
        NnsLedgerCanisterPayload, PrincipalsAuthorizedToCreateCanistersToSubnetsResponse,
    },
    CreateRentalAgreementPayload, EventPage, ExecuteProposalError, RentalAgreement,
    RentalAgreementStatus, RentalConditionId, RentalConditions, RentalRequest,
    SubnetRentalProposalPayload, TopupData, E8S, TRILLION,
};

const SRC_WASM: &str = "../../subnet_rental_canister.wasm.gz";
const LEDGER_WASM: &str = "./tests/ledger-canister.wasm.gz";
const CMC_WASM: &str = "./tests/cycles-minting-canister.wasm.gz";
const XRC_WASM: &str = "./tests/exchange-rate-canister.wasm";
const SRC_ID: Principal = Principal::from_slice(b"\x00\x00\x00\x00\x00\x00\x00\x0D\x01\x01"); // qvhpv-4qaaa-aaaaa-aaagq-cai
const NANOS_PER_SECOND: u64 = 1_000_000_000;
const SUBNET_FOR_RENT: Principal = Principal::from_slice(b"\xBA\x58\xB2\x11\x25\x38\x1B\x05\x67\xE6\x1F\x3F\x2E\xCD\x65\xF3\x77\x10\x31\x60\x84\xEE\x79\x1C\xDF\xDB\x4A\x1A\x02"); // fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae
const USER_1: Principal = Principal::from_slice(b"user1");
const USER_1_INITIAL_BALANCE: Tokens = Tokens::from_e8s(1_000_000_000 * E8S);
const USER_2: Principal = Principal::from_slice(b"user2");
const USER_2_INITIAL_BALANCE: Tokens = Tokens::from_e8s(DEFAULT_FEE.e8s() * 2);
const INITIAL_SRC_CYCLES_BALANCE: u128 = 100 * TRILLION;

fn install_xrc_and_cmc(pic: &PocketIc) {
    // install XRC
    pic.create_canister_with_id(None, None, EXCHANGE_RATE_CANISTER_ID)
        .unwrap();
    let xrc_wasm =
        fs::read(XRC_WASM).expect("Get the Wasm dependencies with ./scripts/get_wasms.sh");
    pic.install_canister(EXCHANGE_RATE_CANISTER_ID, xrc_wasm, vec![], None);

    // install CMC
    pic.create_canister_with_id(None, None, MAINNET_CYCLES_MINTING_CANISTER_ID)
        .unwrap();
    let cmc_wasm =
        fs::read(CMC_WASM).expect("Get the Wasm dependencies with ./scripts/get_wasms.sh");
    let minter = AccountIdentifier::new(&MAINNET_GOVERNANCE_CANISTER_ID, &DEFAULT_SUBACCOUNT);
    let init_arg = CmcInitPayload {
        governance_canister_id: Some(MAINNET_GOVERNANCE_CANISTER_ID),
        minting_account_id: minter.to_string(),
        ledger_canister_id: Some(MAINNET_LEDGER_CANISTER_ID),
        last_purged_notification: None,
        exchange_rate_canister: Some(ExchangeRateCanister::Set(EXCHANGE_RATE_CANISTER_ID)),
        cycles_ledger_canister_id: None,
    };
    pic.install_canister(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        cmc_wasm,
        encode_args((Some(init_arg),)).unwrap(),
        None,
    );
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

    pic.set_time(Time::from_nanos_since_unix_epoch(
        1_620_633_600 * 1_000_000_000,
    )); // set time to make CMC fetch the first rate properly from the XRC

    install_ledger(&pic);
    install_xrc_and_cmc(&pic);

    // Install subnet rental canister.
    let subnet_rental_canister = pic.create_canister_with_id(None, None, SRC_ID).unwrap();
    let src_wasm = fs::read(SRC_WASM).expect("Build the wasm with ./scripts/build.sh");
    pic.install_canister(subnet_rental_canister, src_wasm, vec![], None);
    pic.add_cycles(subnet_rental_canister, INITIAL_SRC_CYCLES_BALANCE);
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
    assert_eq!(res.len(), 1);
    let (rental_condition_id, _rental_conditions) = res.first().unwrap();
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

fn set_xrc_exchange_rate_last_midnight(pic: &PocketIc, exchange_rate_xdr_per_icp: u64) {
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    let midnight = now - now % 86400;
    update::<()>(
        pic,
        EXCHANGE_RATE_CANISTER_ID,
        None,
        "set_exchange_rate_data",
        vec![(midnight, exchange_rate_xdr_per_icp)],
    )
    .unwrap();
}

/// Sets the exchange rate for the CMC to the given value and advances time by 5 minutes.
/// NOTE: The CMC will only have a precision of e.g. 1 ICP = 3.4979 XDR (4 decimal places).
fn set_cmc_exchange_rate(pic: &PocketIc, exchange_rate_xdr_per_icp: u64) {
    // set initial exchange rate to XRC and make CMC fetch it
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    let fetch_time = now + 5 * 60;

    update::<()>(
        pic,
        EXCHANGE_RATE_CANISTER_ID,
        None,
        "set_exchange_rate_data",
        vec![(fetch_time, exchange_rate_xdr_per_icp)],
    )
    .unwrap();

    // advance time by 5 minutes
    pic.advance_time(Duration::from_secs(5 * 60));
    for _ in 0..2 {
        pic.tick();
    }
}

#[test]
fn test_initial_proposal() {
    let pic = setup();

    let user_principal = USER_1;

    let initial_user_balance = check_balance(&pic, user_principal, DEFAULT_SUBACCOUNT);

    // set an exchange rate for the current time on the XRC mock
    set_xrc_exchange_rate_last_midnight(&pic, 5_000_000_000); // 1 ICP = 5 XDR
    let price1 = get_todays_price(&pic);

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    set_xrc_exchange_rate_last_midnight(&pic, 10_000_000_000); // 1 ICP = 10 XDR
    let price2 = get_todays_price(&pic);

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    set_xrc_exchange_rate_last_midnight(&pic, 12_503_823_284); // 1 ICP = 12.503823284 XDR
    let final_subnet_price = get_todays_price(&pic);

    // price should keep declining
    assert!(price1 > price2);
    assert!(price2 > final_subnet_price);

    let extra_amount = Tokens::from_e8s(200 * E8S); // Users might send more than is actually needed
    let total_amount_sent_to_src = final_subnet_price + extra_amount;
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
    set_xrc_exchange_rate_last_midnight(&pic, 10_000_000_000); // 1 ICP = 10 XDR
    let price4 = get_todays_price(&pic);

    // advance time by one day
    pic.advance_time(Duration::from_secs(86400));

    // set an exchange rate for the current time on the XRC mock
    set_xrc_exchange_rate_last_midnight(&pic, 5_000_000_000); // 1 ICP = 5 XDR
    let price5 = get_todays_price(&pic);

    // price should keep increasing
    assert!(final_subnet_price < price4);
    assert!(price4 < price5);

    // the proposal has only been voted two days after its creation
    update::<()>(
        &pic,
        SRC_ID,
        Some(MAINNET_GOVERNANCE_CANISTER_ID),
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
    let rental_request = rental_requests.first().unwrap();
    assert_eq!(rental_request.user, user_principal);
    assert_eq!(
        rental_request.rental_condition_id,
        RentalConditionId::App13CH
    );
    assert_eq!(rental_request.initial_cost_icp, final_subnet_price);

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

    let immediately_locked_amount = Tokens::from_e8s(final_subnet_price.e8s() / 10); // 10% of ICP are locked immediately

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
fn test_create_rental_agreement() {
    let pic = setup();

    // set an exchange rate for the current time on the XRC mock
    set_xrc_exchange_rate_last_midnight(&pic, 3_593_382_591); // 1 ICP = 3.593382591 XDR
    let final_subnet_price = get_todays_price(&pic);

    // transfer the initial payment
    let paid_to_src = final_subnet_price + Tokens::from_e8s(100 * E8S); // 100 ICP extra
    pay_src(&pic, USER_1, paid_to_src);

    // create rental request proposal
    let now = pic.get_time().as_nanos_since_unix_epoch() / NANOS_PER_SECOND;
    let payload = SubnetRentalProposalPayload {
        user: USER_1,
        rental_condition_id: RentalConditionId::App13CH,
        proposal_id: 136408,
        proposal_creation_time_seconds: now,
    };

    // 1 day passes ...
    pic.advance_time(Duration::from_secs(86400));
    for _ in 0..3 {
        pic.tick();
    }

    // set a different current exchange rate the CMC
    let cmc_exchange_rate_xdr_per_billion_icp_at_proposal_execution = 3_497_900_000; // 1 ICP = 3.4979 XDR
    set_cmc_exchange_rate(
        &pic,
        cmc_exchange_rate_xdr_per_billion_icp_at_proposal_execution,
    );

    // run proposal
    update::<()>(
        &pic,
        SRC_ID,
        Some(MAINNET_GOVERNANCE_CANISTER_ID),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();

    // check that the rental request is created
    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ());
    assert_eq!(rental_requests.len(), 1);
    let rental_request = rental_requests.first().unwrap();
    assert_eq!(rental_request.user, USER_1);
    assert_eq!(
        rental_request.rental_condition_id,
        RentalConditionId::App13CH
    );
    assert_eq!(rental_request.initial_cost_icp, final_subnet_price);

    // check all balances are correct
    let locked_icp = rental_request.locked_amount_icp;
    let locked_cycles = rental_request.locked_amount_cycles;
    let initial_cost_icp = rental_request.initial_cost_icp;

    // 10% of the initial cost is converted to cycles, minus the fee
    let icp_to_be_locked = (final_subnet_price.e8s() as u128 / 10) - DEFAULT_FEE.e8s() as u128;
    let expected_locked_cycles_first_locking = (icp_to_be_locked
        * cmc_exchange_rate_xdr_per_billion_icp_at_proposal_execution as u128)
        / 100_000;

    assert_eq!(initial_cost_icp, final_subnet_price); // initial cost is the final price
    assert_eq!(locked_icp.e8s(), final_subnet_price.e8s() / 10); // 10% of the initial cost is locked
    assert_eq!(locked_cycles, expected_locked_cycles_first_locking);

    let src_cycles_balance_after_first_locking = pic.canister_status(SRC_ID, None).unwrap().cycles;

    assert_eq!(
        src_cycles_balance_after_first_locking,
        INITIAL_SRC_CYCLES_BALANCE + expected_locked_cycles_first_locking - 1_000_000_000 // call of get_todays_price()
    );

    // set a different exchange rate for the second locking event (ICP fell)
    let cmc_exchange_rate_at_second_locking = 3_197_900_000; // 1 ICP = 3.1979 XDR
    set_cmc_exchange_rate(&pic, cmc_exchange_rate_at_second_locking);

    // advance time by 31 days
    pic.advance_time(Duration::from_secs(31 * 86400));
    for _ in 0..3 {
        pic.tick();
    }

    let expected_locked_cycles_second_locking =
        (icp_to_be_locked * cmc_exchange_rate_at_second_locking as u128) / 100_000;

    let src_cycles_balance_after_second_locking = pic.canister_status(SRC_ID, None).unwrap().cycles;

    assert_eq!(
        src_cycles_balance_after_second_locking,
        INITIAL_SRC_CYCLES_BALANCE
            + expected_locked_cycles_first_locking
            + expected_locked_cycles_second_locking
            - 1_000_000_000 // call of get_todays_price()
    );

    // at this point, 2 lockings have occured, the initial one and a second one after a month.

    // execute create rental agreement
    // set a different exchange rate for the proposal execution time (ICP rose)
    let cmc_exchange_rate_execute_rental_agreement = 3_897_900_000; // 1 ICP = 3.8979 XDR
    set_cmc_exchange_rate(&pic, cmc_exchange_rate_execute_rental_agreement);

    // get rental request before executing the create rental agreement
    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ());
    assert_eq!(rental_requests.len(), 1);
    let rental_request = rental_requests.first().unwrap();

    let payload = CreateRentalAgreementPayload {
        user: USER_1,
        subnet_id: SUBNET_FOR_RENT,
        proposal_id: 137322,
    };
    update::<()>(
        &pic,
        SRC_ID,
        Some(MAINNET_GOVERNANCE_CANISTER_ID),
        "execute_create_rental_agreement",
        payload.clone(),
    )
    .unwrap();

    let now_nanos = pic.get_time().as_nanos_since_unix_epoch();

    // check whitelisting on CMC
    let cmc_whitelisted_subnets = query::<PrincipalsAuthorizedToCreateCanistersToSubnetsResponse>(
        &pic,
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        None,
        "get_principals_authorized_to_create_canisters_to_subnets",
        (),
    );
    let entries = cmc_whitelisted_subnets.data;
    assert_eq!(entries.len(), 1);
    let entry = entries.first().unwrap();
    assert_eq!(entry.0, USER_1);
    assert_eq!(entry.1, vec![SUBNET_FOR_RENT]);

    // check that the rental request is removed
    let rental_requests =
        query::<Vec<RentalRequest>>(&pic, SRC_ID, None, "list_rental_requests", ());
    assert!(rental_requests.is_empty());

    // check that the rental agreement is created
    let rental_agreements =
        query::<Vec<RentalAgreement>>(&pic, SRC_ID, None, "list_rental_agreements", ());
    assert_eq!(rental_agreements.len(), 1);

    let rental_agreement = rental_agreements.first().unwrap();

    // get rental condition (should still be there)
    let rental_conditions = query::<Vec<(RentalConditionId, RentalConditions)>>(
        &pic,
        SRC_ID,
        None,
        "list_rental_conditions",
        encode_one(()).unwrap(),
    );
    let rental_condition = rental_conditions.first().unwrap().1.clone();

    // check total cycles created
    let remaining_icp_to_be_converted = final_subnet_price.e8s()
        - (final_subnet_price.e8s() / 10)
        - (final_subnet_price.e8s() / 10); // 10% got locked twice

    assert_eq!(
        rental_request.initial_cost_icp.e8s() - rental_request.locked_amount_icp.e8s(),
        remaining_icp_to_be_converted
    );

    let expected_locked_cycles_execute_rental_agreement = (remaining_icp_to_be_converted as u128
        - DEFAULT_FEE.e8s() as u128) // minus the fee to convert ICP to cycles
        * cmc_exchange_rate_execute_rental_agreement as u128
        / 100_000;

    let expected_total_cycles = expected_locked_cycles_first_locking
        + expected_locked_cycles_second_locking
        + expected_locked_cycles_execute_rental_agreement;

    let expected_rental_agreement = RentalAgreement {
        user: USER_1,
        rental_request_proposal_id: 136408,
        subnet_creation_proposal_id: Some(137322),
        subnet_id: SUBNET_FOR_RENT,
        rental_condition_id: RentalConditionId::App13CH,
        creation_time_nanos: now_nanos,
        paid_until_nanos: rental_agreement.creation_time_nanos
            + rental_condition.initial_rental_period_days * 86400 * NANOS_PER_SECOND,
        total_icp_paid: final_subnet_price,
        total_cycles_created: expected_total_cycles,
        total_cycles_burned: 0,
    };

    assert_eq!(rental_agreement, &expected_rental_agreement);

    // check status of subnet rental
    let status_expired = check_subnet_status(&pic);
    assert!(status_expired.description.contains("OK"));
    assert_eq!(status_expired.cycles_left, expected_total_cycles);
    assert_eq!(status_expired.days_left, 180);

    // check SRC cycles balance
    let src_cycles_balance_after_execute_rental_agreement =
        pic.canister_status(SRC_ID, None).unwrap().cycles;
    assert_eq!(
        src_cycles_balance_after_execute_rental_agreement,
        INITIAL_SRC_CYCLES_BALANCE + expected_total_cycles - 1_000_000_000 // call of get_todays_price()
    );

    // try a refund, should give back the 100 ICP extra - fee, and leave the agreement in place
    let user_icp_balance_before_refund = check_balance(&pic, USER_1, DEFAULT_SUBACCOUNT);
    update::<Result<u64, String>>(&pic, SRC_ID, Some(USER_1), "refund", ())
        .unwrap()
        .unwrap();
    let user_icp_balance_after_refund = check_balance(&pic, USER_1, DEFAULT_SUBACCOUNT);
    assert_eq!(
        user_icp_balance_after_refund,
        user_icp_balance_before_refund + Tokens::from_e8s(100 * E8S) - DEFAULT_FEE
    );

    // check that the rental agreement is still in place
    let rental_agreements =
        query::<Vec<RentalAgreement>>(&pic, SRC_ID, None, "list_rental_agreements", ());
    assert_eq!(rental_agreements.len(), 1);
    let rental_agreement = rental_agreements.first().unwrap();
    assert_eq!(rental_agreement, &expected_rental_agreement);

    // advance time by 1 minute
    pic.advance_time(Duration::from_secs(60));
    for _ in 0..3 {
        pic.tick();
    }

    // check that the rental agreement is updated
    let rental_agreements =
        query::<Vec<RentalAgreement>>(&pic, SRC_ID, None, "list_rental_agreements", ());
    assert_eq!(rental_agreements.len(), 1);
    let rental_agreement = rental_agreements.first().unwrap();
    // check that total cycles created is unchanged
    assert_eq!(rental_agreement.total_cycles_created, expected_total_cycles);

    // check that total cycles burned is non-zero
    let amount_burned_in_first_minute = rental_agreement.total_cycles_burned;
    assert!(amount_burned_in_first_minute > 0);

    // check that the SRC cycles balance has decreased by the same amount as the total cycles burned
    let src_cycles_balance_after_burning = pic.canister_status(SRC_ID, None).unwrap().cycles;
    assert_eq!(
        src_cycles_balance_after_burning,
        src_cycles_balance_after_execute_rental_agreement - amount_burned_in_first_minute
    );

    // advance time by 1 minute
    pic.advance_time(Duration::from_secs(60));
    for _ in 0..3 {
        pic.tick();
    }

    // check that the amount burned in the second minute is the same as the first minute
    let rental_agreements =
        query::<Vec<RentalAgreement>>(&pic, SRC_ID, None, "list_rental_agreements", ());
    assert_eq!(rental_agreements.len(), 1);
    let rental_agreement = rental_agreements.first().unwrap();
    let amount_burned_in_second_minute =
        rental_agreement.total_cycles_burned - amount_burned_in_first_minute;

    assert_eq!(
        amount_burned_in_second_minute,
        amount_burned_in_first_minute
    );

    // check that during the entire 180 days, roughly all cycles are burned at the current pace (1 TC as margin)
    let total_cycles_burned_expected = amount_burned_in_first_minute * 60 * 24 * 180;
    assert!(
        total_cycles_burned_expected.abs_diff(rental_agreement.total_cycles_created) < TRILLION
    );

    // check status of subnet rental
    let status_ok = check_subnet_status(&pic);
    assert!(status_ok.description.contains("OK"));
    assert_eq!(status_ok.days_left, 180 - 1); // since we advaced time by a few minutes above and round down

    // do a topup
    // set exchange rates
    let exchange_rate_for_topup = 4_103_000_000; // 1 ICP = 4.103 XDR
    set_cmc_exchange_rate(&pic, exchange_rate_for_topup);
    set_xrc_exchange_rate_last_midnight(&pic, exchange_rate_for_topup);

    // get user's ICP balance
    let user_balance_before_topup = check_balance(&pic, USER_1, DEFAULT_SUBACCOUNT);

    // get estimate for topup
    let estimate = update_multi_arg::<Result<TopupData, String>>(
        &pic,
        SRC_ID,
        None,
        "subnet_topup_estimate",
        (SUBNET_FOR_RENT, Tokens::from_e8s(1_000 * E8S)),
    )
    .unwrap()
    .unwrap();

    println!("estimate: {:?}", estimate);

    // try to do topup with insufficient funds
    let res = update::<Result<TopupData, String>>(
        &pic,
        SRC_ID,
        Some(USER_1),
        "process_subnet_topup",
        SUBNET_FOR_RENT,
    )
    .unwrap();

    // the conversion should fail as the user does not have any ICP
    assert!(res.unwrap_err().contains("insufficient funds"));

    // top up SRC account with 1000 ICP
    let topup = Tokens::from_e8s(1_000 * E8S);
    pay_src(&pic, USER_1, topup);

    let actual_topup = update::<Result<TopupData, String>>(
        &pic,
        SRC_ID,
        Some(USER_1),
        "process_subnet_topup",
        SUBNET_FOR_RENT,
    )
    .unwrap()
    .unwrap();

    // check that the user's balance has decreased by exactly the amount for the topup plus the fee
    let users_balance_after_topup = check_balance(&pic, USER_1, DEFAULT_SUBACCOUNT);
    assert_eq!(
        users_balance_after_topup,
        user_balance_before_topup - topup - DEFAULT_FEE
    );

    // check that the topup is correct, and the estimate yields the same as the actual topup if the price stayed the same
    let expected_topup_cycles = (topup - DEFAULT_FEE - DEFAULT_FEE).e8s() as u128 // transfer to the SRC and transfer to the CMC
        * exchange_rate_for_topup as u128
        / 100_000;

    let expected_topup_days = (expected_topup_cycles / rental_condition.daily_cost_cycles) as u64;
    assert_eq!(actual_topup.cycles_added, expected_topup_cycles);
    assert_eq!(actual_topup.days_added, expected_topup_days);
    assert!(actual_topup
        .description
        .contains(&format!("{expected_topup_cycles} cycles")));
    assert!(actual_topup
        .description
        .contains(&format!("by {expected_topup_days} days")));
    assert!(actual_topup
        .description
        .contains(&format!("subnet {}", SUBNET_FOR_RENT)));

    assert_eq!(actual_topup.cycles_added, estimate.cycles_added);
    assert_eq!(actual_topup.days_added, estimate.days_added);

    // check status of subnet rental
    let status_ok = check_subnet_status(&pic);
    assert!(status_ok.description.contains("OK"));
    assert_eq!(status_ok.days_left, 180 - 1 + expected_topup_days); // since we advaced time by a few minutes above and rounding down

    // advance time to after the 180 + expected_topup_days -1 days (to see warning)
    pic.advance_time(Duration::from_secs((180 + expected_topup_days - 1) * 86400));
    for _ in 0..3 {
        pic.tick();
    }

    // check status of subnet rental
    let status_expired = check_subnet_status(&pic);
    assert!(status_expired.description.contains("WARNING"));
    assert!(status_expired.cycles_left > 0);
    assert_eq!(status_expired.days_left, 0);

    // advance time to after the 1 day (to see rental agreement end)
    pic.advance_time(Duration::from_secs(86400));
    for _ in 0..3 {
        pic.tick();
    }

    // check status of subnet rental
    let status_expired = check_subnet_status(&pic);
    assert!(status_expired.description.contains("TERMINATED"));
    assert_eq!(status_expired.cycles_left, 0);
    assert_eq!(status_expired.days_left, 0);

    // all cycles should be burned
    // check that the SRC cycles balance is back to the initial balance minus the one call of get_todays_price() that was made
    let src_cycles_balance_after_burning = pic.canister_status(SRC_ID, None).unwrap().cycles;
    assert_eq!(
        src_cycles_balance_after_burning,
        INITIAL_SRC_CYCLES_BALANCE - 1_000_000_000 * 2 // 2 calls of get_todays_price(), one explicitly above in the test, one to get the estimate for the topup
    );

    // check that the rental agreement is updated, stating that all cycles are burned
    let rental_agreements =
        query::<Vec<RentalAgreement>>(&pic, SRC_ID, None, "list_rental_agreements", ());
    assert_eq!(rental_agreements.len(), 1);
    let rental_agreement = rental_agreements.first().unwrap();
    assert_eq!(
        rental_agreement.total_cycles_burned,
        rental_agreement.total_cycles_created
    );
}

#[test]
fn test_failed_initial_proposal() {
    let pic = setup();

    let user_principal = USER_1;

    // set an exchange rate for the last midnight on the XRC mock
    set_xrc_exchange_rate_last_midnight(&pic, 12_503_823_284); // 1 ICP = 12.503823284 XDR

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
        Some(MAINNET_GOVERNANCE_CANISTER_ID),
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
    set_xrc_exchange_rate_last_midnight(&pic, 12_503_823_284); // 1 ICP = 12.503823284 XDR

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
        Some(MAINNET_GOVERNANCE_CANISTER_ID),
        "execute_rental_request_proposal",
        payload.clone(),
    )
    .unwrap();

    // it must only work the first time for the same user
    let res = update::<()>(
        &pic,
        SRC_ID,
        Some(MAINNET_GOVERNANCE_CANISTER_ID),
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
    set_xrc_exchange_rate_last_midnight(&pic, 12_503_823_284); // 1 ICP = 12.503823284 XDR

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
        Some(MAINNET_GOVERNANCE_CANISTER_ID),
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
        Err(message) => {
            panic!("Query expected Reply, got Reject: \n{}", message)
        }
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
        Err(message) => {
            panic!("Query expected Reply, got Reject: \n{}", message)
        }
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

fn update_multi_arg<T: CandidType + for<'a> Deserialize<'a>>(
    pic: &PocketIc,
    canister_id: Principal,
    sender: Option<Principal>,
    method: &str,
    args: impl CandidType + ArgumentEncoder,
) -> Result<T, String> {
    match pic.update_call(
        canister_id,
        sender.unwrap_or(Principal::anonymous()),
        method,
        encode_args(args).unwrap(),
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

fn check_subnet_status(pic: &PocketIc) -> RentalAgreementStatus {
    query::<Result<RentalAgreementStatus, String>>(
        pic,
        SRC_ID,
        None,
        "rental_agreement_status",
        SUBNET_FOR_RENT,
    )
    .unwrap()
}
