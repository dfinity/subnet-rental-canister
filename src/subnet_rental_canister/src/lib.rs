use candid::{CandidType, Decode, Deserialize, Encode};
use external_types::NotifyError;
use history::{Event, History};
use ic_cdk::{heartbeat, init, post_upgrade, println, query, update};
use ic_ledger_types::{
    account_balance, transfer, AccountBalanceArgs, AccountIdentifier, Memo, Subaccount, Tokens,
    TransferArgs, TransferError, DEFAULT_FEE, DEFAULT_SUBACCOUNT,
    MAINNET_CYCLES_MINTING_CANISTER_ID, MAINNET_GOVERNANCE_CANISTER_ID, MAINNET_LEDGER_CANISTER_ID,
};
use ic_stable_structures::Memory;
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    DefaultMemoryImpl, StableBTreeMap, Storable,
};
use serde::Serialize;
use std::{borrow::Cow, cell::RefCell, collections::HashMap, time::Duration};

use crate::external_types::{
    Account, IcpXdrConversionRate, IcpXdrConversionRateResponse, NotifyTopUpArg,
    SetAuthorizedSubnetworkListArgs, TransferFromArgs, TransferFromError,
};
use crate::history::EventType;

pub mod external_types;
pub mod history;
mod http_request;

// During billing, the cost in cycles is fixed, but the cost in ICP depends on the exchange rate
const TRILLION: u128 = 1_000_000_000_000;
const E8S: u64 = 100_000_000;
const BILLING_INTERVAL: Duration = Duration::from_secs(60 * 60); // hourly
pub const MEMO_TOP_UP_CANISTER: Memo = Memo(0x50555054); // == 'TPUP'

