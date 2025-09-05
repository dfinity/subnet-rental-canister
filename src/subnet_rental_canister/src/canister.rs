use crate::{
    canister_state::{
        self, get_cached_rate, get_rental_agreement, get_rental_conditions, get_rental_request,
        insert_rental_condition, iter_rental_agreements, iter_rental_conditions,
        iter_rental_requests, persist_event, persist_rental_agreement, persist_rental_request,
        remove_rental_request, update_rental_agreement, update_rental_request, CallerGuard,
    },
    external_calls::{
        check_subaccount_balance, convert_icp_to_cycles, get_exchange_rate_icp_per_xdr_at_time,
        refund_user, set_authorized_subnetwork_list,
    },
    history::EventType,
    CreateRentalAgreementPayload, EventPage, ExecuteProposalError, PriceCalculationData,
    RentalAgreement, RentalAgreementStatus, RentalConditionId, RentalConditions, RentalRequest,
    SubnetRentalProposalPayload, TopUpSummary, BILLION, TRILLION,
};
use candid::Principal;
use ic_cdk::{
    api::{msg_caller, msg_reject, msg_reply},
    init, post_upgrade, println, query, update,
};
use ic_ledger_types::{
    AccountIdentifier, Subaccount, Tokens, DEFAULT_FEE, MAINNET_GOVERNANCE_CANISTER_ID,
};
use std::{cmp::min, time::Duration};

const CYCLES_BURN_INTERVAL_SECONDS: u64 = 60;
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

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
            description: "All nodes must be in Switzerland or Liechtenstein.".to_string(),
            subnet_id: None,
            daily_cost_cycles: 820 * TRILLION,
            initial_rental_period_days: 180,
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
    // Check if any ICP should be locked every hour.
    ic_cdk_timers::set_timer_interval(Duration::from_secs(60 * 60), || {
        ic_cdk::futures::spawn(locking())
    });

    // Burn cycles every minute.
    ic_cdk_timers::set_timer_interval(Duration::from_secs(CYCLES_BURN_INTERVAL_SECONDS), || {
        ic_cdk::futures::spawn(burn_cycles())
    });
}

async fn burn_cycles() {
    for rental_agreement in iter_rental_agreements().into_iter().map(|(_, v)| v) {
        let Ok(_guard_agreement) = CallerGuard::new(rental_agreement.subnet_id, "agreement") else {
            println!(
                "Busy processing another request. Skipping cycles burn for subnet {}",
                rental_agreement.subnet_id
            );
            continue;
        };

        // Because of the copy in the loop head, by the time we execute here, the rental agreement might not exist anymore.
        let Some(rental_agreement) = get_rental_agreement(&rental_agreement.subnet_id) else {
            println!(
                "Rental agreement for subnet {} no longer exists. Skipping cycles burn.",
                rental_agreement.subnet_id
            );
            continue;
        };

        // Now we are sure the rental agreement still exists and only we can modify it.

        let total_cycles_burned = rental_agreement.total_cycles_burned;
        let total_cycles_created = rental_agreement.total_cycles_created;
        let total_cycles_remaining = total_cycles_created.saturating_sub(total_cycles_burned);
        let paid_until_nanos = rental_agreement.paid_until_nanos;
        let now_nanos = ic_cdk::api::time();

        // No more cycles left to burn.
        if total_cycles_remaining == 0 {
            continue;
        }

        // The rental agreement is not paid for anymore.
        if now_nanos >= paid_until_nanos {
            // Burn all remaining cycles.
            println!(
                "Burning all remaining cycles for subnet {} (subnet is no longer paid for)",
                rental_agreement.subnet_id
            );
            let burned = ic_cdk::api::cycles_burn(total_cycles_remaining);
            if let Err(e) = update_rental_agreement(rental_agreement.subnet_id, |mut agreement| {
                agreement.total_cycles_burned =
                    agreement.total_cycles_burned.saturating_add(burned);
                agreement
            }) {
                println!(
                    "Failed to update rental agreement for subnet {}: {}. Skipping.",
                    rental_agreement.subnet_id, e
                );
            }
            continue;
        }

        // The rental agreement is still paid for and has some cycles left to burn.
        let time_remaining_nanos = paid_until_nanos - now_nanos; // guaranteed to be positive due to above check
        let time_remaining_seconds = time_remaining_nanos / BILLION;
        let amount_to_burn_per_minute = (total_cycles_remaining / time_remaining_seconds as u128)
            * CYCLES_BURN_INTERVAL_SECONDS as u128;

        let burned =
            ic_cdk::api::cycles_burn(min(amount_to_burn_per_minute, total_cycles_remaining)); // don't burn more than we have
        if let Err(e) = update_rental_agreement(rental_agreement.subnet_id, |mut agreement| {
            agreement.total_cycles_burned = agreement.total_cycles_burned.saturating_add(burned);
            agreement
        }) {
            println!(
                "Failed to update rental agreement for subnet {}: {}. Skipping.",
                rental_agreement.subnet_id, e
            );
            continue;
        }
    }
}

