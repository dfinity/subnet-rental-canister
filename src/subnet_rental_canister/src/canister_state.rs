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
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    DefaultMemoryImpl, StableBTreeMap,
};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
};

type EventNum = u64;

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
    // The current number of events for each principal. Helps with range queries on the history and with pagination.
    static EVENT_COUNTERS: RefCell<StableBTreeMap<Option<Principal>, EventNum, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2)))));

    // Memory region 3
    // Keys are subnet_id / user principal; or None for changes to rental conditions.
    #[allow(clippy::type_complexity)]
    static HISTORY: RefCell<StableBTreeMap<(Option<Principal>, EventNum), Event, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(3)))));

    // Memory region 4
    // Cache for ICP/XDR exchange rates.
    // The keys are timestamps in seconds since epoch (rounded to midnight).
    // The values are (rate, decimal) where the rate is scaled by 10^decimals.
    static RATES: RefCell<StableBTreeMap<u64, (u64, u32), VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(4)))));
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

pub fn get_rental_request(user: &Principal) -> Option<RentalRequest> {
    RENTAL_REQUESTS.with_borrow(|map| map.get(user))
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

/// Returns the rental agreement for the given subnet_id.
pub fn get_rental_agreement(subnet_id: &Principal) -> Option<RentalAgreement> {
    RENTAL_AGREEMENTS.with_borrow(|map| map.get(subnet_id))
}

pub fn iter_rental_agreements() -> Vec<(Principal, RentalAgreement)> {
    RENTAL_AGREEMENTS.with_borrow(|map| map.iter().collect())
}

pub fn update_rental_agreement(
    subnet_id: Principal,
    f: impl FnOnce(RentalAgreement) -> RentalAgreement,
) -> Result<(), String> {
    RENTAL_AGREEMENTS.with_borrow_mut(|map| match map.get(&subnet_id) {
        None => Err("Subnet_id has no rental agreement.".to_string()),
        Some(value) => {
            map.insert(subnet_id, f(value));
            Ok(())
        }
    })
}

pub fn get_cached_rate(time: u64) -> Option<(u64, u32)> {
    RATES.with_borrow(|map| map.get(&time))
}

pub fn cache_rate(time: u64, rate: u64, decimals: u32) {
    RATES.with_borrow_mut(|map| map.insert(time, (rate, decimals)));
}

/// Returns the next unused sequence number for the given principal and increases
/// the underlying counter. Starts at 0.
pub fn next_seq(mbp: Option<Principal>) -> EventNum {
    EVENT_COUNTERS.with_borrow_mut(|map| {
        let cur = map.get(&mbp).unwrap_or_default();
        map.insert(mbp, cur + 1);
        cur
    })
}

/// Returns the largest _used_ sequence number without increasing the underlying counter.
/// Returns None if no sequence number has been drawn for this principal yet.
pub fn get_current_seq(mbp: Option<Principal>) -> Option<EventNum> {
    EVENT_COUNTERS
        .with_borrow(|map| map.get(&mbp))
        .and_then(|x| {
            #[allow(clippy::let_and_return)]
            let y = x.checked_sub(1);
            #[cfg(test)]
            assert!(y.is_some());
            y
        })
}

pub fn persist_event(event: impl Into<Event>, key: Option<Principal>) {
    // get the next sequence number for this principal
    let seq = next_seq(key);
    HISTORY.with_borrow_mut(|map| {
        let composite_key = (key, seq);
        map.insert(composite_key, event.into());
    })
}

/// Returns a page of events for the given principal, and the event number of the oldest event in that page.
/// If older_than is None, the most recent page is returned.
/// Otherwise, the provided event number is just outside of (i.e., more recent than) the returned page,
/// all elements of which are older than the given value.
pub fn get_history_page(
    principal: Option<Principal>,
    older_than: Option<u64>,
    page_size: u64,
) -> (Vec<Event>, u64) {
    // User-provided value has priority. If not given, use the most recent event.
    // In that case, +1 for range end inclusion.
    let high_seq = older_than.unwrap_or_else(|| {
        get_current_seq(principal)
            .map(|x| x + 1)
            .unwrap_or_default()
    });
    let low_seq = high_seq.saturating_sub(page_size);
    let start = (principal, low_seq);
    let end = (principal, high_seq);
    let page = HISTORY.with_borrow(|map| map.range(start..end).map(|(_k, v)| v).collect());
    (page, low_seq)
}

/// Create a RentalRequest if it does not already exist, and persist the corresponding event.
pub fn persist_rental_request(rental_request: RentalRequest) -> Result<(), String> {
    RENTAL_REQUESTS.with_borrow_mut(|requests| {
        let user = rental_request.user;
        if requests.contains_key(&user) {
            return Err(format!("User {user} already has an active RentalRequest"));
        };
        requests.insert(user, rental_request.clone());
        println!("Created rental request: {:?}", &rental_request);
        persist_event(
            EventType::RentalRequestCreated { rental_request },
            Some(user),
        );
        Ok(())
    })
}

pub fn take_rental_request(user: Principal) -> Option<RentalRequest> {
    RENTAL_REQUESTS.with_borrow_mut(|requests| requests.remove(&user))
}

/// Create a RentalAgreement if it does not already exist, and persist the corresponding event.
pub fn persist_rental_agreement(rental_agreement: RentalAgreement) -> Result<(), String> {
    RENTAL_AGREEMENTS.with_borrow_mut(|agreements| {
        let subnet_id = rental_agreement.subnet_id;
        if agreements.contains_key(&subnet_id) {
            return Err(format!(
                "Subnet_id {subnet_id} already has an active RentalAgreement"
            ));
        }
        agreements.insert(subnet_id, rental_agreement.clone());
        println!("Created rental agreement: {:?}", &rental_agreement);
        persist_event(
            EventType::RentalAgreementCreated {
                user: rental_agreement.user,
                rental_request_proposal_id: rental_agreement.rental_request_proposal_id,
                subnet_creation_proposal_id: rental_agreement.subnet_creation_proposal_id,
                rental_condition_id: rental_agreement.rental_condition_id,
            },
            Some(subnet_id),
        );
        Ok(())
    })
}

#[cfg(test)]
mod canister_state_test {
    use super::*;
    use crate::history::EventType;
    use ic_ledger_types::Tokens;

    #[test]
    fn test_history_pagination() {
        fn make_event(time_nanos: u64) -> Event {
            Event::_mk_event(
                time_nanos,
                EventType::RentalRequestCreated {
                    rental_request: RentalRequest {
                        user: Principal::anonymous(),
                        initial_cost_icp: Tokens::from_e8s(100),
                        locked_amount_icp: Tokens::from_e8s(10),
                        locked_amount_cycles: 99,
                        initial_proposal_id: 99,
                        creation_time_nanos: time_nanos,
                        rental_condition_id: RentalConditionId::App13CH,
                        last_locking_time_nanos: 99,
                    },
                },
            )
        }
        persist_event(make_event(1), None);
        persist_event(make_event(2), None);
        persist_event(make_event(3), None);
        persist_event(make_event(4), None);
        persist_event(make_event(5), None);
        let (events, oldest) = get_history_page(None, None, 2);
        assert_eq!(events[0].time_nanos(), 4);
        assert_eq!(events[1].time_nanos(), 5);
        assert_eq!(events.len(), 2);
        let (events, oldest) = get_history_page(None, Some(oldest), 2);
        assert_eq!(events[0].time_nanos(), 2);
        assert_eq!(events[1].time_nanos(), 3);
        assert_eq!(events.len(), 2);
        let (events, oldest) = get_history_page(None, Some(oldest), 2);
        assert_eq!(events[0].time_nanos(), 1);
        assert_eq!(events.len(), 1);
        let (events, oldest) = get_history_page(None, Some(oldest), 2);
        assert!(events.is_empty());
        assert_eq!(oldest, 0);
        // also test empty history
        let (events, oldest) = get_history_page(Some(Principal::anonymous()), None, 2);
        assert!(events.is_empty());
        assert_eq!(oldest, 0);
        let (events, oldest) = get_history_page(Some(Principal::anonymous()), Some(3), 2);
        assert!(events.is_empty());
        assert_eq!(oldest, 1); // because 3 - 2 = 1
    }
}