type SubnetId = Principal;

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    // Memory region 0
    static RENTAL_AGREEMENTS: RefCell<StableBTreeMap<Principal, RentalAgreement, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    // Memory region 1
    static RENTAL_ACCOUNTS: RefCell<StableBTreeMap<Principal, RentalAccount, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1)))));

    // Memory region 2
    static HISTORY: RefCell<StableBTreeMap<Principal, History, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2)))));


    /// Hardcoded subnets and their rental conditions. TODO: make this editable via proposal (method), not canister upgrade.
    static SUBNETS: RefCell<HashMap<Principal, RentalConditions>> = HashMap::from([
        (candid::Principal::from_text("bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe").unwrap().into(),
            RentalConditions {
                daily_cost_cycles: 1_000 * TRILLION,
                initial_rental_period_days: 365,
                billing_period_days: 30,
                warning_threshold_days: 60,
            },
        ),
        (candid::Principal::from_text("fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae").unwrap().into(),
            RentalConditions {
                daily_cost_cycles: 2_000 * TRILLION,
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
pub struct RentalAgreement {
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

#[derive(Debug, Clone, Copy, CandidType, Deserialize)]
pub struct RentalAccount {
    /// The date (in nanos since epoch) until which the rental agreement is paid for.
    pub covered_until: u64,
    /// This account's share of cycles among the SRC's cycles.
    /// Increased by the payment process (via timer).
    /// Decreased by the burning process (via heartbeat).
    pub cycles_balance: u128,
    /// The last point in time (nanos since epoch) when cycles were burned in a heartbeat.
    pub last_burned: u64,
}

impl Storable for RentalAccount {
    // should be bounded once we replace string with real type
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

#[init]
fn init() {
    ic_cdk_timers::set_timer_interval(BILLING_INTERVAL, || ic_cdk::spawn(billing()));
    println!("Subnet rental canister initialized");
}

#[post_upgrade]
fn post_upgrade() {
    ic_cdk_timers::set_timer_interval(BILLING_INTERVAL, || ic_cdk::spawn(billing()));
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
    let creation_date = ic_cdk::api::time();
    let initial_rental_period_days = 365;
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(
            subnet_id,
            RentalAgreement {
                user: renter,
                subnet_id,
                principals: vec![renter, user],
                rental_conditions: RentalConditions {
                    daily_cost_cycles: 1_000 * TRILLION,
                    initial_rental_period_days,
                    billing_period_days: 30,
                    warning_threshold_days: 60,
                },
                creation_date,
            },
        )
    });
    let test_exchange_rate_e8s_cycles: u128 = 72_401; // 1 ICP = 7.2401T cycles
    let test_icp_balance_e8s: u64 = 50_500 * E8S; // just over one year covered
    RENTAL_ACCOUNTS.with(|map| {
        map.borrow_mut().insert(
            subnet_id,
            RentalAccount {
                covered_until: creation_date + days_to_nanos(initial_rental_period_days),
                cycles_balance: test_icp_balance_e8s as u128 * test_exchange_rate_e8s_cycles,
                last_burned: creation_date,
            },
        )
    });
}

#[query]
fn list_subnet_conditions() -> HashMap<SubnetId, RentalConditions> {
    SUBNETS.with(|map| map.borrow().clone())
}

#[query]
fn list_rental_agreements() -> Vec<RentalAgreement> {
    RENTAL_AGREEMENTS.with(|map| map.borrow().iter().map(|(_, v)| v).collect())
}

#[query]
fn get_rental_accounts() -> Vec<(Principal, RentalAccount)> {
    RENTAL_ACCOUNTS.with(|map| map.borrow().iter().collect())
}

#[derive(Clone, CandidType, Deserialize)]
pub struct ValidatedSubnetRentalProposal {
    pub subnet_id: candid::Principal,
    pub user: candid::Principal,
    pub principals: Vec<candid::Principal>,
    pub historical_exchange_rate_timestamp: u64,
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

async fn get_historical_exchange_rate_cycles_per_e8s(timestamp: u64) -> u64 {
    // TODO: implement
    println!(
        "Getting historical exchange rate for timestamp {}",
        timestamp
    );
    get_exchange_rate_cycles_per_e8s().await
}

/// TODO: Argument should be something like ValidatedSRProposal, created by governance canister via
/// SRProposal::validate().
#[update]
async fn accept_rental_agreement(
    ValidatedSubnetRentalProposal {
        subnet_id,
        user,
        principals,
        historical_exchange_rate_timestamp,
    }: ValidatedSubnetRentalProposal,
) -> Result<(), ExecuteProposalError> {
    verify_caller_is_governance()?;

    // Get rental conditions.
    // If the governance canister was able to validate, then this entry must exist, so we can unwrap.
    let rental_conditions = SUBNETS.with(|rc| *rc.borrow().get(&subnet_id.into()).unwrap());
    // Creation date in nanoseconds since epoch.
    let creation_date = ic_cdk::api::time();

    if RENTAL_AGREEMENTS.with(|map| map.borrow().contains_key(&subnet_id.into())) {
        println!(
            "Subnet is already in an active rental agreement: {:?}",
            &subnet_id
        );
        let err = ExecuteProposalError::SubnetAlreadyRented;
        persist_event(
            EventType::Failed {
                user: user.into(),
                reason: err.clone(),
            }
            .into(),
            subnet_id.into(),
        );
        return Err(err);
    }

    // Check if the user has enough ICP to cover the initial rental period.
    let needed_cycles = rental_conditions.daily_cost_cycles
        * (rental_conditions.initial_rental_period_days as u128);

    let exchange_rate =
        get_historical_exchange_rate_cycles_per_e8s(historical_exchange_rate_timestamp).await;
    let needed_icp = Tokens::from_e8s((needed_cycles / (exchange_rate as u128)) as u64);

    let icp_balance = check_balance(&user).await;
    if icp_balance < needed_icp {
        println!("Insufficient ICP balance to cover cost for initial rental period");
        return Err(ExecuteProposalError::InsufficientFunds);
    }

    // Use ICRC2 to transfer ICP from user to SRC.
    let transfer_to_src_result = ic_cdk::call::<_, (Result<u128, TransferFromError>,)>(
        MAINNET_LEDGER_CANISTER_ID,
        "icrc2_transfer_from",
        (TransferFromArgs {
            to: Account {
                // owner: MAINNET_CYCLES_MINTING_CANISTER_ID,
                // subaccount: Some(Subaccount::from(ic_cdk::id())),
                owner: ic_cdk::id(),
                subaccount: None,
            },
            fee: Some(DEFAULT_FEE.e8s() as u128),
            spender_subaccount: None,
            from: Account {
                owner: user,
                subaccount: None,
            },
            memo: Some(MEMO_TOP_UP_CANISTER), // For some reason, the CMC does not see this memo if we send it directly to the CMC with the icrc2_transfer_from; it arrives with memo 0.
            // Therefore, we send it to the SRC first (this canister), and then send it to the CMC with a normal transfer (non-icrc2).
            created_at_time: None,
            amount: (needed_icp - DEFAULT_FEE).e8s() as u128,
        },),
    )
    .await
    .expect("Failed to call ledger canister")
    .0;

    if let Err(err) = transfer_to_src_result {
        println!("Transfer from user to SRC failed: {:?}", err);
        persist_event(
            EventType::Failed {
                user: user.into(),
                reason: ExecuteProposalError::TransferUserToSrcError(err.clone()),
            }
            .into(),
            subnet_id.into(),
        );
        return Err(ExecuteProposalError::TransferUserToSrcError(err));
    }

    // Create preliminary rental agreement.
    let rental_agreement = RentalAgreement {
        user: user.into(),
        subnet_id: subnet_id.into(),
        principals: principals.into_iter().map(|p| p.into()).collect(),
        rental_conditions,
        creation_date,
    };

    // Whitelist principals for subnet
    for user in &rental_agreement.principals {
        // TODO: what about duplicates in rental_agreement.principals?
        // TODO: what about duplicates in rental_agreement.principals and existing principals in the list?
        ic_cdk::call::<_, ()>(
            MAINNET_CYCLES_MINTING_CANISTER_ID,
            "set_authorized_subnetwork_list",
            (SetAuthorizedSubnetworkListArgs {
                who: Some(user.0),
                subnets: vec![subnet_id],
            },),
        )
        .await
        .expect("Failed to call CMC");
    }

    // Use normal transfer to send the ICP from SRC to the CMC.
    let transfer_to_cmc_result = transfer(
        MAINNET_LEDGER_CANISTER_ID,
        TransferArgs {
            to: AccountIdentifier::new(
                &MAINNET_CYCLES_MINTING_CANISTER_ID,
                &Subaccount::from(ic_cdk::id()),
            ),
            fee: DEFAULT_FEE,
            from_subaccount: None,
            amount: needed_icp - DEFAULT_FEE - DEFAULT_FEE,
            memo: MEMO_TOP_UP_CANISTER,
            created_at_time: None,
        },
    )
    .await
    .expect("Failed to call ledger canister");

    let Ok(block_index) = transfer_to_cmc_result else {
        let err = transfer_to_cmc_result.unwrap_err();
        println!("Transfer from SRC to CMC failed: {:?}", err);
        persist_event(
            EventType::Failed {
                user: user.into(),
                reason: ExecuteProposalError::TransferSrcToCmcError(err.clone()),
            }
            .into(),
            subnet_id.into(),
        );
        return Err(ExecuteProposalError::TransferSrcToCmcError(err));
    };

    // Notify CMC about the top-up. This is what triggers the exchange from ICP to cycles.
    let notify_top_up_result = ic_cdk::call::<_, (Result<u128, NotifyError>,)>(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        "notify_top_up",
        (NotifyTopUpArg {
            block_index,
            canister_id: ic_cdk::id(),
        },),
    )
    .await
    .expect("Failed to call CMC")
    .0;

    // TODO: This might now be slightly less than the requested amount, due to price fluctuations since the check above.
    let Ok(actual_cycles) = notify_top_up_result else {
        let err = notify_top_up_result.unwrap_err();
        println!("Notify top-up failed: {:?}", err);
        persist_event(
            EventType::Failed {
                user: user.into(),
                reason: ExecuteProposalError::NotifyTopUpError(err.clone()),
            }
            .into(),
            subnet_id.into(),
        );
        return Err(ExecuteProposalError::NotifyTopUpError(err));
    };

    // Add the rental agreement to the rental agreement map.
    println!("Creating rental agreement: {:?}", &rental_agreement);
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut()
            .insert(subnet_id.into(), rental_agreement.clone());
    });

    // Add the rental account to the rental account map.
    let rental_account = RentalAccount {
        covered_until: creation_date + days_to_nanos(rental_conditions.initial_rental_period_days),
        cycles_balance: actual_cycles, // TODO: what about remaining cycles? what if this rental account already exists?
        last_burned: creation_date,
    };
    println!("Creating rental account: {:?}", &rental_account);
    RENTAL_ACCOUNTS.with(|map| map.borrow_mut().insert(subnet_id.into(), rental_account));

    persist_event(
        EventType::Created { rental_agreement }.into(),
        subnet_id.into(),
    );

    Ok(())
}

async fn billing() {
    let exchange_rate_cycles_per_e8s = get_exchange_rate_cycles_per_e8s().await;

    for (subnet_id, rental_agreement) in
        RENTAL_AGREEMENTS.with(|map| map.borrow().iter().collect::<Vec<_>>())
    {
        {
            let Some(RentalAccount { covered_until, .. }) =
                RENTAL_ACCOUNTS.with(|map| map.borrow_mut().get(&subnet_id))
            else {
                println!(
                    "FATAL: No rental account found for active rental agreement {:?}",
                    &subnet_id
                );
                continue;
            };

            // Check if subnet is covered for next billing_period amount of days.
            let now = ic_cdk::api::time();
            let billing_period_nanos =
                days_to_nanos(rental_agreement.rental_conditions.billing_period_days);

            if covered_until < now + billing_period_nanos {
                // Next billing period is not fully covered anymore.
                // Try to withdraw ICP and convert to cycles.
                let icp_balance = check_balance(&rental_agreement.user.0).await;

                let needed_cycles = rental_agreement.rental_conditions.daily_cost_cycles
                    * rental_agreement.rental_conditions.billing_period_days as u128;
                let cycles_available_to_mint =
                    (icp_balance.e8s() as u128) * (exchange_rate_cycles_per_e8s as u128);

                if cycles_available_to_mint < needed_cycles {
                    println!("Insufficient ICP balance to cover cost for next billing period");
                    // TODO: issue WARNING event
                    continue;
                }

                // TODO: do exchange with CMC and get actual amount of cycles
                // TODO: This might now be slightly less than the requested amount, due to price fluctuations since the check above.
                let actual_cycles = needed_cycles;

                // Add cycles to rental account, update covered_until.
                RENTAL_ACCOUNTS.with(|map| {
                    let mut rental_account = map.borrow().get(&subnet_id).unwrap();
                    rental_account.covered_until += billing_period_nanos;
                    rental_account.cycles_balance += actual_cycles;
                    map.borrow_mut().insert(subnet_id, rental_account);
                });
                println!("Now covered until {}", covered_until);

                persist_event(
                    EventType::PaymentSuccess {
                        amount: 0,
                        covered_until,
                    }
                    .into(),
                    subnet_id,
                );
            } else {
                // Next billing period is still fully covered.
                println!("Subnet is covered until {} now is {}", covered_until, now);
            }
        }
    }
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
    .expect("Failed to call CMC")
    .0;

    xdr_permyriad_per_icp
}

async fn check_balance(owner: &candid::Principal) -> Tokens {
    account_balance(
        MAINNET_LEDGER_CANISTER_ID,
        AccountBalanceArgs {
            account: AccountIdentifier::new(
                owner,
                &DEFAULT_SUBACCOUNT, // TODO: subaccounts of users?
            ),
        },
    )
    .await
    .expect("Failed to call ledger canister")
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

fn persist_event(event: Event, subnet: Principal) {
    HISTORY.with(|map| {
        let mut history = map.borrow().get(&subnet).unwrap_or_default();
        history.events.push(event);
        map.borrow_mut().insert(subnet, history);
    })
}

#[query]
fn get_history(subnet: candid::Principal) -> Option<Vec<Event>> {
    HISTORY.with(|map| {
        map.borrow()
            .get(&subnet.into())
            .map(|history| history.events)
    })
}

#[heartbeat]
fn canister_heartbeat() {
    RENTAL_ACCOUNTS.with(|map| {
        update_map(map, |subnet_id, account| {
            let Some(rental_agreement) = RENTAL_AGREEMENTS.with(|map| map.borrow().get(&subnet_id))
            else {
                println!(
                    "Fatal: Failed to find active rental agreement for active rental account {:?}",
                    subnet_id
                );
                return account;
            };
            let cost_cycles_per_second =
                rental_agreement.rental_conditions.daily_cost_cycles / 86400;
            let now = ic_cdk::api::time();
            let nanos_since_last_burn = now - account.last_burned;
            // cost_cycles_per_second: ~10^10 < 10^12
            // nanos_since_last_burn:  ~10^9  < 10^15
            // product                        < 10^27 << 10^38 (u128_max)
            // divided by 1B                    10^-9
            // amount                         < 10^18
            let amount = cost_cycles_per_second * nanos_since_last_burn as u128 / 1_000_000_000;
            if account.cycles_balance < amount {
                println!("Failed to burn cycles for agreement {:?}", subnet_id);
                return account;
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
            let cycles_balance = account.cycles_balance - amount;
            let last_burned = now;
            println!(
                "Burned {} cycles for agreement {:?}, remaining: {}",
                amount, subnet_id, cycles_balance
            );
            RentalAccount {
                covered_until: account.covered_until,
                cycles_balance,
                last_burned,
            }
        });
    });
}
