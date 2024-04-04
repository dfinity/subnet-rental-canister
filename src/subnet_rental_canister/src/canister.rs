use crate::canister_state::{
    self, create_rental_request, get_rental_conditions, insert_rental_condition,
    iter_rental_conditions,
};
use crate::external_calls::{
    call_with_retry, convert_icp_to_cycles, get_current_proposal_info,
    get_exchange_rate_xdr_per_icp_at_time, notify_top_up, transfer_to_cmc, transfer_to_src_main,
    whitelist_principals,
};
use crate::{
    canister_state::persist_event, history::EventType, RentalConditionId, RentalConditions,
    TRILLION,
};
use crate::{ExecuteProposalError, SubnetRentalProposalPayload, BILLION};
use candid::Principal;
use ic_cdk::{init, post_upgrade, query};
use ic_cdk::{println, update};
use ic_ledger_types::{
    AccountIdentifier, Subaccount, Tokens, TransferError, DEFAULT_FEE, DEFAULT_SUBACCOUNT,
    MAINNET_GOVERNANCE_CANISTER_ID,
};

////////// CANISTER METHODS //////////

#[init]
fn init() {
    // ic_cdk_timers::set_timer_interval(BILLING_INTERVAL, || ic_cdk::spawn(billing()));

    // Persist initial rental conditions in history.
    let initial_conditions = vec![(
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
        insert_rental_condition(k.clone(), v.clone());
        persist_event(
            EventType::RentalConditionsChanged {
                rental_condition_id: *k,
                rental_conditions: Some(v.clone()),
            },
            // Associate events that might belong to no subnet with None.
            None,
        );
    }
    println!("Subnet rental canister initialized");
}

#[post_upgrade]
fn post_upgrade() {
    // ic_cdk_timers::set_timer_interval(BILLING_INTERVAL, || ic_cdk::spawn(billing()));

    // persist all rental conditions in the history
    for (k, v) in iter_rental_conditions().iter() {
        println!("Loaded rental condition {:?}: {:?}", k, v);
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

// #[heartbeat]
// fn canister_heartbeat() {
// BILLING_RECORDS.with(|map| {
//     update_map(map, |subnet_id, billing_record| {
//         let Some(rental_agreement) = RENTAL_AGREEMENTS.with(|map| map.borrow().get(&subnet_id))
//         else {
//             println!(
//                 "Fatal: Failed to find active rental agreement for active billing record of subnet {}",
//                 subnet_id.0
//             );
//             return billing_record;
//         };
//         let cost_cycles_per_second =
//             rental_agreement.get_rental_conditions().daily_cost_cycles / 86400;
//         let now = ic_cdk::api::time();
//         let nanos_since_last_burn = now - billing_record.last_burned;
//         // cost_cycles_per_second: ~10^10 < 10^12
//         // nanos_since_last_burn:  ~10^9  < 10^15
//         // product                        < 10^27 << 10^38 (u128_max)
//         // divided by 1B                    10^-9
//         // amount                         < 10^18
//         let amount = cost_cycles_per_second * nanos_since_last_burn as u128 / 1_000_000_000;
//         if billing_record.cycles_balance < amount {
//             println!("Failed to burn cycles for subnet {}", subnet_id.0);
//             return billing_record;
//         }
//         // TODO: disabled for testing;
//         // let canister_total_available_cycles = ic_cdk::api::canister_balance128();
//         // if canister_total_available_cycles < amount {
//         //     println!(
//         //         "Fatal: Canister has fewer cycles {} than subaccount {:?}: {}",
//         //         canister_total_available_cycles, subnet_id, account.cycles_balance
//         //     );
//         //     return account;
//         // }
//         // Burn must succeed now
//         ic_cdk::api::cycles_burn(amount);
//         let cycles_balance = billing_record.cycles_balance - amount;
//         let last_burned = now;
//         println!(
//             "Burned {} cycles for subnet {}, remaining: {}",
//             amount, subnet_id.0, cycles_balance
//         );
//         BillingRecord {
//             covered_until: billing_record.covered_until,
//             cycles_balance,
//             last_burned,
//         }
//     });
// });
// }

////////// QUERY METHODS //////////

#[query]
pub fn list_rental_conditions() -> Vec<(RentalConditionId, RentalConditions)> {
    iter_rental_conditions()
}

// #[query]
// pub fn list_rental_agreements() -> Vec<RentalAgreement> {
//     RENTAL_AGREEMENTS.with(|map| map.borrow().iter().map(|(_, v)| v).collect())
// }

// #[query]
// pub fn list_billing_records() -> Vec<(Principal, BillingRecord)> {
//     BILLING_RECORDS.with(|map| map.borrow().iter().collect())
// }

// #[query]
// pub fn get_history(subnet: candid::Principal) -> Option<Vec<Event>> {
//     HISTORY.with(|map| {
//         map.borrow()
//             .get(&subnet.into())
//             .map(|history| history.events)
//     })
// }

////////// UPDATE METHODS //////////

// #[update]
// pub async fn terminate_rental_agreement(
//     RentalTerminationProposal { subnet_id }: RentalTerminationProposal,
// ) -> Result<(), ExecuteProposalError> {
//     // delist all principals from whitelists
//     // remove all entries in this canister
//     // persist in history
//     verify_caller_is_governance()?;
//     if let Some(RentalAgreement {
//         user,
//         subnet_id: _subnet_id,
//         principals,
//         creation_date: _creation_date,
//     }) = RENTAL_AGREEMENTS.with(|map| map.borrow_mut().get(&subnet_id.into()))
//     {
//         delist_principals(
//             subnet_id,
//             &principals
//                 .into_iter()
//                 .chain(std::iter::once(user))
//                 .unique()
//                 .map(|p| p)
//                 .collect(),
//         )
//         .await;
//         // TODO: possibly degrade subnet if not degraded yet
//         delete_rental_agreement(subnet_id.into());
//     } else {
//         println!("Error: Termination proposal contains a subnet_id that is not in an active rental agreement: {}", subnet_id);
//         return Err(ExecuteProposalError::SubnetNotRented);
//     }
//     Ok(())
// }

// TODO: Argument will be provided by governance canister after validation
#[update]
pub async fn execute_rental_request_proposal(
    SubnetRentalProposalPayload {
        user,
        rental_condition_type: rental_condition_id,
        proposal_id,
        proposal_creation_time,
    }: SubnetRentalProposalPayload,
) -> Result<(), ExecuteProposalError> {
    verify_caller_is_governance()?;

    // unwrap safety:
    // the rental_condition_type key must have a value in the rental conditions map at compile time.
    // TODO: unit test
    let RentalConditions {
        description,
        subnet_id,
        daily_cost_cycles,
        initial_rental_period_days,
        billing_period_days,
    } = get_rental_conditions(rental_condition_id).expect("Fatal: Rental conditions not found");

    // ------------------------------------------------------------------
    // Attempt to transfer enough ICP to cover the initial rental period.
    let needed_cycles = daily_cost_cycles.saturating_mul(initial_rental_period_days as u128);
    let exchange_rate_query_time = round_to_previous_midnight(proposal_creation_time);
    let res =
        call_with_retry(|| get_exchange_rate_xdr_per_icp_at_time(exchange_rate_query_time)).await;
    let Ok(exchange_rate_xdr_per_icp) = res else {
        println!("Fatal: Failed to get exchange rate");
        return Err(ExecuteProposalError::CallXRCFailed(res.unwrap_err()));
    };

    // trillion / e8 = 10_000
    let e8s = (needed_cycles as f64 / exchange_rate_xdr_per_icp) as u64 / 10_000;
    let needed_icp = Tokens::from_e8s(e8s);
    println!(
        "SRC requires {} cycles or {} ICP, according to exchange rate {}",
        needed_cycles, needed_icp, exchange_rate_xdr_per_icp
    );

    // transfer from user-derived subaccount to SRC main
    let res = call_with_retry(|| transfer_to_src_main(user.into(), needed_icp - DEFAULT_FEE)).await;
    let Ok(_) = res else {
        println!("Fatal: Failed to transfer ICP to SRC main account");
        return Err(ExecuteProposalError::TransferUserToSrcError(
            res.unwrap_err(),
        ));
    };
    // TODO: persist transferred amount

    // lock 10% by converting to cycles
    let lock_amount_icp = Tokens::from_e8s(e8s / 10);

    let locked_cycles = convert_icp_to_cycles(lock_amount_icp).await.unwrap();

    // TODO: recover from error OR fail early in proposal
    create_rental_request(user, locked_cycles, proposal_id, rental_condition_id).unwrap();

    // Either proceed with existing subnet_id, or start polling for future subnet creation.
    if let Some(subnet_id) = subnet_id {
        println!("Reusing existing subnet {:?}", subnet_id);
        // TODO: Create rental agreement
    } else {
        // TODO: Start polling
    }

    Ok(())
}

// Technically an update method, but called via canister timers.
// pub async fn billing() {
//     let exchange_rate_cycles_per_e8s = get_current_avg_exchange_rate_cycles_per_e8s().await;

//     for (subnet_id, rental_agreement) in
//         RENTAL_AGREEMENTS.with(|map| map.borrow().iter().collect::<Vec<_>>())
//     {
//         {
//             let Some(BillingRecord { covered_until, .. }) =
//                 BILLING_RECORDS.with(|map| map.borrow_mut().get(&subnet_id))
//             else {
//                 println!(
//                     "FATAL: No billing record found for active rental agreement for subnet {}",
//                     &subnet_id.0
//                 );
//                 continue;
//             };

//             // Check if subnet is covered for next billing_period amount of days.
//             let now = ic_cdk::api::time();
//             let billing_period_nanos =
//                 days_to_nanos(rental_agreement.get_rental_conditions().billing_period_days);

//             if covered_until < now {
//                 println!(
//                     "Subnet {} is not covered anymore, degrading...",
//                     subnet_id.0
//                 );
//                 // TODO: Degrade service
//                 persist_event(EventType::Degraded, subnet_id);
//             } else if covered_until < now + billing_period_nanos {
//                 // Next billing period is not fully covered anymore.
//                 // Try to withdraw ICP and convert to cycles.
//                 let needed_cycles = rental_agreement
//                     .get_rental_conditions()
//                     .daily_cost_cycles
//                     .saturating_mul(
//                         rental_agreement.get_rental_conditions().billing_period_days as u128,
//                     ); // TODO: get up to date rental conditions

//                 let icp_amount = Tokens::from_e8s(
//                     needed_cycles.saturating_div(exchange_rate_cycles_per_e8s as u128) as u64,
//                 );

//                 // Transfer ICP to SRC.
//                 let transfer_to_src_result =
//                     icrc2_transfer_to_src(rental_agreement.user.0, icp_amount - DEFAULT_FEE).await;

//                 if let Err(err) = transfer_to_src_result {
//                     println!(
//                         "{}: Transfer from user {} to SRC failed: {:?}",
//                         subnet_id.0, rental_agreement.user.0, err
//                     );
//                     persist_event(
//                         EventType::PaymentFailure {
//                             reason: format!("{err:?}"),
//                         },
//                         subnet_id,
//                     );
//                     continue;
//                 }

//                 // Transfer ICP to CMC.
//                 let transfer_to_cmc_result =
//                     transfer_to_cmc(icp_amount - DEFAULT_FEE - DEFAULT_FEE).await;
//                 let Ok(block_index) = transfer_to_cmc_result else {
//                     // TODO: This should not happen.
//                     let err = transfer_to_cmc_result.unwrap_err();
//                     println!("Transfer from SRC to CMC failed: {:?}", err);
//                     continue;
//                 };

//                 // Call notify_top_up to exchange ICP for cycles.
//                 let notify_top_up_result = notify_top_up(block_index).await;
//                 let Ok(actual_cycles) = notify_top_up_result else {
//                     let err = notify_top_up_result.unwrap_err();
//                     println!("Notify top-up failed: {:?}", err);
//                     continue;
//                 };

//                 // Add cycles to billing record, update covered_until.
//                 let new_covered_until = covered_until + billing_period_nanos;
//                 BILLING_RECORDS.with(|map| {
//                     let mut billing_record = map.borrow().get(&subnet_id).unwrap();
//                     billing_record.covered_until = new_covered_until;
//                     billing_record.cycles_balance += actual_cycles;
//                     map.borrow_mut().insert(subnet_id, billing_record);
//                 });

//                 println!("Now covered until {}", new_covered_until);
//                 persist_event(
//                     EventType::PaymentSuccess {
//                         amount: icp_amount,
//                         covered_until: new_covered_until,
//                     },
//                     subnet_id,
//                 );
//             } else {
//                 // Next billing period is still fully covered.
//                 println!("Subnet is covered until {}, now is {}", covered_until, now);
//             }
//         }
//     }
// }

// ============================================================================
// Misc

fn verify_caller_is_governance() -> Result<(), ExecuteProposalError> {
    if ic_cdk::caller() != MAINNET_GOVERNANCE_CANISTER_ID {
        println!("Caller is not the governance canister");
        return Err(ExecuteProposalError::UnauthorizedCaller);
    }
    Ok(())
}

fn round_to_previous_midnight(time: u64) -> u64 {
    time - time % 86400
}
