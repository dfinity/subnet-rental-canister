use std::time::Duration;

use crate::canister_state::{
    self, create_rental_request, get_cached_rate, get_rental_agreement, get_rental_conditions,
    get_rental_request, insert_rental_condition, iter_rental_conditions, iter_rental_requests,
    remove_rental_request, update_rental_request, CallerGuard,
};
use crate::external_calls::{
    check_subaccount_balance, convert_icp_to_cycles, get_exchange_rate_xdr_per_icp_at_time,
    refund_user,
};
use crate::{
    canister_state::persist_event, history::EventType, EventPage, RentalConditionId,
    RentalConditions, TRILLION,
};
use crate::{
    ExecuteProposalError, PriceCalculationData, RentalRequest, SubnetRentalProposalPayload, BILLION,
};
use candid::Principal;
use ic_cdk::api::call::{reject, reply};
use ic_cdk::{init, post_upgrade, query};
use ic_cdk::{println, update};
use ic_ledger_types::{
    account_balance, transfer, AccountBalanceArgs, AccountIdentifier, Memo, Subaccount, Tokens,
    TransferArgs, DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_GOVERNANCE_CANISTER_ID,
    MAINNET_LEDGER_CANISTER_ID,
};

////////// CANISTER METHODS //////////

#[init]
fn init() {
    set_initial_conditions();
    println!("Subnet rental canister initialized");
    start_timers();
}

#[post_upgrade]
fn post_upgrade() {
    set_initial_conditions();
    start_timers();
}

/// Persist initial rental conditions in global map and history.
fn set_initial_conditions() {
    let initial_conditions = [(
        RentalConditionId::App13CH,
        RentalConditions {
            description: "All nodes must be in Switzerland.".to_string(),
            subnet_id: None,
            daily_cost_cycles: 835 * TRILLION,
            initial_rental_period_days: 180,
            billing_period_days: 30,
        },
    )];
    for (k, v) in initial_conditions.iter() {
        println!("Created initial rental condition {:?}: {:?}", k, v);
        insert_rental_condition(*k, v.clone());
        persist_event(
            EventType::RentalConditionsChanged {
                rental_condition_id: *k,
                rental_conditions: Some(v.clone()),
            },
            // Associate events that might belong to no subnet with None.
            None,
        );
    }
}

fn start_timers() {
    // ic_cdk_timers::set_timer_interval(BILLING_INTERVAL, || ic_cdk::spawn(billing()));

    // check if any ICP should be locked
    ic_cdk_timers::set_timer_interval(Duration::from_secs(60 * 60 * 24), || {
        ic_cdk::spawn(locking())
    });
}

async fn locking() {
    let now = ic_cdk::api::time();
    for rental_request in iter_rental_requests().into_iter().map(|(_, v)| v) {
        let RentalRequest {
            user,
            initial_cost_icp,
            refundable_icp,
            locked_amount_icp,
            locked_amount_cycles,
            initial_proposal_id,
            creation_date,
            rental_condition_id,
            last_locking_time,
        } = rental_request;
        if (now - last_locking_time) / BILLION / 60 / 60 / 24 >= 30 {
            let lock_amount_icp = Tokens::from_e8s(initial_cost_icp.e8s() / 10);
            // Only try to lock if we haven't already locked 100% or more.
            // Use multiplication result to account for rounding errors.
            if locked_amount_icp >= Tokens::from_e8s(lock_amount_icp.e8s() * 10) {
                println!("Rental request for {} is already fully locked.", user);
                continue;
            }
            println!(
                "SRC will lock {} ICP for rental request of user {}.",
                lock_amount_icp, user
            );
            let res = convert_icp_to_cycles(lock_amount_icp, Subaccount::from(user)).await;
            let Ok(locked_cycles) = res else {
                println!(
                    "Failed to convert ICP to cycles for rental request of user {}",
                    user
                );
                let e = res.unwrap_err();
                persist_event(
                    EventType::LockingFailure {
                        user,
                        reason: format!("{:?}", e),
                    },
                    Some(user),
                );
                continue;
            };
            println!("SRC gained {} cycles from the locked ICP.", locked_cycles);
            persist_event(
                EventType::LockingSuccess {
                    user,
                    amount: lock_amount_icp,
                    cycles: locked_cycles,
                },
                Some(user),
            );
            let refundable_icp = refundable_icp - lock_amount_icp;
            let locked_amount_icp = locked_amount_icp + lock_amount_icp;
            let locked_amount_cycles = locked_amount_cycles + locked_cycles;
            let new_rental_request = RentalRequest {
                user,
                initial_cost_icp,
                refundable_icp,
                locked_amount_icp,
                locked_amount_cycles,
                initial_proposal_id,
                creation_date,
                rental_condition_id,
                // we risk not accounting for a few days in case this function does not run as scheduled
                last_locking_time: now,
            };
            update_rental_request(user, move |_| new_rental_request).unwrap();
        }
    }
}

