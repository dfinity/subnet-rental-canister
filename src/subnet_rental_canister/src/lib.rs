use candid::{CandidType, Decode, Deserialize, Encode, Nat};
use external_types::NotifyError;
use history::{Event, EventType, History};
use ic_cdk::println;
use ic_ledger_types::{
    transfer, AccountIdentifier, Memo, Subaccount, Tokens, TransferArgs, TransferError,
    DEFAULT_FEE, MAINNET_CYCLES_MINTING_CANISTER_ID, MAINNET_GOVERNANCE_CANISTER_ID,
    MAINNET_LEDGER_CANISTER_ID,
};
use ic_stable_structures::Memory;
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    DefaultMemoryImpl, StableBTreeMap, Storable,
};
use icrc_ledger_types::icrc1::account::Account;
use icrc_ledger_types::icrc2::transfer_from::{TransferFromArgs, TransferFromError};

use serde::Serialize;
use std::{borrow::Cow, cell::RefCell, time::Duration};

use crate::external_types::{
    IcpXdrConversionRate, IcpXdrConversionRateResponse, NotifyTopUpArg,
    SetAuthorizedSubnetworkListArgs,
};

pub mod canister;
pub mod external_types;
pub mod history;
mod http_request;

pub const TRILLION: u128 = 1_000_000_000_000;
pub const E8S: u64 = 100_000_000;
const MAX_PRINCIPAL_SIZE: u32 = 29;
const BILLING_INTERVAL: Duration = Duration::from_secs(60 * 60); // hourly
const MEMO_TOP_UP_CANISTER: Memo = Memo(0x50555054); // == 'TPUP'

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    // Memory region 0
    // Only modify via _set_rental_conditions so that history is updated alongside.
    static RENTAL_CONDITIONS: RefCell<StableBTreeMap<Principal, RentalConditions, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    // Memory region 1
    static RENTAL_AGREEMENTS: RefCell<StableBTreeMap<Principal, RentalAgreement, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1)))));

    // Memory region 2
    static BILLING_RECORDS: RefCell<StableBTreeMap<Principal, BillingRecord, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2)))));

    // Memory region 3
    static HISTORY: RefCell<StableBTreeMap<Principal, History, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(3)))));

}

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
}

impl Storable for RentalConditions {
    // TODO: find max size and bound
    const BOUND: Bound = Bound::Unbounded;
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

/// The governance canister validates proposals and calls the SRC with
/// this argument. It limits the principals vector to a reasonable size.
#[derive(Clone, CandidType, Deserialize)]
pub struct ValidatedSubnetRentalProposal {
    pub subnet_id: candid::Principal,
    pub user: candid::Principal,
    /// Other principals to be whitelisted.
    pub principals: Vec<candid::Principal>,
    /// Nanoseconds since epoch.
    pub proposal_creation_time: u64,
}

/// The governance canister calls the SRC with this argument on
/// a termination proposal.
#[derive(Clone, CandidType, Deserialize)]
pub struct RentalTerminationProposal {
    subnet_id: candid::Principal,
}

#[derive(CandidType, Debug, Clone, Deserialize)]
pub enum ExecuteProposalError {
    SubnetNotRentable,
    SubnetAlreadyRented,
    UnauthorizedCaller,
    InsufficientFunds,
    TransferUserToSrcError(TransferFromError),
    TransferSrcToCmcError(TransferError),
    NotifyTopUpError(NotifyError),
    SubnetNotRented,
}
/// Immutable rental agreement; mutabla data belongs in BillingRecord. A rental agreement is
/// uniquely identified by the (subnet_id, creation_date) 'composite key'.
#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct RentalAgreement {
    /// The principal which pays for the subnet via ICRC-2 approval. Will be whitelisted.
    pub user: Principal,
    /// The subnet to be rented.
    pub subnet_id: Principal,
    /// Other principals to be whitelisted.
    pub principals: Vec<Principal>,
    /// Rental agreement creation date in nanoseconds since epoch.
    pub creation_date: u64,
}

