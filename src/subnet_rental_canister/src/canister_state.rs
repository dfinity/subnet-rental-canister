/// The canister state is modelled using stable structures, so that upgrade safety is guaranteed.
///
/// Since the SRC handles an arbitrary number of rented subnets, we associate the subnet_id with
/// the state structs by using StableBTreeMaps.
///
/// Relevant updates to state leave a trace in the corresponding History trace log.  
use crate::{
    history::{Event, EventType},
    Principal, RentalAgreement, RentalConditionId, RentalConditions, RentalRequest,
};
use ic_cdk::println;
use ic_ledger_types::Tokens;
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    DefaultMemoryImpl, StableBTreeMap,
};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
};

thread_local! {

    static RENTAL_CONDITIONS: RefCell<HashMap<RentalConditionId, RentalConditions>> =
        RefCell::new(HashMap::new());

    static LOCKS: RefCell<Locks> = const {RefCell::new(Locks{ids: BTreeSet::new()}) };

    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    // Memory region 0
    /// The keys are user principals, because a subnet_id might not be known at request time. Furthermore, only one active
    /// request is allowed per user principal.
    static RENTAL_REQUESTS: RefCell<StableBTreeMap<Principal, RentalRequest, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    // Memory region 1
    // Keys are subnet_ids
    static RENTAL_AGREEMENTS: RefCell<StableBTreeMap<Principal, RentalAgreement, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1)))));

    // Memory region 2
    // Keys are subnet_id, user principal or None for changes to rental conditions.
    #[allow(clippy::type_complexity)]
    static HISTORY: RefCell<StableBTreeMap<(Option<Principal>, Event), (), VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2)))));

    // Memory region 3
    // Cache for ICP/XDR exchange rates.
    // The keys are timestamps in seconds since epoch (rounded to midnight).
    // The values are (rate, decimal) where the rate is scaled by 10^decimals.
    static RATES: RefCell<StableBTreeMap<u64, (u64, u32), VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(3)))));
}

