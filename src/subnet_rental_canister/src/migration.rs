//! One-shot state migrations that run in `post_upgrade`.
//!
//! # Why a migration and not an endpoint
//!
//! Moving a rental agreement to a different rental condition is not something the renter
//! decides unilaterally: the SRC does not enforce subnet topology, so the billing change
//! only makes sense alongside a registry change that shrinks the subnet. Exposing it as a
//! public endpoint would let a renter reprice their agreement without the corresponding
//! topology change, so instead the switch is performed once, as part of the upgrade that
//! accompanies an NNS proposal.
//!
//! # Idempotency
//!
//! Each migration keys off state that the migration itself changes, so re-running it is a
//! no-op. No migration-version counter is stored: a second upgrade simply finds nothing
//! left to do. This matters because `post_upgrade` runs on *every* upgrade, not just the
//! one that ships the migration.
//!
//! # Lifetime
//!
//! These functions are expected to be deleted once the migration has been applied on
//! mainnet and the result has been verified.

use crate::{
    canister_state::{
        get_rental_agreement, get_rental_conditions, persist_event, update_rental_agreement,
    },
    history::EventType,
    RentalConditionId, BILLION, SECONDS_PER_DAY,
};
use candid::Principal;
use ic_cdk::println;

/// The Swiss Subnet, whose rental agreement moves from App13CH to App7CH.
/// Re-exported as `MIGRATION_TARGET_SUBNET` so the integration test drives the real
/// principal rather than a copy that could drift from it.
pub const TARGET_SUBNET: &str = "3zsyy-cnoqf-tvlun-ymf55-tkpca-ox7uw-kfxoh-7khwq-2gz43-wafem-lqe";

/// Moves `TARGET_SUBNET`'s rental agreement from App13CH to App7CH, repricing the
/// cycles they have already paid for but not yet burned.
///
/// Idempotent through the condition id itself: only agreements still on App13CH are
/// touched, so the second upgrade finds none.
///
/// This is not a payment. The unburned cycles the user already owns are repriced at the
/// cheaper App7CH daily cost, which extends the paid period. Pricing starts at the
/// migration time rather than scaling the old `paid_until_nanos`, because the elapsed part
/// of the period was already burned at the old rate.
pub fn app13ch_to_app7ch() {
    let Ok(subnet_id) = Principal::from_text(TARGET_SUBNET) else {
        println!("Migration to App7CH skipped: TARGET_SUBNET is not a valid principal");
        return;
    };

    let Some(agreement) = get_rental_agreement(&subnet_id) else {
        println!("Migration to App7CH skipped: no rental agreement for subnet {subnet_id}");
        return;
    };

    if agreement.rental_condition_id != RentalConditionId::App13CH {
        println!("Migration to App7CH already applied or not applicable; nothing to do");
        return;
    }

    let Some(new_conditions) = get_rental_conditions(RentalConditionId::App7CH) else {
        println!("Migration to App7CH skipped: App7CH rental conditions not found");
        return;
    };

    let cycles_remaining = agreement
        .total_cycles_created
        .saturating_sub(agreement.total_cycles_burned);
    let old_paid_until_nanos = agreement.paid_until_nanos;

    let now_nanos = ic_cdk::api::time();
    let new_paid_until_nanos = reprice(
        cycles_remaining,
        new_conditions.daily_cost_cycles,
        now_nanos,
    );

    if let Err(e) = update_rental_agreement(subnet_id, |mut agreement| {
        agreement.rental_condition_id = RentalConditionId::App7CH;
        agreement.paid_until_nanos = new_paid_until_nanos;
        agreement
    }) {
        println!("Migration of subnet {subnet_id} to App7CH failed: {e}");
        return;
    }

    persist_event(
        EventType::RentalConditionSwitched {
            subnet_id,
            user: agreement.user,
            old_condition_id: RentalConditionId::App13CH,
            new_condition_id: RentalConditionId::App7CH,
            cycles_remaining,
            old_paid_until_nanos,
            new_paid_until_nanos,
        },
        Some(subnet_id),
    );
    println!(
        "Migrated subnet {subnet_id} from App13CH to App7CH: {cycles_remaining} cycles now \
        paid until {new_paid_until_nanos} (was {old_paid_until_nanos})"
    );
}

