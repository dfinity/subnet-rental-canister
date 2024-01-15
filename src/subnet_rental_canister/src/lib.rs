use candid::{CandidType, Decode, Deserialize, Encode, Nat};
use external_types::NotifyError;
use history::{Event, History};
use ic_cdk::{heartbeat, init, post_upgrade, println, query, update};
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
use itertools::Itertools;
use serde::Serialize;
use std::{borrow::Cow, cell::RefCell, time::Duration};

use crate::external_types::{
    IcpXdrConversionRate, IcpXdrConversionRateResponse, NotifyTopUpArg,
    SetAuthorizedSubnetworkListArgs,
};
use crate::history::EventType;

pub mod external_types;
pub mod history;
mod http_request;

pub const TRILLION: u128 = 1_000_000_000_000;
pub const E8S: u64 = 100_000_000;
const MAX_PRINCIPAL_SIZE: u32 = 29;
const BILLING_INTERVAL: Duration = Duration::from_secs(60 * 60); // hourly
const MEMO_TOP_UP_CANISTER: Memo = Memo(0x50555054); // == 'TPUP'

type SubnetId = Principal;

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

#[derive(Clone, CandidType, Deserialize)]
pub struct ValidatedSubnetRentalProposal {
    pub subnet_id: candid::Principal,
    pub user: candid::Principal,
    pub principals: Vec<candid::Principal>, // TODO: limit size
    pub proposal_creation_timestamp: u64,
}

#[derive(CandidType, Debug, Clone, Deserialize)]
pub enum ExecuteProposalError {
    SubnetAlreadyRented,
    UnauthorizedCaller,
    InsufficientFunds,
    TransferUserToSrcError(TransferFromError),
    TransferSrcToCmcError(TransferError),
    NotifyTopUpError(NotifyError),
}
/// Immutable rental agreement; mutabla data and log events should refer to it via the id.
#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct RentalAgreement {
    pub user: Principal,
    pub subnet_id: SubnetId,
    pub principals: Vec<Principal>,
    // nanoseconds since epoch
    creation_date: u64,
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

////////// CANISTER METHODS //////////

#[init]
fn init() {
    ic_cdk_timers::set_timer_interval(BILLING_INTERVAL, || ic_cdk::spawn(billing()));
    // Populate rental conditions map and persist these changes in history.
    set_initial_rental_conditions();
    println!("Subnet rental canister initialized");
}

#[post_upgrade]
fn post_upgrade() {
    ic_cdk_timers::set_timer_interval(BILLING_INTERVAL, || ic_cdk::spawn(billing()));
}

#[heartbeat]
fn canister_heartbeat() {
    BILLING_RECORDS.with(|map| {
        update_map(map, |subnet_id, billing_record| {
            let Some(rental_agreement) = RENTAL_AGREEMENTS.with(|map| map.borrow().get(&subnet_id))
            else {
                println!(
                    "Fatal: Failed to find active rental agreement for active billing record of subnet {}",
                    subnet_id.0
                );
                return billing_record;
            };
            let cost_cycles_per_second =
                rental_agreement.get_rental_conditions().daily_cost_cycles / 86400;
            let now = ic_cdk::api::time();
            let nanos_since_last_burn = now - billing_record.last_burned;
            // cost_cycles_per_second: ~10^10 < 10^12
            // nanos_since_last_burn:  ~10^9  < 10^15
            // product                        < 10^27 << 10^38 (u128_max)
            // divided by 1B                    10^-9
            // amount                         < 10^18
            let amount = cost_cycles_per_second * nanos_since_last_burn as u128 / 1_000_000_000;
            if billing_record.cycles_balance < amount {
                println!("Failed to burn cycles for subnet {}", subnet_id.0);
                return billing_record;
            }
            // TODO: disabled for testing;
            // let canister_total_available_cycles = ic_cdk::api::canister_balance128();
            // if canister_total_available_cycles < amount {
            //     println!(
            //         "Fatal: Canister has fewer cycles {} than subaccount {:?}: {}",
            //         canister_total_available_cycles, subnet_id, account.cycles_balance
            //     );
            //     return account;
            // }
            // Burn must succeed now
            ic_cdk::api::cycles_burn(amount);
            let cycles_balance = billing_record.cycles_balance - amount;
            let last_burned = now;
            println!(
                "Burned {} cycles for subnet {}, remaining: {}",
                amount, subnet_id.0, cycles_balance
            );
            BillingRecord {
                covered_until: billing_record.covered_until,
                cycles_balance,
                last_burned,
            }
        });
    });
}

