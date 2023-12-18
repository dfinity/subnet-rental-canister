use candid::{CandidType, Decode, Deserialize, Encode};
use ic_cdk::{init, query, update};
use ic_ledger_types::{
    account_balance, transfer, AccountBalanceArgs, AccountIdentifier, BlockIndex, Memo, Subaccount,
    TransferArgs, TransferError, DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_GOVERNANCE_CANISTER_ID,
    MAINNET_LEDGER_CANISTER_ID,
};
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    DefaultMemoryImpl, StableBTreeMap, Storable,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{borrow::Cow, cell::RefCell, collections::HashMap};

pub mod external_types;
mod http_request;

// During billing, the cost in cycles is fixed, but the cost in ICP depends on the exchange rate
const _XDR_COST_PER_DAY: u64 = 1;
const T: u128 = 1_000_000_000_000;

type SubnetId = Principal;

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    // Memory region 0
    static RENTAL_AGREEMENTS: RefCell<StableBTreeMap<Principal, RentalAgreement, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    /// Hardcoded subnets and their rental conditions.
    static SUBNETS: RefCell<HashMap<Principal, RentalConditions>> = HashMap::from([
        (candid::Principal::from_text("bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe").unwrap().into(),
            RentalConditions {
                daily_cost_cycles: 1_000 * T,
                initial_rental_period_days: 365,
                billing_period_days: 30,
                warning_threshold_days: 60,
            },
        ),
        (candid::Principal::from_text("fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae").unwrap().into(),
            RentalConditions {
                daily_cost_cycles: 2_000 * T,
                initial_rental_period_days: 183,
                billing_period_days: 30,
                warning_threshold_days: 4 * 7,
            },
        ),
    ]).into();
}

const MAX_PRINCIPAL_SIZE: u32 = 29;

#[derive(
    Debug, Clone, Copy, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, CandidType, Hash,
)]
pub struct Principal(pub candid::Principal);

impl From<candid::Principal> for Principal {
    fn from(value: candid::Principal) -> Self {
        Self(value)
    }
}

impl Storable for Principal {
    const BOUND: Bound = Bound::Bounded {
        max_size: MAX_PRINCIPAL_SIZE,
        is_fixed_size: false,
    };
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(self.0.as_slice().to_vec())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Self(candid::Principal::try_from_slice(bytes.as_ref()).unwrap())
    }
}

/// Set of conditions for a specific subnet up for rent.
#[derive(Debug, Clone, Copy, CandidType, Deserialize)]
pub struct RentalConditions {
    daily_cost_cycles: u128,
    initial_rental_period_days: u64,
    billing_period_days: u64,
    warning_threshold_days: u64,
}

/// Immutable rental agreement; mutabla data and log events should refer to it via the id.
#[derive(Debug, Clone, CandidType, Deserialize)]
struct RentalAgreement {
    user: Principal,
    subnet_id: SubnetId,
    principals: Vec<Principal>,
    rental_conditions: RentalConditions,
    // nanoseconds since epoch
    creation_date: u64,
}

impl Storable for RentalAgreement {
    // should be bounded once we replace string with real type
    const BOUND: Bound = Bound::Unbounded;
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

#[init]
fn init() {
    ic_cdk::println!("Subnet rental canister initialized");
}

#[update]
fn demo_add_rental_agreement() {
    // TODO: remove this endpoint before release
    // Hardcoded rental agreement for testing
    let subnet_id = candid::Principal::from_text(
        "bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe",
    )
    .unwrap()
    .into();
    let renter = candid::Principal::from_slice(b"user1").into();
    let user = candid::Principal::from_slice(b"user2").into();
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(
            subnet_id,
            RentalAgreement {
                user: renter,
                subnet_id,
                principals: vec![renter, user],
                rental_conditions: RentalConditions {
                    daily_cost_cycles: 1_000 * T,
                    initial_rental_period_days: 365,
                    billing_period_days: 30,
                    warning_threshold_days: 60,
                },
                creation_date: 1702394252000000000,
            },
        )
    });
}