/// How long `cycles_remaining` lasts at `daily_cost_cycles`, as a deadline from `now_nanos`.
///
/// Truncating to a per-second cost makes a day cost marginally less than
/// `daily_cost_cycles`, so the deadline is fractionally generous. Same rounding as the
/// top-up path.
fn reprice(cycles_remaining: u128, daily_cost_cycles: u128, now_nanos: u64) -> u64 {
    let cost_cycles_per_second = daily_cost_cycles / (SECONDS_PER_DAY as u128);
    if cost_cycles_per_second == 0 {
        return u64::MAX;
    }
    let seconds_covered = cycles_remaining / cost_cycles_per_second;
    (seconds_covered.saturating_mul(BILLION as u128))
        .try_into()
        .map_or(u64::MAX, |nanos: u64| now_nanos.saturating_add(nanos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TRILLION;

    const APP13CH_DAILY: u128 = 820 * TRILLION;
    const APP7CH_DAILY: u128 = 440 * TRILLION;
    const DAY_NANOS: u64 = SECONDS_PER_DAY * BILLION;

    #[test]
    fn reprice_is_exact_for_whole_days() {
        assert_eq!(reprice(10 * APP7CH_DAILY, APP7CH_DAILY, 0), 10 * DAY_NANOS);
    }

    #[test]
    fn reprice_offsets_from_now() {
        let now = 12_345 * DAY_NANOS;
        let cycles = 10 * APP7CH_DAILY;
        assert_eq!(reprice(cycles, APP7CH_DAILY, now), now + 10 * DAY_NANOS);
    }

    #[test]
    fn cheaper_condition_buys_more_time() {
        // 180 * 820 / 440 = 335.45
        let days = reprice(180 * APP13CH_DAILY, APP7CH_DAILY, 0) / DAY_NANOS;
        assert_eq!(days, 335);
    }

    #[test]
    fn reprice_credits_only_whole_seconds() {
        let cost_per_second = APP7CH_DAILY / (SECONDS_PER_DAY as u128);
        // A truncated per-second cost undercharges the day, so just under a day's
        // worth still covers the full day.
        assert!(cost_per_second * (SECONDS_PER_DAY as u128) < APP7CH_DAILY);
        assert_eq!(reprice(APP7CH_DAILY - 1, APP7CH_DAILY, 0), DAY_NANOS);

        let cycles = cost_per_second * (SECONDS_PER_DAY as u128 - 1);
        assert_eq!(reprice(cycles, APP7CH_DAILY, 0), DAY_NANOS - BILLION);

        let cycles = APP7CH_DAILY + APP7CH_DAILY / 2;
        assert_eq!(reprice(cycles, APP7CH_DAILY, 0) / DAY_NANOS, 1);
    }

    #[test]
    fn no_cycles_left_means_no_time_left() {
        let now = 99 * DAY_NANOS;
        assert_eq!(reprice(0, APP7CH_DAILY, 0), 0);
        assert_eq!(reprice(0, APP7CH_DAILY, now), now);
    }

    #[test]
    fn reprice_saturates_instead_of_overflowing() {
        assert_eq!(reprice(u128::MAX, APP7CH_DAILY, 0), u64::MAX);
        assert_eq!(reprice(10 * APP7CH_DAILY, APP7CH_DAILY, u64::MAX), u64::MAX);
        // A daily cost below one cycle per second would divide by zero.
        assert_eq!(reprice(APP7CH_DAILY, 1, 0), u64::MAX);
    }
}