////////// QUERY METHODS //////////

#[query]
fn list_rental_conditions() -> Vec<(SubnetId, RentalConditions)> {
    RENTAL_CONDITIONS.with(|map| map.borrow().iter().collect())
}

#[query]
fn list_rental_agreements() -> Vec<RentalAgreement> {
    RENTAL_AGREEMENTS.with(|map| map.borrow().iter().map(|(_, v)| v).collect())
}

#[query]
fn list_billing_records() -> Vec<(Principal, BillingRecord)> {
    BILLING_RECORDS.with(|map| map.borrow().iter().collect())
}

#[query]
fn get_history(subnet: candid::Principal) -> Option<Vec<Event>> {
    HISTORY.with(|map| {
        map.borrow()
            .get(&subnet.into())
            .map(|history| history.events)
    })
}

////////// UPDATE METHODS //////////

/// Insert or overwrite existing rental conditions with Some(value), or remove
/// rental conditions for this subnet altogether by passing None.
#[update]
fn set_rental_conditions(
    subnet_id: candid::Principal,
    mb_rental_conditions: Option<RentalConditions>,
) -> Result<(), ExecuteProposalError> {
    verify_caller_is_governance()?;
    // Insert or overwrite
    if let Some(RentalConditions {
        daily_cost_cycles,
        initial_rental_period_days,
        billing_period_days,
    }) = mb_rental_conditions
    {
        _set_rental_conditions(
            subnet_id,
            daily_cost_cycles,
            initial_rental_period_days,
            billing_period_days,
        );
    } else {
        // Remove this subnet as up for rent by deleting the rental conditions
        // Fail if an active rental agreement exists
        if RENTAL_AGREEMENTS.with(|map| map.borrow().contains_key(&subnet_id.into())) {
            return Err(ExecuteProposalError::SubnetAlreadyRented);
        }

        if let Some(rental_conditions) =
            RENTAL_CONDITIONS.with(|map| map.borrow_mut().remove(&subnet_id.into()))
        {
            persist_event(
                EventType::RentalConditionsRemoved { rental_conditions },
                subnet_id,
            );
        } else {
            println!(
                "Failed to remove rental conditions for subnet {}: Not found",
                subnet_id
            );
        }
    }
    Ok(())
}

/// Use only this function to make changes to RENTAL_CONDITIONS, so that
/// all changes are persisted in the history.
/// Internally used in canister init.
fn _set_rental_conditions(
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

/// Call this in init
fn set_initial_rental_conditions() {
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

#[update]
// TODO: remove this endpoint before release
fn demo_add_rental_agreement() {
    // Hardcoded rental agreement for testing.
    let subnet_id = candid::Principal::from_text(
        "bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe",
    )
    .unwrap()
    .into();
    let renter = candid::Principal::from_slice(b"user1").into();
    let user = candid::Principal::from_slice(b"user2").into();
    let creation_date = ic_cdk::api::time();
    let initial_rental_period_days = 365;
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(
            subnet_id,
            RentalAgreement {
                user: renter,
                subnet_id,
                principals: vec![renter, user],
                creation_date,
            },
        )
    });
    let test_exchange_rate_e8s_cycles: u128 = 72_401; // 1 ICP = 7.2401T cycles
    let test_icp_balance_e8s: u64 = 50_500 * E8S; // just over one year covered
    BILLING_RECORDS.with(|map| {
        map.borrow_mut().insert(
            subnet_id,
            BillingRecord {
                covered_until: creation_date + days_to_nanos(initial_rental_period_days),
                cycles_balance: test_icp_balance_e8s as u128 * test_exchange_rate_e8s_cycles,
                last_burned: creation_date,
            },
        )
    });
}