////////// QUERY METHODS //////////

#[query]
pub fn list_rental_conditions() -> Vec<(RentalConditionId, RentalConditions)> {
    iter_rental_conditions()
}

#[query]
pub fn list_rental_requests() -> Vec<RentalRequest> {
    iter_rental_requests()
        .into_iter()
        .map(|(_k, v)| v)
        .collect()
}

/// Get the first page (the most recent) of events associated with the provided principal by
/// passing `older_than: None`.
/// The principal should be a user/tenant or a subnet id.
/// Returns both a vector of events and a token to provide in a subsequent call to this method
/// to retrieve the page before by passing `older_than: Some(continuation)`.
#[query]
pub fn get_history_page(principal: Principal, older_than: Option<u64>) -> EventPage {
    let page_size = 20;
    let (events, continuation) =
        canister_state::get_history_page(Some(principal), older_than, page_size);
    EventPage {
        events,
        continuation,
    }
}

/// Like `get_history_page` but for the changes in rental conditions.
#[query]
pub fn get_rental_conditions_history_page(older_than: Option<u64>) -> EventPage {
    let page_size = 20;
    let (events, continuation) = canister_state::get_history_page(None, older_than, page_size);
    EventPage {
        events,
        continuation,
    }
}

/// The future tenant may call this to derive the subaccount into which they must
/// transfer ICP.
#[query]
pub fn get_payment_subaccount() -> AccountIdentifier {
    AccountIdentifier::new(&ic_cdk::id(), &Subaccount::from(ic_cdk::caller()))
}

// #[query]
// pub fn list_rental_agreements() -> Vec<RentalAgreement> {
//     RENTAL_AGREEMENTS.with(|map| map.borrow().iter().map(|(_, v)| v).collect())
// }

////////// UPDATE METHODS //////////

/// Calculate the price of a subnet in ICP according to the exchange rate at the previous UTC midnight.
#[update]
pub async fn get_todays_price(id: RentalConditionId) -> Result<Tokens, String> {
    let Some(RentalConditions {
        description: _,
        subnet_id: _,
        daily_cost_cycles,
        initial_rental_period_days,
        billing_period_days: _,
    }) = get_rental_conditions(id)
    else {
        return Err("RentalConditionId not found".to_string());
    };
    let now_secs = ic_cdk::api::time() / BILLION;
    let prev_midnight = round_to_previous_midnight(now_secs);
    // Consult cache:
    let (scaled_exchange_rate_xdr_per_icp, decimals) = if let Some(tup) =
        get_cached_rate(prev_midnight)
    {
        tup
    } else {
        // Cache miss; only call the XRC if nobody else is currently making this call.
        let guard_res = CallerGuard::new(Principal::anonymous(), "XRC");
        if guard_res.is_err() {
            return Err(
                "Failed to acquire lock on calling exchange rate canister. Try again.".to_string(),
            );
        }
        // Call exchange rate canister.
        let res = get_exchange_rate_xdr_per_icp_at_time(prev_midnight).await;
        let Ok(tup) = res else {
            return Err(format!(
                "Failed to call the exchange rate canister: {:?}",
                res.unwrap_err()
            ));
        };
        drop(guard_res);
        tup
    };
    let res = calculate_subnet_price(
        daily_cost_cycles,
        initial_rental_period_days,
        scaled_exchange_rate_xdr_per_icp,
        decimals,
    );
    match res {
        Ok(tokens) => Ok(tokens),
        Err(e) => Err(format!("Failed to calculate price: {:?}", e)),
    }
}

