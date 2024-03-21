/// The canister state is modelled using stable structures, so that upgrade safety is guaranteed.
///
/// Since the SRC handles an arbitrary number of rented subnets, we associate the subnet_id with
/// the state structs by using StableBTreeMaps.
///
/// Most updates to state should leave a trace in the corresponding History trace log.
///
use crate::{
    history::{Event, EventType, History},
    Principal, RentalAgreement, RentalConditionType, RentalConditions, RentalRequest,
    SubnetSpecification, APP13SWITZERLAND,
};
use ic_cdk::println;
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    DefaultMemoryImpl, StableBTreeMap,
};
use std::{cell::RefCell, collections::HashMap};

thread_local! {

    static RENTAL_CONDITIONS: RefCell<HashMap<RentalConditionType, RentalConditions>> =
        RefCell::new(HashMap::from([(RentalConditionType::App13Switzerland, APP13SWITZERLAND); 1]));

    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    // Memory region 0
    /// The keys are user principals, because a subnet_id is not known at request time. Furthermore, only one active
    /// request is allowed per user principal.
    static RENTAL_REQUESTS: RefCell<StableBTreeMap<Principal, RentalRequest, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    // Memory region 1
    static RENTAL_AGREEMENTS: RefCell<StableBTreeMap<Principal, RentalAgreement, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1)))));

    // Memory region 2
    static HISTORY: RefCell<StableBTreeMap<Principal, History, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2)))));

}

pub fn get_rental_conditions(key: RentalConditionType) -> Option<RentalConditions> {
    RENTAL_CONDITIONS.with_borrow(|map| map.get(&key).cloned())
}

pub fn iter_rental_conditions() -> Vec<(RentalConditionType, RentalConditions)> {
    RENTAL_CONDITIONS.with_borrow(|map| map.iter().map(|(k, v)| (*k, *v)).collect())
}

pub fn get_rental_request(requester: &Principal) -> Option<RentalRequest> {
    RENTAL_REQUESTS.with_borrow(|map| map.get(requester))
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

pub fn persist_event(event: impl Into<Event>, subnet: Principal) {
    HISTORY.with_borrow_mut(|map| {
        let mut history = map.get(&subnet).unwrap_or_default();
        history.events.push(event.into());
        map.insert(subnet, history);
    })
}

/// Create a RentalRequest with the current time as create_date, insert into canister state
/// and persist the corresponding event.
pub fn create_rental_request(
    user: Principal,
    locked_amount_cycles: u128,
    initial_proposal_id: u64,
    subnet_spec: SubnetSpecification,
    rental_condition_type: RentalConditionType,
) -> Result<(), String> {
    let now = ic_cdk::api::time();
    let rental_request = RentalRequest {
        user,
        locked_amount_cycles,
        initial_proposal_id,
        creation_date: now,
        subnet_spec,
        rental_condition_type,
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
            persist_event(EventType::RentalRequestCreated { rental_request }, user);
            Ok(())
        }
    })
}

pub fn take_rental_request(user: Principal) -> Option<RentalRequest> {
    RENTAL_REQUESTS.with_borrow_mut(|requests| requests.remove(&user))
}

/// Create a RentalAgreement with consistent timestamps, insert into canister state
/// and create the corresponding event.
#[allow(clippy::too_many_arguments)]
pub fn create_rental_agreement(
    subnet_id: Principal,
    user: Principal,
    initial_proposal_id: u64,
    subnet_creation_proposal_id: Option<u64>,
    subnet_spec: SubnetSpecification,
    rental_condition_type: RentalConditionType,
    covered_until: u64,
    cycles_balance: u128,
) -> Result<(), String> {
    let now = ic_cdk::api::time();
    let rental_agreement = RentalAgreement {
        user,
        initial_proposal_id,
        subnet_creation_proposal_id,
        subnet_spec: subnet_spec.clone(),
        rental_condition_type,
        creation_date: now,
        covered_until,
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
                    subnet_spec,
                    rental_condition_type,
                },
                subnet_id,
            );
            Ok(())
        }
    })
}

// Rental agreements have an associated BillingAccount, which must be removed at the same time.
// TODO: only call this if agreement exists...
// fn delete_rental_agreement(subnet_id: Principal) {
//     let rental_agreement =
//         RENTAL_AGREEMENTS.with(|map| map.borrow_mut().remove(&subnet_id).unwrap());
//     // let billing_record = BILLING_RECORDS.with(|map| map.borrow_mut().remove(&subnet_id).unwrap());
//     persist_event(
//         EventType::Terminated {
//             rental_agreement,
//             // billing_record,
//         },
//         subnet_id,
//     );
// }

// Pass one of the global StableBTreeMaps and a function that transforms a value.
// pub fn update_map<K, V, M>(map: &RefCell<StableBTreeMap<K, V, M>>, f: impl Fn(K, V) -> V)
// where
//     K: Storable + Ord + Clone,
//     V: Storable,
//     M: Memory,
// {
//     let keys: Vec<K> = map.borrow().iter().map(|(k, _v)| k).collect();
//     for key in keys {
//         let value = map.borrow().get(&key).unwrap();
//         map.borrow_mut().insert(key.clone(), f(key, value));
//     }
// }