async fn locking() {
    let now_nanos = ic_cdk::api::time();
    for rental_request in iter_rental_requests().into_iter().map(|(_, v)| v) {
        let RentalRequest {
            user,
            initial_cost_icp,
            locked_amount_icp,
            locked_amount_cycles,
            initial_proposal_id,
            creation_time_nanos,
            rental_condition_id,
            last_locking_time_nanos,
        } = rental_request;

        let days_since_last_locking =
            (now_nanos - last_locking_time_nanos) / BILLION / 60 / 60 / 24;
        // If the last locking time is less than 30 days ago, skip.
        if days_since_last_locking < 30 {
            continue;
        }

        // The amount we want to lock is 10% of the initial cost
        let ten_percent_icp = Tokens::from_e8s(initial_cost_icp.e8s() / 10);

        // Only try to lock if we haven't already locked 100% or more.
        // Use multiplication result to account for rounding errors.
        if locked_amount_icp >= Tokens::from_e8s(ten_percent_icp.e8s() * 10) {
            println!("Rental request for {user} is already fully locked.");
            continue;
        }

        let Ok(_guard_request) = CallerGuard::new(user, "request") else {
            println!("Busy processing another request. Skipping.");
            continue;
        };

        // Convert ICP to cycles.
        let locked_cycles =
            match convert_icp_to_cycles(ten_percent_icp, Subaccount::from(user)).await {
                Ok(locked_cycles) => locked_cycles,
                Err(error) => {
                    println!("Failed to convert ICP to cycles for rental request of user {user}.");
                    persist_event(
                        EventType::LockingFailure {
                            user,
                            reason: format!("{error:?}"),
                        },
                        Some(user),
                    );
                    continue;
                }
            };

        println!("SRC gained {locked_cycles} cycles from the locked ICP.");
        persist_event(
            EventType::LockingSuccess {
                user,
                amount: ten_percent_icp,
                cycles: locked_cycles,
            },
            Some(user),
        );

        let locked_amount_icp = locked_amount_icp + ten_percent_icp;
        let locked_amount_cycles = locked_amount_cycles + locked_cycles;
        let new_rental_request = RentalRequest {
            user,
            initial_cost_icp,
            locked_amount_icp,
            locked_amount_cycles,
            initial_proposal_id,
            creation_time_nanos,
            rental_condition_id,
            // we risk not accounting for a few days in case this function does not run as scheduled
            last_locking_time_nanos: now_nanos,
        };
        update_rental_request(user, move |_| new_rental_request).unwrap();
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
/// The principal should be a user or a subnet id.
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

/// Derive the account into which a user must transfer ICP for renting a subnet.
#[query]
pub fn get_payment_account(user: Principal) -> String {
    AccountIdentifier::new(&ic_cdk::api::canister_self(), &Subaccount::from(user)).to_hex()
}

/// List all active rental agreements.
#[query]
pub fn list_rental_agreements() -> Vec<RentalAgreement> {
    iter_rental_agreements()
        .into_iter()
        .map(|(_k, v)| v)
        .collect()
}

/// Returns the status of a rental agreement w.r.t. payment coverage.
#[query]
pub fn rental_agreement_status(subnet_id: Principal) -> Result<RentalAgreementStatus, String> {
    let Some(rental_agreement) = get_rental_agreement(&subnet_id) else {
        return Err("Rental agreement not found".to_string());
    };
    let now_nanos = ic_cdk::api::time();
    let paid_until_nanos = rental_agreement.paid_until_nanos;

    let cycles_left = rental_agreement
        .total_cycles_created
        .saturating_sub(rental_agreement.total_cycles_burned);
    let days_left = calculate_days_remaining(paid_until_nanos, now_nanos);

    let description = if now_nanos > rental_agreement.paid_until_nanos {
        "PAST DUE: This rental agreement needs to be topped up to continue.".to_string()
    } else if days_left <= 30 {
        format!("WARNING: This rental agreement is only covered for {days_left} more days. Please top up the subnet.")
    } else {
        format!("OK: This rental agreement is covered for {days_left} more days.")
    };

    Ok(RentalAgreementStatus {
        description,
        cycles_left,
        days_left,
    })
}

////////// UPDATE METHODS //////////

/// Calculate the price of a subnet in ICP according to the exchange rate at the previous UTC midnight.
/// The first call per day will cost 1_000_000_000 cycles.
#[update]
pub async fn get_todays_price(id: RentalConditionId) -> Result<Tokens, String> {
    let Some(conditions) = get_rental_conditions(id) else {
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
        let res = get_exchange_rate_icp_per_xdr_at_time(prev_midnight).await;
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
        conditions.daily_cost_cycles,
        conditions.initial_rental_period_days,
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
    // Note: The NNS Governance canister makes sure that there is only one non-terminal subnet rental request proposal
    // at a time, effectively preventing concurrent calls to this method.
    // See [here](https://github.com/dfinity/ic/blob/d8fb1363ef39bae56493a8e48907c76b50d914e6/rs/nns/governance/src/governance.rs#L5061).

    if let Err(e) = execute_rental_request_proposal_(payload).await {
        msg_reject(format!("Subnet rental request proposal failed: {:?}", e));
    } else {
        msg_reply(candid::encode_one(()).unwrap());
    }

    pub async fn execute_rental_request_proposal_(
        SubnetRentalProposalPayload {
            user,
            rental_condition_id,
            proposal_id,
            proposal_creation_time_seconds,
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

        // make sure no concurrent calls to this method can exist, in addition to governance's check.
        let _guard_request =
            CallerGuard::new(user, "request").expect("Fatal: Concurrent call on user");
        let _guard = CallerGuard::new(Principal::anonymous(), "execute_rental_request_proposal")
            .expect("Fatal: Concurrent call on execute_rental_request_proposal");

        // Fail if user has an existing rental request going on
        if get_rental_request(&user).is_some() {
            println!("Fatal: User already has an open SubnetRentalRequest waiting for completion.");
            let e = ExecuteProposalError::UserAlreadyRequestingSubnetRental;
            return with_error(user, proposal_id, e);
        }

        // Fail if user has an active rental agreement
        if iter_rental_agreements().iter().any(|(_, v)| v.user == user) {
            println!("Fatal: User already has an active rental agreement.");
            let e = ExecuteProposalError::UserAlreadyHasAgreement;
            return with_error(user, proposal_id, e);
        }

        // unwrap safety:
        // The rental_condition_id key must have a value in the rental conditions map due to `init` and `post_upgrade`.
        let RentalConditions {
            subnet_id,
            daily_cost_cycles,
            initial_rental_period_days,
            ..
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
                println!(
                    "Fatal: The given rental condition id is already part of a rental request."
                );
                let e = ExecuteProposalError::SubnetAlreadyRequested;
                return with_error(user, proposal_id, e);
            }
        }
        println!("Proceeding with rental request execution.");

        // ------------------------------------------------------------------
        // Attempt to transfer enough ICP to cover the initial rental period.
        // Proposal creation time passed by NNS Governance is in seconds.
        let exchange_rate_query_time = round_to_previous_midnight(proposal_creation_time_seconds);

        // Call exchange rate canister.
        let res = get_exchange_rate_icp_per_xdr_at_time(exchange_rate_query_time).await;
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

        let now_nanos = ic_cdk::api::time();
        let rental_request = RentalRequest {
            user,
            initial_cost_icp: needed_icp,
            locked_amount_icp: lock_amount_icp,
            locked_amount_cycles: locked_cycles,
            initial_proposal_id: proposal_id,
            creation_time_nanos: now_nanos,
            rental_condition_id,
            last_locking_time_nanos: now_nanos,
        };

        // unwrap safety: The user cannot have an open rental request, as ensured at the start of this function.
        persist_rental_request(rental_request).unwrap();
        println!("Created rental request for user {}", &user);

        Ok(())
    }
}

/// This function is called by the NNS Governance canister to create a rental agreement from an existing rental request.
/// 1. The remaining ICP is converted to cycles and added to the total cycles created.
/// 2. The user is whitelisted on the CMC.
/// 3. The rental request is removed, which will terminate the monthly locking process.
/// 4. The rental agreement is persisted.
#[update(manual_reply = true)]
pub async fn execute_create_rental_agreement(payload: CreateRentalAgreementPayload) {
    if let Err(e) = execute_create_rental_agreement_(payload).await {
        msg_reject(format!("Creating rental agreement failed: {:?}", e));
    } else {
        msg_reply(candid::encode_one(()).unwrap());
    }

    pub async fn execute_create_rental_agreement_(
        payload: CreateRentalAgreementPayload,
    ) -> Result<(), ExecuteProposalError> {
        verify_caller_is_governance()?;
        let _guard_user =
            CallerGuard::new(payload.user, "request").expect("Fatal: Concurrent call on user");
        let _guard_subnet =
            CallerGuard::new(payload.subnet_id, "nns").expect("Fatal: Concurrent call on subnet");

        // Check if the user has an active rental request.
        let Some(rental_request) = get_rental_request(&payload.user) else {
            return Err(ExecuteProposalError::RentalRequestNotFound);
        };

        // Fail if the subnet is already being rented.
        if get_rental_agreement(&payload.subnet_id).is_some() {
            return Err(ExecuteProposalError::SubnetAlreadyRented);
        }

        let rental_condition = get_rental_conditions(rental_request.rental_condition_id)
            .expect("Fatal: Rental condition not found");
        let initial_rental_period_nanos =
            rental_condition.initial_rental_period_days * SECONDS_PER_DAY * BILLION;

        // Convert all remaining ICP to cycles.
        let remaining_icp = rental_request.initial_cost_icp - rental_request.locked_amount_icp;
        let converted_cycles =
            convert_icp_to_cycles(remaining_icp, Subaccount::from(payload.user)).await?;
        let total_cycles_created =
            converted_cycles.saturating_add(rental_request.locked_amount_cycles);

        // Create the rental agreement.
        let now_nanos = ic_cdk::api::time();
        let rental_agreement = RentalAgreement {
            user: payload.user,
            subnet_id: payload.subnet_id,
            rental_request_proposal_id: rental_request.initial_proposal_id,
            subnet_creation_proposal_id: Some(payload.proposal_id),
            rental_condition_id: rental_request.rental_condition_id,
            creation_time_nanos: now_nanos,
            paid_until_nanos: now_nanos + initial_rental_period_nanos,
            total_icp_paid: rental_request.initial_cost_icp,
            total_cycles_created,
            total_cycles_burned: 0,
        };

        set_authorized_subnetwork_list(&payload.user, &payload.subnet_id).await;

        // Removing the rental request will also stop the monthly locking process which locks 10% of the initial cost.
        remove_rental_request(&payload.user).unwrap(); // It is checked above that the user has a rental request.

        persist_rental_agreement(rental_agreement).unwrap(); // It is checked above that the subnet is not being rented.

        Ok(())
    }
}

/// If the calling user has a rental request, the rental request will be deleted,
/// the locked cycles will be burned, and the user will be refunded the remaining ICP.
/// If the calling user has no rental request or an active rental agreement,
/// the SRC will refund the entire balance on the caller subaccount.
/// Returns the block index of the refund transaction.
#[update]
pub async fn refund() -> Result<u64, String> {
    let caller = msg_caller();
    // To not flood the ledger canister, we only do one refund at a time.
    let Ok(_guard_refund) = CallerGuard::new(Principal::anonymous(), "refund") else {
        return Err("Busy processing another request. Try again.".to_string());
    };
    // We might remove a rental request, so we need to acquire a lock on it.
    let Ok(_guard_request) = CallerGuard::new(caller, "request") else {
        return Err("Busy processing another request. Try again.".to_string());
    };

    let balance = check_subaccount_balance(Subaccount::from(caller)).await;
    if balance < DEFAULT_FEE {
        return Err(format!(
            "Failed refund: {caller} has insufficient funds {balance}"
        ));
    }
    let to_be_refunded = balance - DEFAULT_FEE;

    let block_id = refund_user(caller, to_be_refunded).await.map_err(|e| {
        format!(
            "Failed to refund {} ICP to {}: {:?}",
            to_be_refunded, caller, e
        )
    })?;

    // If the user has a rental request, burn the locked cycles and remove the request.
    if let Some(rental_request) = get_rental_request(&caller) {
        ic_cdk::api::cycles_burn(rental_request.locked_amount_cycles);
        println!(
            "Burned {} locked cycles after refunding",
            rental_request.locked_amount_cycles
        );
        remove_rental_request(&caller).unwrap(); // Safe because we checked above that the user has a rental request.
        persist_event(
            EventType::RentalRequestCancelled { rental_request },
            Some(caller),
        );
    };

    println!(
        "SRC refunded {} ICP to {}, block_id: {}",
        to_be_refunded, caller, block_id
    );
    Ok(block_id)
}

/// Estimates how many cycles and days a given ICP amount would provide for a subnet rental.
/// Uses the (potentially cached) exchange rate from the previous midnight to calculate the conversion.
#[update]
pub async fn subnet_top_up_estimate(
    subnet_id: Principal,
    icp: Tokens,
) -> Result<TopUpSummary, String> {
    let Some(rental_agreement) = get_rental_agreement(&subnet_id) else {
        return Err("Rental agreement not found".to_string());
    };

    let now_secs = ic_cdk::api::time() / BILLION;
    let prev_midnight = round_to_previous_midnight(now_secs);

    let Ok((scaled_exchange_rate_xdr_per_icp, decimals)) =
        get_exchange_rate_icp_per_xdr_at_time(prev_midnight).await
    else {
        return Err("Failed to get exchange rate".to_string());
    };

    if icp < DEFAULT_FEE {
        return Err(format!(
            "Must top up more than {} ICP to cover the default fee",
            DEFAULT_FEE
        ));
    }

    let to_be_topped_up = icp - DEFAULT_FEE; // User pays the default fee to send funds to the SRC.

    let estimated_cycles = (to_be_topped_up - DEFAULT_FEE).e8s() as u128 // Account for internal transfer to CMC cost.
        * scaled_exchange_rate_xdr_per_icp as u128
        / (u128::pow(10, decimals) / 10_000); // Factor 10_000 to go from trillion cycles (10^12) to e8s (10^8).

    let rental_condition = get_rental_conditions(rental_agreement.rental_condition_id)
        .ok_or("Rental condition not found")?;

    let estimated_days = (estimated_cycles / rental_condition.daily_cost_cycles) as u64;

    let description = format!(
        "Estimate: {} ICP would provide approximately {} cycles, \
        extending the rental for subnet {} for about {} days. \
        Note that this is an estimate and the actual amount will vary.",
        icp, estimated_cycles, subnet_id, estimated_days
    );

    Ok(TopUpSummary {
        description,
        cycles_added: estimated_cycles,
        days_added: estimated_days,
    })
}

/// Callable by anyone to trigger the conversion of ICP to cycles and the extension of the rental agreement.
#[update]
pub async fn top_up_subnet(subnet_id: Principal) -> Result<TopUpSummary, String> {
    let Ok(_guard_agreement) = CallerGuard::new(subnet_id, "agreement") else {
        return Err("Concurrent call, aborting".to_string());
    };

    let Some(rental_agreement) = get_rental_agreement(&subnet_id) else {
        return Err("Rental agreement not found".to_string());
    };

    let user_icp_balance = check_subaccount_balance(Subaccount::from(rental_agreement.user)).await;

    if user_icp_balance < DEFAULT_FEE {
        return Err(format!(
            "Failed to top up: {} has insufficient funds {}",
            rental_agreement.user, user_icp_balance
        ));
    }

    let icp_amount_for_cycles = user_icp_balance - DEFAULT_FEE;

    // If the user were to withdraw before this call, the function would return an error.
    let actual_cycles = convert_icp_to_cycles(
        icp_amount_for_cycles,
        Subaccount::from(rental_agreement.user),
    )
    .await
    .map_err(|e| format!("Failed to convert ICP to cycles: {:?}", e))?;
    println!(
        "Converted {} ICP to {} cycles for subnet {}",
        icp_amount_for_cycles, actual_cycles, subnet_id
    );

    // calculate the new paid_until_nanos
    let daily_cost_cycles = get_rental_conditions(rental_agreement.rental_condition_id)
        .expect("Fatal: Rental Condition not found")
        .daily_cost_cycles;
    let cost_cycles_per_second = daily_cost_cycles / (SECONDS_PER_DAY as u128); // convert cost to cycles per second, rounding down
    let seconds_charged = actual_cycles / cost_cycles_per_second; // calculate how many seconds the topup covers, rounding down to nearest second
    let nanos_charged = seconds_charged.saturating_mul(BILLION as u128);
    let new_paid_until_nanos =
        (rental_agreement.paid_until_nanos as u128).saturating_add(nanos_charged);

    let new_paid_until_nanos = match new_paid_until_nanos.try_into() {
        Ok(val) => val,
        Err(_) => {
            // The user topped up the subnet beyond the year 2554.
            // At that point, u64 is too small to represent the number of nanoseconds since 1970.
            println!(
                "Warning: Top-up amount of {icp_amount_for_cycles} ICP = {actual_cycles} \
                cycles for {subnet_id} \
                caused a u64 overflow, capping at maximum possible u64 value"
            );
            u64::MAX
        }
    };

    let new_total_cycles_created = rental_agreement
        .total_cycles_created
        .saturating_add(actual_cycles);

    // Until the year 2554, u64 is enough to represent the number of days.
    let days_added = (seconds_charged / (SECONDS_PER_DAY as u128)) as u64;

    // update rental agreement
    update_rental_agreement(subnet_id, |mut agreement| {
        agreement.total_cycles_created = new_total_cycles_created;
        agreement.total_icp_paid += user_icp_balance; // Tokens do saturating adds
        agreement.paid_until_nanos = new_paid_until_nanos;
        agreement
    })
    .unwrap(); // Safe because we checked above that the rental agreement exists.

    let description = format!(
        "Topped up subnet {} with {} ICP corresponding to {} cycles, \
        extending the rental agreement by {} days",
        subnet_id, user_icp_balance, actual_cycles, days_added,
    );

    Ok(TopUpSummary {
        description,
        cycles_added: actual_cycles,
        days_added,
    })
}

// ============================================================================
// Misc

fn verify_caller_is_governance() -> Result<(), ExecuteProposalError> {
    if msg_caller() != MAINNET_GOVERNANCE_CANISTER_ID {
        println!("Caller is not the governance canister");
        return Err(ExecuteProposalError::UnauthorizedCaller);
    }
    Ok(())
}

fn round_to_previous_midnight(time_secs: u64) -> u64 {
    time_secs - time_secs % 86400
}

fn calculate_days_remaining(paid_until_nanos: u64, now_nanos: u64) -> u64 {
    if paid_until_nanos <= now_nanos {
        return 0;
    }
    let remaining_nanos = paid_until_nanos - now_nanos;
    remaining_nanos / (SECONDS_PER_DAY * BILLION) // convert to days
}

// allow candid-extractor to derive candid interface from rust code
ic_cdk::export_candid!();