impl RentalAgreement {
    pub fn get_rental_conditions(&self) -> RentalConditions {
        // unwrap justified because no rental agreement can exist without rental conditions
        RENTAL_CONDITIONS.with(|map| map.borrow().get(&self.subnet_id).unwrap())
    }
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

/// Mutable data belonging to an active rental agreement.
#[derive(Debug, Clone, Copy, CandidType, Deserialize)]
pub struct BillingRecord {
    /// The date (in nanos since epoch) until which the rental agreement is paid for.
    pub covered_until: u64,
    /// This subnet's share of cycles among the SRC's cycles.
    /// Increased by the payment process (via timer).
    /// Decreased by the burning process (via heartbeat).
    pub cycles_balance: u128,
    /// The last point in time (nanos since epoch) when cycles were burned in a heartbeat.
    pub last_burned: u64,
}

impl Storable for BillingRecord {
    // Should be bounded once we replace string with real type.
    const BOUND: Bound = Bound::Bounded {
        max_size: 54, // TODO: figure out the actual size
        is_fixed_size: false,
    };
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

/// Rental agreement map and billing records map must be in sync, so we add them together
fn create_rental_agreement(
    subnet_id: Principal,
    rental_agreement: RentalAgreement,
    billing_record: BillingRecord,
) {
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(subnet_id, rental_agreement.clone());
    });
    println!("Created rental agreement: {:?}", &rental_agreement);
    BILLING_RECORDS.with(|map| map.borrow_mut().insert(subnet_id, billing_record));
    println!("Created billing record: {:?}", &billing_record);
    persist_event(EventType::Created { rental_agreement }, subnet_id);
}

/// Rental agreements have an associated BillingAccount, which must be removed at the same time.
/// TODO: only call this if agreement exists...
fn delete_rental_agreement(subnet_id: Principal) {
    let rental_agreement =
        RENTAL_AGREEMENTS.with(|map| map.borrow_mut().remove(&subnet_id).unwrap());
    let billing_record = BILLING_RECORDS.with(|map| map.borrow_mut().remove(&subnet_id).unwrap());
    persist_event(
        EventType::Terminated {
            rental_agreement,
            billing_record,
        },
        subnet_id,
    );
}

async fn whitelist_principals(subnet_id: candid::Principal, principals: &Vec<Principal>) {
    for user in principals {
        ic_cdk::call::<_, ()>(
            MAINNET_CYCLES_MINTING_CANISTER_ID,
            "set_authorized_subnetwork_list",
            (SetAuthorizedSubnetworkListArgs {
                who: Some(user.0),
                subnets: vec![subnet_id], // TODO: Add to the current list, don't overwrite
            },),
        )
        .await
        .expect("Failed to call CMC"); // TODO: handle error
    }
}

async fn delist_principals(_subnet_id: candid::Principal, principals: &Vec<candid::Principal>) {
    // TODO: if we allow multiple subnets per user:
    // first read the current list,
    // remove this subnet from the list and then
    // re-whitelist the principal for the remaining list
    for user in principals {
        ic_cdk::call::<_, ()>(
            MAINNET_CYCLES_MINTING_CANISTER_ID,
            "set_authorized_subnetwork_list",
            (SetAuthorizedSubnetworkListArgs {
                who: Some(*user),
                subnets: vec![],
            },),
        )
        .await
        .expect("Failed to call CMC"); // TODO: handle error
    }
}

async fn notify_top_up(block_index: u64) -> Result<u128, NotifyError> {
    ic_cdk::call::<_, (Result<u128, NotifyError>,)>(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        "notify_top_up",
        (NotifyTopUpArg {
            block_index,
            canister_id: ic_cdk::id(),
        },),
    )
    .await
    .expect("Failed to call CMC") // TODO: handle error
    .0
    // TODO: In the canister logs, the CMC claims that the burning of ICPs failed, but the cycles are minted anyway.
    // It states that the "transfer fee should be 0.00010000 Token", but that fee is hardcoded to
    // (ZERO)[https://sourcegraph.com/github.com/dfinity/ic@8126ad2fab0196908d9456a65914a3e05179ac4b/-/blob/rs/nns/cmc/src/main.rs?L1835]
    // in the CMC, and cannot be changed from outside. What's going on here?
}

async fn transfer_to_cmc(amount: Tokens) -> Result<u64, TransferError> {
    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        TransferArgs {
            to: AccountIdentifier::new(
                &MAINNET_CYCLES_MINTING_CANISTER_ID,
                &Subaccount::from(ic_cdk::id()),
            ),
            fee: DEFAULT_FEE,
            from_subaccount: None,
            amount,
            memo: MEMO_TOP_UP_CANISTER,
            created_at_time: None,
        },
    )
    .await
    .expect("Failed to call ledger canister") // TODO: handle error
}

