use candid::{decode_one, encode_args, encode_one, Principal};
use ic_ledger_types::{
    AccountBalanceArgs, AccountIdentifier, BlockIndex, Memo, Subaccount, Tokens, TransferArgs,
    TransferError, DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_GOVERNANCE_CANISTER_ID,
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
const LEDGER_WASM: &str = "./tests/ledger-canister.wasm.gz";

const SUBNET_FOR_RENT: &str = "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae";
const E8S: u64 = 100_000_000;
const USER_1: Principal = Principal::from_slice(b"user1");
const USER_1_INITIAL_BALANCE: Tokens = Tokens::from_e8s(1_000 * E8S);
const USER_2: Principal = Principal::from_slice(b"user2");
const USER_2_INITIAL_BALANCE: Tokens = Tokens::from_e8s(DEFAULT_FEE.e8s() * 2);

fn setup() -> (PocketIc, Principal) {
    let pic = PocketIcBuilder::new().with_nns_subnet().build();

    // Install subnet rental canister.
    let subnet_rental_canister = pic.create_canister();
    let src_wasm = fs::read(SRC_WASM).expect("Build the wasm with ./scripts/build.sh");
    pic.install_canister(subnet_rental_canister, src_wasm, vec![], None);

    // Install ICP ledger canister.
    pic.create_canister_with_id(
        Some(MAINNET_LEDGER_CANISTER_ID),
        None,
        MAINNET_LEDGER_CANISTER_ID,
    )
    .unwrap();
    let icp_ledger_canister_wasm = fs::read(LEDGER_WASM)
        .expect("Download the test wasm files with ./scripts/download_wasms.sh");

    let controller_and_minter =
        AccountIdentifier::new(&MAINNET_LEDGER_CANISTER_ID, &DEFAULT_SUBACCOUNT);
    let user_1 = AccountIdentifier::new(&USER_1, &DEFAULT_SUBACCOUNT);
    let user_2 = AccountIdentifier::new(&USER_2, &DEFAULT_SUBACCOUNT);

    let icp_ledger_init_args = NnsLedgerCanisterPayload::Init(NnsLedgerCanisterInitPayload {
        minting_account: controller_and_minter.to_string(),
        initial_values: HashMap::from([
            (
                controller_and_minter.to_string(),
                Tokens::from_e8s(1_000_000_000 * E8S),
            ),
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
        Some(MAINNET_LEDGER_CANISTER_ID),
    );

    (pic, subnet_rental_canister)
}

#[test]
fn test_attempt_refund_balance_zero() {
    let (pic, src_id) = setup();

    let user = Principal::from_slice(b"no_balance_user");
    let subnet_id = Principal::from_text(SUBNET_FOR_RENT).unwrap();

    // Attempt refund for valid subnet fails
    let WasmResult::Reply(res) = pic
        .update_call(
            src_id,
            user,
            "attempt_refund",
            encode_one(subnet_id).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    let res = decode_one::<Result<BlockIndex, TransferError>>(&res).unwrap();

    assert!(matches!(
        res,
        Err(TransferError::InsufficientFunds {
            balance: Tokens::ZERO
        })
    ));

    // Attempt refund from another subnet also fails
    let WasmResult::Reply(res) = pic
        .update_call(
            src_id,
            user,
            "attempt_refund",
            encode_one(Principal::anonymous()).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    let res = decode_one::<Result<BlockIndex, TransferError>>(&res).unwrap();

    assert!(matches!(
        res,
        Err(TransferError::InsufficientFunds {
            balance: Tokens::ZERO
        })
    ));
}

#[test]
fn test_attempt_refund_success() {
    let (pic, src_id) = setup();
    let subnet_id = Principal::from_text(SUBNET_FOR_RENT).unwrap();

    // Get Subaccount
    let WasmResult::Reply(res) = pic
        .query_call(
            src_id,
            USER_1,
            "get_subaccount",
            encode_args((USER_1, subnet_id)).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    let subaccount = decode_one::<Subaccount>(&res).unwrap();

    let initial_balance = check_user_1_balance(&pic);
    assert_eq!(initial_balance, USER_1_INITIAL_BALANCE);

    // Transfer 100 ICP to the subaccount
    let transfer_amount = Tokens::from_e8s(100 * E8S);
    let WasmResult::Reply(res) = pic
        .update_call(
            MAINNET_LEDGER_CANISTER_ID,
            USER_1,
            "transfer",
            encode_one(TransferArgs {
                memo: Memo(0),
                amount: transfer_amount,
                fee: DEFAULT_FEE,
                from_subaccount: None,
                to: AccountIdentifier::new(&src_id, &subaccount),
                created_at_time: None,
            })
            .unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };

    let transfer_result = decode_one::<Result<BlockIndex, TransferError>>(&res).unwrap();
    assert!(transfer_result.is_ok());

    // Balance is reduced by transfer amount and fee
    let balance = check_user_1_balance(&pic);
    assert_eq!(balance, initial_balance - transfer_amount - DEFAULT_FEE);

    // Attempt refund from another user fails
    let WasmResult::Reply(res) = pic
        .update_call(
            src_id,
            Principal::from_slice(b"some other user"),
            "attempt_refund",
            encode_one(subnet_id).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    let attempt_refund = decode_one::<Result<BlockIndex, TransferError>>(&res).unwrap();
    assert!(attempt_refund.is_err());

    // Balance is unchanged
    let balance_after_failed_withdrawal = check_user_1_balance(&pic);
    assert_eq!(balance_after_failed_withdrawal, balance);

    // Attempt refund for another subnet
    let WasmResult::Reply(res) = pic
        .update_call(
            src_id,
            USER_1,
            "attempt_refund",
            encode_one(Principal::anonymous()).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    let attempt_refund = decode_one::<Result<BlockIndex, TransferError>>(&res).unwrap();
    assert!(attempt_refund.is_err());

    // Balance is unchanged
    let balance_after_failed_withdrawal_2 = check_user_1_balance(&pic);
    assert_eq!(balance_after_failed_withdrawal_2, balance);

    // Attempt refund success
    let WasmResult::Reply(res) = pic
        .update_call(
            src_id,
            USER_1,
            "attempt_refund",
            encode_one(subnet_id).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    let attempt_refund = decode_one::<Result<BlockIndex, TransferError>>(&res).unwrap();
    assert!(attempt_refund.is_ok());

    let balance = check_user_1_balance(&pic);
    assert_eq!(balance, initial_balance - DEFAULT_FEE - DEFAULT_FEE);

    // Attempt refund to refund again fails
    let WasmResult::Reply(res) = pic
        .update_call(
            src_id,
            USER_1,
            "attempt_refund",
            encode_one(subnet_id).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    let attempt_refund = decode_one::<Result<BlockIndex, TransferError>>(&res).unwrap();
    assert!(attempt_refund.is_err());

    // Balance is unchanged
    let balance_after_failed_withdrawal_3 = check_user_1_balance(&pic);
    assert_eq!(balance_after_failed_withdrawal_3, balance);
}

#[test]
fn test_attempt_refund_close_to_zero() {
    let (pic, src_id) = setup();
    let subnet_id = Principal::from_text(SUBNET_FOR_RENT).unwrap();

    // Get Subaccount
    let WasmResult::Reply(res) = pic
        .query_call(
            src_id,
            USER_2,
            "get_subaccount",
            encode_args((USER_2, subnet_id)).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    let subaccount = decode_one::<Subaccount>(&res).unwrap();

    let initial_balance = check_user_2_balance(&pic);
    assert_eq!(initial_balance, USER_2_INITIAL_BALANCE);

    // Transfer DEFAULT_FEE ICP to the subaccount
    let transfer_amount = DEFAULT_FEE;
    let WasmResult::Reply(res) = pic
        .update_call(
            MAINNET_LEDGER_CANISTER_ID,
            USER_2,
            "transfer",
            encode_one(TransferArgs {
                memo: Memo(0),
                amount: transfer_amount,
                fee: DEFAULT_FEE,
                from_subaccount: None,
                to: AccountIdentifier::new(&src_id, &subaccount),
                created_at_time: None,
            })
            .unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };

    let transfer_result = decode_one::<Result<BlockIndex, TransferError>>(&res).unwrap();
    assert!(transfer_result.is_ok());

    // Balance is reduced by transfer amount and fee
    // Balance is now 0
    let balance = check_user_2_balance(&pic);
    assert_eq!(balance, initial_balance - transfer_amount - DEFAULT_FEE);
    assert_eq!(balance, Tokens::ZERO);

    // Attempt refund success
    let WasmResult::Reply(res) = pic
        .update_call(
            src_id,
            USER_2,
            "attempt_refund",
            encode_one(subnet_id).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    let attempt_refund = decode_one::<Result<BlockIndex, TransferError>>(&res).unwrap();
    assert!(attempt_refund.is_ok());

    let balance = check_user_2_balance(&pic);
    assert_eq!(balance, initial_balance - DEFAULT_FEE - DEFAULT_FEE);
    assert_eq!(balance, Tokens::ZERO);
}

fn check_user_1_balance(pic: &PocketIc) -> Tokens {
    check_user_balance(pic, USER_1)
}

fn check_user_2_balance(pic: &PocketIc) -> Tokens {
    check_user_balance(pic, USER_2)
}

fn check_user_balance(pic: &PocketIc, user: Principal) -> Tokens {
    let WasmResult::Reply(res) = pic
        .query_call(
            MAINNET_LEDGER_CANISTER_ID,
            user,
            "account_balance",
            encode_one(AccountBalanceArgs {
                account: AccountIdentifier::new(&user, &DEFAULT_SUBACCOUNT),
            })
            .unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };
    decode_one(&res).unwrap()
}

#[test]
fn test_get_subaccount() {
    let (pic, canister_id) = setup();

    let subnet_id = Principal::from_text(SUBNET_FOR_RENT).unwrap();

    let WasmResult::Reply(res) = pic
        .query_call(
            canister_id,
            Principal::anonymous(),
            "get_subaccount",
            encode_args((USER_1, subnet_id)).unwrap(),
        )
        .unwrap()
    else {
        panic!("Expected a reply")
    };

    let actual = decode_one::<Subaccount>(&res).unwrap();

    let mut hasher = Sha256::new();
    hasher.update(USER_1.as_slice());
    hasher.update(subnet_id.as_slice());
    let should = Subaccount(hasher.finalize().into());

    assert_eq!(actual, should);
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
    let arg = ValidatedSubnetRentalProposal {
        subnet_id: Principal::from_text(subnet_id_str).unwrap().into(),
        user: USER_1.into(),
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
    let wasm_res = add_test_rental_agreement(&pic, &canister_id, SUBNET_FOR_RENT);

    let WasmResult::Reply(res) = wasm_res else {
        panic!("Expected a reply");
    };

    let res = decode_one::<Result<(), ExecuteProposalError>>(&res).unwrap();
    assert!(res.is_ok());

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
        subnet_id: Principal::from_text(SUBNET_FOR_RENT).unwrap().into(),
        user: USER_1.into(),
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