// Used both for public endpoint `get_todays_price` and proposal execution.
fn calculate_subnet_price(
    daily_cost_cycles: u128,
    initial_rental_period_days: u64,
    scaled_exchange_rate_xdr_per_icp: u64,
    decimals: u32,
) -> Result<Tokens, ExecuteProposalError> {
    // ICP needed = cycles needed / cycles_per_icp
    //            = cycles needed / (scaled_rate_xdr_per_icp / 10^decimals)
    //            = cycles needed / ((scaled_rate_cycles_per_e8s * 10^4) / 10^decimals)
    //            = cycles needed * 10^decimals / scaled_rate
    // Factor of 10_000 = trillion / 1e8 to go from trillion cycles (XDR) to 10^-8 ICP (e8s).
    // The divisions are performed at the end so that accuracy is lost at the very end (if at all).
    let needed_cycles = daily_cost_cycles.checked_mul(initial_rental_period_days as u128);
    let e8s = needed_cycles
        .and_then(|x| x.checked_mul(u128::pow(10, decimals)))
        .and_then(|x| x.checked_div(scaled_exchange_rate_xdr_per_icp as u128))
        .and_then(|x| x.checked_div(10_000));
    let Some(e8s) = e8s else {
        return Err(ExecuteProposalError::PriceCalculationError(
            PriceCalculationData {
                daily_cost_cycles,
                initial_rental_period_days,
                scaled_exchange_rate_xdr_per_icp,
                decimals,
            },
        ));
    };
    let tokens = Tokens::from_e8s(e8s as u64);
    println!(
        "SRC requires {} cycles or {} ICP, according to exchange rate {}",
        needed_cycles.unwrap(), // Safe because we err out above.
        tokens,
        scaled_exchange_rate_xdr_per_icp as f64 / u64::pow(10, decimals) as f64
    );
    Ok(tokens)
}

#[update(manual_reply = true)]
pub async fn execute_rental_request_proposal(payload: SubnetRentalProposalPayload) {
    if let Err(e) = execute_rental_request_proposal_(payload).await {
        reject(&format!("Subnet rental request proposal failed: {:?}", e));
    } else {
        reply(((),));
    }
}