// TODO: Argument will be provided by governance canister after validation
#[update]
async fn accept_rental_agreement(
    ValidatedSubnetRentalProposal {
        subnet_id,
        user,
        principals,
        proposal_creation_timestamp,
    }: ValidatedSubnetRentalProposal,
) -> Result<(), ExecuteProposalError> {
    verify_caller_is_governance()?;

    let principals_to_whitelist = principals
        .into_iter()
        .chain(std::iter::once(user))
        .unique()
        .map(|p| p.into())
        .collect();

    // Get rental conditions.
    // If the governance canister was able to validate, then this entry must exist, so we can unwrap.
    let rental_conditions =
        RENTAL_CONDITIONS.with(|rc| rc.borrow().get(&subnet_id.into()).unwrap());

    if RENTAL_AGREEMENTS.with(|map| map.borrow().contains_key(&subnet_id.into())) {
        println!(
            "Subnet {} is already in an active rental agreement",
            &subnet_id
        );
        let err = ExecuteProposalError::SubnetAlreadyRented;
        persist_event(
            EventType::Failed {
                user: user.into(),
                reason: err.clone(),
            },
            subnet_id,
        );
        return Err(err);
    }

    // Attempt to transfer enough ICP to cover the initial rental period.
    let needed_cycles = rental_conditions
        .daily_cost_cycles
        .saturating_mul(rental_conditions.initial_rental_period_days as u128);
    let exchange_rate =
        get_historical_avg_exchange_rate_cycles_per_e8s(proposal_creation_timestamp).await; // TODO: might need rounding
    let needed_icp = Tokens::from_e8s((needed_cycles.saturating_div(exchange_rate as u128)) as u64);

    // Use ICRC2 to transfer ICP from the user to the SRC.
    let transfer_to_src_result = icrc2_transfer_to_src(user, needed_icp - DEFAULT_FEE).await;
    if let Err(err) = transfer_to_src_result {
        println!("Transfer from user to SRC failed: {:?}", err);
        persist_event(
            EventType::Failed {
                user: user.into(),
                reason: ExecuteProposalError::TransferUserToSrcError(err.clone()),
            },
            subnet_id,
        );
        return Err(ExecuteProposalError::TransferUserToSrcError(err));
    }

    // Whitelist principals for subnet.
    whitelist_principals(subnet_id, &principals_to_whitelist).await;
    let rental_agreement_creation_date = ic_cdk::api::time();

    // Transfer the ICP from the SRC to the CMC.
    let transfer_to_cmc_result = transfer_to_cmc(needed_icp - DEFAULT_FEE - DEFAULT_FEE).await;
    let Ok(block_index) = transfer_to_cmc_result else {
        let err = transfer_to_cmc_result.unwrap_err();
        println!("Transfer from SRC to CMC failed: {:?}", err);
        persist_event(
            EventType::Failed {
                user: user.into(),
                reason: ExecuteProposalError::TransferSrcToCmcError(err.clone()),
            },
            subnet_id,
        );
        return Err(ExecuteProposalError::TransferSrcToCmcError(err));
    };

    // Notify CMC about the top-up. This is what triggers the exchange from ICP to cycles.
    let notify_top_up_result = notify_top_up(block_index).await;
    let Ok(actual_cycles) = notify_top_up_result else {
        let err = notify_top_up_result.unwrap_err();
        println!("Notify top-up failed: {:?}", err);
        persist_event(
            EventType::Failed {
                user: user.into(),
                reason: ExecuteProposalError::NotifyTopUpError(err.clone()),
            },
            subnet_id,
        );
        return Err(ExecuteProposalError::NotifyTopUpError(err));
    };

    // Add the rental agreement to the rental agreement map.
    let rental_agreement = RentalAgreement {
        user: user.into(),
        subnet_id: subnet_id.into(),
        principals: principals_to_whitelist,
        creation_date: rental_agreement_creation_date,
    };
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut()
            .insert(subnet_id.into(), rental_agreement.clone());
    });
    println!("Created rental agreement: {:?}", &rental_agreement);

    // Add the billing record to the billing records map.
    let billing_record = BillingRecord {
        covered_until: rental_agreement_creation_date
            + days_to_nanos(rental_conditions.initial_rental_period_days),
        cycles_balance: actual_cycles, // TODO: what about remaining cycles? what if this billing record already exists?
        last_burned: rental_agreement_creation_date,
    };
    BILLING_RECORDS.with(|map| map.borrow_mut().insert(subnet_id.into(), billing_record));
    println!("Created billing record: {:?}", &billing_record);

    persist_event(EventType::Created { rental_agreement }, subnet_id);

    Ok(())
}

