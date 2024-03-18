use std::cell::RefCell;

use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    DefaultMemoryImpl, Memory, StableBTreeMap, Storable,
};

use crate::{
    history::{Event, EventType, History},
    BillingRecord, Principal, RentalAgreement, RentalConditions,
};

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

/// Use only this function to make changes to RENTAL_CONDITIONS, so that
/// all changes are persisted in the history.
/// Internally used in canister_init, externally available as an update method
/// which only the governance canister can call, see set_rental_conditions().
pub fn set_rental_conditions(
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