pub async fn execute_rental_request_proposal_(
    SubnetRentalProposalPayload {
        user,
        rental_condition_id,
        proposal_id,
        proposal_creation_time,
    }: SubnetRentalProposalPayload,
) -> Result<(), ExecuteProposalError> {
    /// This function makes sense locally only because EventType::RentalRequestFailed is fixed.
    fn with_error(
        user: Principal,
        proposal_id: u64,
        e: ExecuteProposalError,
    ) -> Result<(), ExecuteProposalError> {
        persist_event(
            EventType::RentalRequestFailed {
                user,
                proposal_id,
                reason: format!("{:?}", e.clone()),
            },
            Some(user),
        );
        Err(e)
    }
    verify_caller_is_governance()?;

    let _guard = CallerGuard::new(user, "rental_request").unwrap();
    // Fail if user has an existing rental request going on
    if get_rental_request(&user).is_some() {
        println!("Fatal: User already has an open SubnetRentalRequest waiting for completion.");
        let e = ExecuteProposalError::UserAlreadyRequestingSubnetRental;
        return with_error(user, proposal_id, e);
    }

    // Fail if user has an active rental agreement
    if get_rental_agreement(&user).is_some() {
        println!("Fatal: User already has an active rental agreement.");
        let e = ExecuteProposalError::UserAlreadyHasAgreement;
        return with_error(user, proposal_id, e);
    }

    // unwrap safety:
    // The rental_condition_id key must have a value in the rental conditions map due to `init` and `post_upgrade`.
    let RentalConditions {
        description: _,
        subnet_id,
        daily_cost_cycles,
        initial_rental_period_days,
        billing_period_days: _,
    } = get_rental_conditions(rental_condition_id).expect("Fatal: Rental conditions not found");

    // Fail if the provided subnet is already being rented:
    match subnet_id {
        None => {}
        Some(subnet_id) => {
            if get_rental_agreement(&subnet_id).is_some() {
                println!("Fatal: Subnet is already being rented.");
                let e = ExecuteProposalError::SubnetAlreadyRented;
                return with_error(user, proposal_id, e);
            }
        }
    }
    // Fail if the provided rental_condition_id (i.e., subnet) is already part of a pending rental request:
    for (_, rental_request) in iter_rental_requests().iter() {
        if rental_request.rental_condition_id == rental_condition_id {
            println!("Fatal: The given rental condition id is already part of a rental request.");
            let e = ExecuteProposalError::SubnetAlreadyRequested;
            return with_error(user, proposal_id, e);
        }
    }
    println!("Proceeding with rental request execution.");

    // ------------------------------------------------------------------
    // Attempt to transfer enough ICP to cover the initial rental period.
    // The XRC canister has a resolution of seconds, the SRC in nanos.
    let exchange_rate_query_time = round_to_previous_midnight(proposal_creation_time / BILLION);

    // Call exchange rate canister.
    let res = get_exchange_rate_xdr_per_icp_at_time(exchange_rate_query_time).await;
    let Ok((scaled_exchange_rate_xdr_per_icp, decimals)) = res else {
        let e = ExecuteProposalError::CallXRCFailed(format!("{:?}", res.unwrap_err()));
        return with_error(user, proposal_id, e);
    };

    let res = calculate_subnet_price(
        daily_cost_cycles,
        initial_rental_period_days,
        scaled_exchange_rate_xdr_per_icp,
        decimals,
    );
    let Ok(needed_icp) = res else {
        println!("Fatal: Failed to get exchange rate");
        let e = res.unwrap_err();
        return with_error(user, proposal_id, e);
    };

    // Check that the amount the user transferred to the SRC/user subaccount covers the initial cost.
    let available_icp = check_subaccount_balance(Subaccount::from(user)).await;
    println!(
        "Available icp: {}; Needed icp: {}",
        available_icp, needed_icp
    );
    if needed_icp > available_icp {
        println!("Fatal: Not enough ICP on the user subaccount to cover the initial period.");
        let e = ExecuteProposalError::InsufficientFunds {
            have: available_icp,
            need: needed_icp,
        };
        return with_error(user, proposal_id, e);
    }

    // Lock 10% by converting to cycles
    let lock_amount_icp = Tokens::from_e8s(needed_icp.e8s() / 10);
    println!(
        "SRC will lock 10% of the initial cost: {} ICP.",
        lock_amount_icp
    );

    let res = convert_icp_to_cycles(lock_amount_icp, Subaccount::from(user)).await;
    let Ok(locked_cycles) = res else {
        println!("Fatal: Failed to convert ICP to cycles");
        let e = res.unwrap_err();
        return with_error(user, proposal_id, e);
    };
    println!("SRC gained {} cycles from the locked ICP.", locked_cycles);
    let lock_time = ic_cdk::api::time();

    let refundable_icp = available_icp - lock_amount_icp;
    // unwrap safety: The user cannot have an open rental request, as ensured at the start of this function.
    create_rental_request(
        user,
        needed_icp,
        refundable_icp,
        lock_amount_icp,
        locked_cycles,
        proposal_id,
        rental_condition_id,
        lock_time,
    )
    .unwrap();
    println!("Created rental request for tenant {}", user);

    // Either proceed with existing subnet_id, or start polling for future subnet creation.
    if let Some(subnet_id) = subnet_id {
        println!("Reusing existing subnet {}", subnet_id);
        // TODO: Create rental agreement
    } else {
        // TODO: Start polling
    }

    Ok(())
}