struct Locks {
    pub ids: BTreeSet<(Principal, &'static str)>,
}

/// A way to acquire locks for a principal before entering a critical section.
/// The str is simply a cheap tag in case different types of locks are needed
/// for the same principal.
pub struct CallerGuard {
    id: (Principal, &'static str),
}

impl CallerGuard {
    pub fn new(principal: Principal, tag: &'static str) -> Result<Self, String> {
        let id = (principal, tag);
        LOCKS.with_borrow_mut(|locks| {
            let held_locks = &mut locks.ids;
            if held_locks.contains(&id) {
                return Err("Failed to acquire lock".to_string());
            }
            held_locks.insert(id);
            Ok(Self { id })
        })
    }
}

impl Drop for CallerGuard {
    fn drop(&mut self) {
        LOCKS.with_borrow_mut(|locks| locks.ids.remove(&self.id));
    }
}

// ====================================================================================================================

pub fn get_rental_conditions(key: RentalConditionId) -> Option<RentalConditions> {
    RENTAL_CONDITIONS.with_borrow(|map| map.get(&key).cloned())
}

pub fn insert_rental_condition(key: RentalConditionId, value: RentalConditions) {
    RENTAL_CONDITIONS.with_borrow_mut(|map| map.insert(key, value));
}

pub fn iter_rental_conditions() -> Vec<(RentalConditionId, RentalConditions)> {
    RENTAL_CONDITIONS.with_borrow(|map| map.iter().map(|(k, v)| (*k, v.clone())).collect())
}

pub fn get_rental_request(requester: &Principal) -> Option<RentalRequest> {
    RENTAL_REQUESTS.with_borrow(|map| map.get(requester))
}

/// Used to mutate an existing rental request.
pub fn update_rental_request(
    requester: Principal,
    f: impl FnOnce(RentalRequest) -> RentalRequest,
) -> Result<(), String> {
    RENTAL_REQUESTS.with_borrow_mut(|map| match map.get(&requester) {
        None => Err("Princial has no rental agreement.".to_string()),
        Some(value) => {
            map.insert(requester, f(value));
            Ok(())
        }
    })
}

pub fn remove_rental_request(requester: &Principal) -> Option<RentalRequest> {
    RENTAL_REQUESTS.with_borrow_mut(|map| map.remove(requester))
}

pub fn iter_rental_requests() -> Vec<(Principal, RentalRequest)> {
    RENTAL_REQUESTS.with_borrow(|map| map.iter().collect())
}

pub fn get_rental_agreement(subnet_id: &Principal) -> Option<RentalAgreement> {
    RENTAL_AGREEMENTS.with_borrow(|map| map.get(subnet_id))
}

pub fn iter_rental_agreements() -> Vec<(Principal, RentalAgreement)> {
    RENTAL_AGREEMENTS.with_borrow(|map| map.iter().collect())
}

pub fn get_cached_rate(time: u64) -> Option<(u64, u32)> {
    RATES.with_borrow(|map| map.get(&time))
}

pub fn cache_rate(time: u64, rate: u64, decimals: u32) {
    RATES.with_borrow_mut(|map| map.insert(time, (rate, decimals)));
}

pub fn persist_event(event: impl Into<Event>, key: Option<Principal>) {
    HISTORY.with_borrow_mut(|map| {
        let composite_key = (key, event.into());
        map.insert(composite_key, ());
    })
}

pub fn get_history(principal: Option<Principal>) -> Vec<Event> {
    let start = Event::_start_bound();
    let end = Event::_end_bound();
    let start = (principal, start);
    let end = (principal, end);
    // (Option<Principal>, u64, Event), range query
    // start sequence number with 1. draw new sequence number for each principal when creating an event...
    // keep sequence number in separate map? //
    // Map<Option<Principal>, SequenceNum> gives last seq num
    // Map<(Option<Principal>, SequenceNum), Event>

    HISTORY.with_borrow(|map| map.range(start..end).map(|((_, v), _)| v).collect())
}

/// Create a RentalRequest with the current time as create_date, insert into canister state
/// and persist the corresponding event.
pub fn create_rental_request(
    user: Principal,
    refundable_icp: Tokens,
    locked_amount_cycles: u128,
    initial_proposal_id: u64,
    rental_condition_id: RentalConditionId,
    last_locking_time: u64,
    lock_amount_icp: Tokens,
) -> Result<(), String> {
    let now = ic_cdk::api::time();
    let rental_request = RentalRequest {
        user,
        refundable_icp,
        locked_amount_cycles,
        initial_proposal_id,
        creation_date: now,
        rental_condition_id,
        last_locking_time,
        lock_amount_icp,
    };
    RENTAL_REQUESTS.with_borrow_mut(|requests| {
        if requests.contains_key(&user) {
            Err(format!(
                "Principal {} already has an active RentalRequest",
                &user
            ))
        } else {
            requests.insert(user, rental_request.clone());
            println!("Created rental request: {:?}", &rental_request);
            persist_event(
                EventType::RentalRequestCreated { rental_request },
                Some(user),
            );
            Ok(())
        }
    })
}

pub fn take_rental_request(user: Principal) -> Option<RentalRequest> {
    RENTAL_REQUESTS.with_borrow_mut(|requests| requests.remove(&user))
}

/// Create a RentalAgreement with the current time as creation_date, insert into canister state  
/// and create the corresponding event.
#[allow(clippy::too_many_arguments)]
pub fn create_rental_agreement(
    subnet_id: Principal,
    user: Principal,
    initial_proposal_id: u64,
    subnet_creation_proposal_id: Option<u64>,
    rental_condition_id: RentalConditionId,
    cycles_balance: u128,
) -> Result<(), String> {
    let now = ic_cdk::api::time();
    // unwrap safety: all rental_condition_id keys have a value
    // in the static global HashMap at compile time.
    let initial_rental_period_nanos = get_rental_conditions(rental_condition_id)
        .unwrap()
        .initial_rental_period_days
        * 86_400
        * 1_000_000_000;
    let rental_agreement = RentalAgreement {
        user,
        initial_proposal_id,
        subnet_creation_proposal_id,
        rental_condition_id,
        creation_date: now,
        covered_until: now + initial_rental_period_nanos,
        cycles_balance,
        last_burned: now,
    };
    RENTAL_AGREEMENTS.with_borrow_mut(|agreements| {
        if agreements.contains_key(&subnet_id) {
            Err(format!(
                "Subnet_id {:?} already has an active RentalAgreement",
                subnet_id
            ))
        } else {
            agreements.insert(subnet_id, rental_agreement.clone());
            println!("Created rental agreement: {:?}", &rental_agreement);
            persist_event(
                EventType::RentalAgreementCreated {
                    user,
                    initial_proposal_id,
                    subnet_creation_proposal_id,
                    rental_condition_id,
                },
                Some(subnet_id),
            );
            Ok(())
        }
    })
}