#[update]
/// Attempts to refund the user's deposit. If the user has insufficient funds, returns an error.
/// Otherwise, returns the block index of the transaction.
// TODO: what to do if calling ICP ledger canister fails?
async fn attempt_refund(subnet_id: candid::Principal) -> Result<BlockIndex, TransferError> {
    let caller = ic_cdk::caller();

    // Generate subaccount for caller and specified subnet
    let subaccount = get_subaccount(caller, subnet_id);

    // Check SRC's balance of that subaccount
    let balance_args = AccountBalanceArgs {
        account: AccountIdentifier::new(&ic_cdk::id(), &subaccount),
    };
    let balance = account_balance(MAINNET_LEDGER_CANISTER_ID, balance_args)
        .await
        .expect("Failed to call ICP ledger canister");

    if balance < DEFAULT_FEE {
        ic_cdk::println!("Balance is insufficient");
        return Err(TransferError::InsufficientFunds { balance });
    }

    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        TransferArgs {
            memo: Memo(0), // TODO: should we use a memo?
            amount: balance - DEFAULT_FEE,
            fee: DEFAULT_FEE,
            from_subaccount: Some(subaccount),
            to: AccountIdentifier::new(&caller, &DEFAULT_SUBACCOUNT),
            created_at_time: None, // TODO: should we use a timestamp?
        },
    )
    .await
    .expect("Failed to call ICP ledger canister")
}

#[query]
fn get_subaccount(user: candid::Principal, subnet_id: candid::Principal) -> Subaccount {
    let mut hasher = Sha256::new();
    hasher.update(user.as_slice());
    hasher.update(subnet_id.as_slice());
    Subaccount(hasher.finalize().into())
}

#[query]
fn list_subnet_conditions() -> HashMap<SubnetId, RentalConditions> {
    SUBNETS.with(|map| map.borrow().clone())
}

#[query]
fn list_rental_agreements() -> Vec<RentalAgreement> {
    RENTAL_AGREEMENTS.with(|map| map.borrow().iter().map(|(_, v)| v).collect())
}

#[derive(Clone, CandidType, Deserialize)]
pub struct ValidatedSubnetRentalProposal {
    pub subnet_id: Principal,
    pub user: Principal,
    pub principals: Vec<Principal>,
}

#[derive(CandidType, Debug, Clone, Deserialize)]
pub enum ExecuteProposalError {
    Failure(String),
    SubnetAlreadyRented,
    UnauthorizedCaller,
    InsufficientFunds,
}

/// TODO: Argument should be something like ValidatedSRProposal, created by government canister via
/// SRProposal::validate().
/// validate needs to ensure:
/// - subnet not currently rented
/// - A single deposit transaction exists and covers the necessary amount.
/// - The deposit was made to the <subnet_id>-subaccount of the SRC.
#[update]
async fn accept_rental_agreement(
    ValidatedSubnetRentalProposal {
        subnet_id,
        user,
        principals,
    }: ValidatedSubnetRentalProposal,
) -> Result<(), ExecuteProposalError> {
    verify_caller_is_governance()?;

    // Get rental conditions.
    // If the governance canister was able to validate, then this entry must exist, so we can unwrap.
    let rental_conditions = SUBNETS.with(|rc| *rc.borrow().get(&subnet_id).unwrap());
    // Creation date in nanoseconds since epoch.
    let creation_date = ic_cdk::api::time();

    let rental_agreement = RentalAgreement {
        user,
        subnet_id,
        principals,
        rental_conditions,
        creation_date,
    };

    // Add the rental agreement to the rental agreement map.
    if RENTAL_AGREEMENTS.with(|map| map.borrow().contains_key(&subnet_id)) {
        ic_cdk::println!(
            "Subnet is already in an active rental agreement: {:?}",
            &subnet_id
        );
        return Err(ExecuteProposalError::SubnetAlreadyRented);
    }
    // TODO: log this event in the persisted log
    ic_cdk::println!("Creating rental agreement: {:?}", &rental_agreement);
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(subnet_id, rental_agreement);
    });

    // TODO: Whitelist the principal
    // let result: CallResult<()> = call(CMC, "set_authorized_subnetwork_list", (Some(user), vec![subnet_id])).await;

    Ok(())
}

#[derive(Clone, CandidType, Deserialize, Debug)]
pub struct RejectedSubnetRentalProposal {
    pub nns_proposal_id: u64,
    pub refund_address: [u8; 32],
}

fn verify_caller_is_governance() -> Result<(), ExecuteProposalError> {
    if ic_cdk::caller() != MAINNET_GOVERNANCE_CANISTER_ID {
        ic_cdk::println!("Caller is not the governance canister");
        return Err(ExecuteProposalError::UnauthorizedCaller);
    }
    Ok(())
}