/// Returns the block index of the refund transaction.
#[update]
pub async fn refund() -> Result<u64, String> {
    // Overall guard to prevent spamming the ledger canister.
    let overall_guard = CallerGuard::new(Principal::anonymous(), "refund");
    if overall_guard.is_err() {
        return Err("Only one refund may execute at a time. Try again".to_string());
    }
    let caller = ic_cdk::caller();
    // Before removing the rental request, acquire a lock on it, so that the
    // polling process cannot concurrently convert the request into a rental agreement.
    let guard_res = CallerGuard::new(caller, "rental_request");
    if guard_res.is_err() {
        return Err("Failed to acquire lock. Try again.".to_string());
    }
    // Does the caller have an active rental request?
    match get_rental_request(&caller) {
        Some(
            rental_request @ RentalRequest {
                user,
                initial_cost_icp: _,
                refundable_icp,
                locked_amount_icp: _,
                locked_amount_cycles,
                initial_proposal_id,
                creation_date: _,
                rental_condition_id: _,
                last_locking_time: _,
            },
        ) => {
            println!("Refund requested for user principal {}", &caller);
            // Refund the remaining ICP on the SRC/user subaccount to the user.
            let res = refund_user(user, refundable_icp - DEFAULT_FEE, initial_proposal_id).await;
            let Ok(block_id) = res else {
                return Err(format!(
                    "Failed to refund {} ICP to {}: {:?}",
                    refundable_icp - DEFAULT_FEE,
                    user,
                    res.unwrap_err()
                ));
            };
            println!(
                "SRC refunded {} ICP to {}, block_id: {}",
                refundable_icp - DEFAULT_FEE,
                user,
                block_id
            );
            ic_cdk::api::cycles_burn(locked_amount_cycles);
            println!(
                "SRC burned {} locked cycles after refunding.",
                locked_amount_cycles
            );
            // remove rental request from global map
            remove_rental_request(&caller);

            persist_event(
                EventType::RentalRequestCancelled {
                    rental_request: rental_request.clone(),
                },
                Some(user),
            );

            Ok(block_id)
        }
        None => {
            println!("Caller has no open rental request. Refunding all funds on the caller subaccount of the SRC.");
            let src_principal = ic_cdk::id();
            let res = account_balance(
                MAINNET_LEDGER_CANISTER_ID,
                AccountBalanceArgs {
                    account: AccountIdentifier::new(&src_principal, &Subaccount::from(caller)),
                },
            )
            .await;
            let Ok(balance) = res else {
                return Err(format!("Failed to refund: {:?}", res.unwrap_err()));
            };
            if balance < DEFAULT_FEE {
                return Err(format!(
                    "Failed refund: {} has insufficient funds {}",
                    caller, balance
                ));
            }
            let res = transfer(
                MAINNET_LEDGER_CANISTER_ID,
                TransferArgs {
                    to: AccountIdentifier::new(&caller, &DEFAULT_SUBACCOUNT),
                    fee: DEFAULT_FEE,
                    from_subaccount: Some(Subaccount::from(caller)),
                    amount: balance - DEFAULT_FEE,
                    memo: Memo(0),
                    created_at_time: None,
                },
            )
            .await
            .expect("Failed refund: Failed to call ledger canister");
            let Ok(block_id) = res else {
                return Err(format!(
                    "Failed to refund {} ICP to {}: {:?}",
                    balance - DEFAULT_FEE,
                    caller,
                    res.unwrap_err()
                ));
            };
            println!(
                "SRC refunded {} ICP to {}, block_id: {}",
                balance - DEFAULT_FEE,
                caller,
                block_id
            );
            Ok(block_id)
        }
    }
}

// ============================================================================
// Misc

fn verify_caller_is_governance() -> Result<(), ExecuteProposalError> {
    if ic_cdk::caller() != MAINNET_GOVERNANCE_CANISTER_ID {
        println!("Caller is not the governance canister");
        return Err(ExecuteProposalError::UnauthorizedCaller);
    }
    Ok(())
}

fn round_to_previous_midnight(time_secs: u64) -> u64 {
    time_secs - time_secs % 86400
}

// allow candid-extractor to derive candid interface from rust code
ic_cdk::export_candid!();