////////// HELPER FUNCTIONS //////////

async fn billing() {
    let exchange_rate_cycles_per_e8s = get_current_avg_exchange_rate_cycles_per_e8s().await;

    for (subnet_id, rental_agreement) in
        RENTAL_AGREEMENTS.with(|map| map.borrow().iter().collect::<Vec<_>>())
    {
        {
            let Some(BillingRecord { covered_until, .. }) =
                BILLING_RECORDS.with(|map| map.borrow_mut().get(&subnet_id))
            else {
                println!(
                    "FATAL: No billing record found for active rental agreement for subnet {}",
                    &subnet_id.0
                );
                continue;
            };

            // Check if subnet is covered for next billing_period amount of days.
            let now = ic_cdk::api::time();
            let billing_period_nanos =
                days_to_nanos(rental_agreement.get_rental_conditions().billing_period_days);

            if covered_until < now {
                println!(
                    "Subnet {} is not covered anymore, degrading...",
                    subnet_id.0
                );
                // TODO: Degrade service
                persist_event(EventType::Degraded, subnet_id);
            } else if covered_until < now + billing_period_nanos {
                // Next billing period is not fully covered anymore.
                // Try to withdraw ICP and convert to cycles.
                let needed_cycles = rental_agreement
                    .get_rental_conditions()
                    .daily_cost_cycles
                    .saturating_mul(
                        rental_agreement.get_rental_conditions().billing_period_days as u128,
                    ); // TODO: get up to date rental conditions

                let icp_amount = Tokens::from_e8s(
                    needed_cycles.saturating_div(exchange_rate_cycles_per_e8s as u128) as u64,
                );

                // Transfer ICP to SRC.
                let transfer_to_src_result =
                    icrc2_transfer_to_src(rental_agreement.user.0, icp_amount - DEFAULT_FEE).await;

                if let Err(err) = transfer_to_src_result {
                    println!(
                        "{}: Transfer from user {} to SRC failed: {:?}",
                        subnet_id.0, rental_agreement.user.0, err
                    );
                    persist_event(
                        EventType::PaymentFailure {
                            reason: format!("{err:?}"),
                        },
                        subnet_id,
                    );
                    continue;
                }

                // Transfer ICP to CMC.
                let transfer_to_cmc_result =
                    transfer_to_cmc(icp_amount - DEFAULT_FEE - DEFAULT_FEE).await;
                let Ok(block_index) = transfer_to_cmc_result else {
                    // TODO: This should not happen.
                    let err = transfer_to_cmc_result.unwrap_err();
                    println!("Transfer from SRC to CMC failed: {:?}", err);
                    continue;
                };

                // Call notify_top_up to exchange ICP for cycles.
                let notify_top_up_result = notify_top_up(block_index).await;
                let Ok(actual_cycles) = notify_top_up_result else {
                    let err = notify_top_up_result.unwrap_err();
                    println!("Notify top-up failed: {:?}", err);
                    continue;
                };

                // Add cycles to billing record, update covered_until.
                let new_covered_until = covered_until + billing_period_nanos;
                BILLING_RECORDS.with(|map| {
                    let mut billing_record = map.borrow().get(&subnet_id).unwrap();
                    billing_record.covered_until = new_covered_until;
                    billing_record.cycles_balance += actual_cycles;
                    map.borrow_mut().insert(subnet_id, billing_record);
                });

                println!("Now covered until {}", new_covered_until);
                persist_event(
                    EventType::PaymentSuccess {
                        amount: icp_amount,
                        covered_until: new_covered_until,
                    },
                    subnet_id,
                );
            } else {
                // Next billing period is still fully covered.
                println!("Subnet is covered until {}, now is {}", covered_until, now);
            }
        }
    }
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