async fn icrc2_transfer_to_src(
    user: candid::Principal,
    amount: Tokens,
) -> Result<u128, TransferFromError> {
    ic_cdk::call::<_, (Result<u128, TransferFromError>,)>(
        MAINNET_LEDGER_CANISTER_ID,
        "icrc2_transfer_from",
        (TransferFromArgs {
            to: Account {
                owner: ic_cdk::id(),
                subaccount: None,
            },
            fee: None,
            spender_subaccount: None,
            from: Account {
                owner: user,
                subaccount: None,
            },
            memo: None,
            created_at_time: None,
            amount: Nat::from(amount.e8s()),
        },),
    )
    .await
    .expect("Failed to call ledger canister") // TODO: handle error
    .0
}

async fn get_historical_avg_exchange_rate_cycles_per_e8s(timestamp: u64) -> u64 {
    // TODO: implement
    println!(
        "Getting historical average exchange rate for timestamp {}",
        timestamp
    );
    get_exchange_rate_cycles_per_e8s().await
}

async fn get_current_avg_exchange_rate_cycles_per_e8s() -> u64 {
    // TODO: implement
    get_exchange_rate_cycles_per_e8s().await
}

async fn get_exchange_rate_cycles_per_e8s() -> u64 {
    let IcpXdrConversionRateResponse {
        data: IcpXdrConversionRate {
            xdr_permyriad_per_icp,
            ..
        },
        ..
    } = ic_cdk::call::<_, (IcpXdrConversionRateResponse,)>(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        "get_icp_xdr_conversion_rate",
        (),
    )
    .await
    .expect("Failed to call CMC") // TODO: handle error
    .0;

    xdr_permyriad_per_icp
}

fn verify_caller_is_governance() -> Result<(), ExecuteProposalError> {
    if ic_cdk::caller() != MAINNET_GOVERNANCE_CANISTER_ID {
        println!("Caller is not the governance canister");
        return Err(ExecuteProposalError::UnauthorizedCaller);
    }
    Ok(())
}

fn days_to_nanos(days: u64) -> u64 {
    days * 24 * 60 * 60 * 1_000_000_000
}

/// Use only this function to make changes to RENTAL_CONDITIONS, so that
/// all changes are persisted in the history.
/// Internally used in canister_init, externally available as an update method
/// which only the governance canister can call, see set_rental_conditions().
pub fn _set_rental_conditions(
    subnet_id: candid::Principal,
    daily_cost_cycles: u128,
    initial_rental_period_days: u64,
    billing_period_days: u64,
) {
    let rental_conditions = RentalConditions {
        daily_cost_cycles,
        initial_rental_period_days,
        billing_period_days,
    };
    RENTAL_CONDITIONS.with(|map| map.borrow_mut().insert(subnet_id.into(), rental_conditions));
    persist_event(
        EventType::RentalConditionsChanged { rental_conditions },
        subnet_id,
    );
}

/// Called in canister_init
pub fn set_initial_rental_conditions() {
    _set_rental_conditions(
        candid::Principal::from_text(
            "bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe",
        )
        .unwrap(),
        1_000 * TRILLION,
        365,
        30,
    );
    _set_rental_conditions(
        candid::Principal::from_text(
            "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
        )
        .unwrap(),
        2_000 * TRILLION,
        183,
        30,
    );
}

/// Pass one of the global StableBTreeMaps and a function that transforms a value.
fn update_map<K, V, M>(map: &RefCell<StableBTreeMap<K, V, M>>, f: impl Fn(K, V) -> V)
where
    K: Storable + Ord + Clone,
    V: Storable,
    M: Memory,
{
    let keys: Vec<K> = map.borrow().iter().map(|(k, _v)| k).collect();
    for key in keys {
        let value = map.borrow().get(&key).unwrap();
        map.borrow_mut().insert(key.clone(), f(key, value));
    }
}

fn persist_event(event: impl Into<Event>, subnet: impl Into<Principal>) {
    HISTORY.with(|map| {
        let subnet = subnet.into();
        let mut history = map.borrow().get(&subnet).unwrap_or_default();
        history.events.push(event.into());
        map.borrow_mut().insert(subnet, history);
    })
}
