use candid::Principal;
use ic_ledger_types::{
    transfer, AccountBalanceArgs, AccountIdentifier, Memo, Subaccount, Tokens, TransferArgs,
    TransferError, DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_CYCLES_MINTING_CANISTER_ID,
    MAINNET_LEDGER_CANISTER_ID,
};

use crate::canister_state::{cache_rate, get_cached_rate};
use crate::external_canister_interfaces::exchange_rate_canister::{
    Asset, AssetClass, ExchangeRate, ExchangeRateError, ExchangeRateMetadata,
    GetExchangeRateRequest, GetExchangeRateResult, EXCHANGE_RATE_CANISTER_PRINCIPAL_STR,
};

use crate::external_canister_interfaces::governance_canister::{
    ListProposalInfo, ListProposalInfoResponse, GOVERNANCE_CANISTER_PRINCIPAL_STR,
};
use crate::external_types::{NotifyError, NotifyTopUpArg, SetAuthorizedSubnetworkListArgs};
use crate::{ExecuteProposalError, MEMO_TOP_UP_CANISTER};
use ic_cdk::println;

pub async fn whitelist_principals(subnet_id: Principal, user: &Principal) {
    ic_cdk::call::<_, ()>(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        "set_authorized_subnetwork_list",
        (SetAuthorizedSubnetworkListArgs {
            who: Some(*user),
            subnets: vec![subnet_id], // TODO: Add to the current list, don't overwrite
        },),
    )
    .await
    .expect("Failed to call CMC");
}

pub async fn notify_top_up(block_index: u64) -> Result<u128, NotifyError> {
    ic_cdk::call::<_, (Result<u128, NotifyError>,)>(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        "notify_top_up",
        (NotifyTopUpArg {
            block_index,
            canister_id: ic_cdk::id(),
        },),
    )
    .await
    .expect("Failed to call CMC")
    .0
    // TODO: In the canister logs, the CMC claims that the burning of ICPs failed, but the cycles are minted anyway.
    // It states that the "transfer fee should be 0.00010000 Token", but that fee is hardcoded to
    // (ZERO)[https://sourcegraph.com/github.com/dfinity/ic@8126ad2fab0196908d9456a65914a3e05179ac4b/-/blob/rs/nns/cmc/src/main.rs?L1835]
    // in the CMC, and cannot be changed from outside. What's going on here?
}

pub async fn transfer_to_cmc(amount: Tokens, source: Subaccount) -> Result<u64, TransferError> {
    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        TransferArgs {
            to: AccountIdentifier::new(
                &MAINNET_CYCLES_MINTING_CANISTER_ID,
                &Subaccount::from(ic_cdk::id()),
            ),
            fee: DEFAULT_FEE,
            from_subaccount: Some(source),
            amount,
            memo: MEMO_TOP_UP_CANISTER,
            created_at_time: None,
        },
    )
    .await
    .expect("Failed to call ledger canister")
}

/// Transfer ICP from user-derived subaccount to SRC default subaccount
pub async fn transfer_to_src_main(
    source: Subaccount,
    amount: Tokens,
    proposal_id: u64,
) -> Result<u64, TransferError> {
    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        TransferArgs {
            to: AccountIdentifier::new(&ic_cdk::id(), &DEFAULT_SUBACCOUNT),
            fee: DEFAULT_FEE,
            from_subaccount: Some(source),
            amount,
            // deduplication
            memo: Memo(proposal_id),
            created_at_time: None,
        },
    )
    .await
    .expect("Failed to call ledger canister")
}

pub async fn refund_user(
    user_principal: Principal,
    amount: Tokens,
    proposal_id: u64,
) -> Result<u64, TransferError> {
    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        TransferArgs {
            to: AccountIdentifier::new(&user_principal, &DEFAULT_SUBACCOUNT),
            fee: DEFAULT_FEE,
            from_subaccount: Some(Subaccount::from(user_principal)),
            amount,
            memo: Memo(proposal_id),
            created_at_time: None,
        },
    )
    .await
    .expect("Failed to call ledger canister")
}

/// Query the XDR/ICP exchange rate at the given time in seconds since epoch.
/// Returns (rate, decimals), where the rate is scaled by 10^decimals.
/// This function attempts to read from the global RATES cache and updates it.
pub async fn get_exchange_rate_xdr_per_icp_at_time(
    time_secs_since_epoch: u64,
) -> Result<(u64, u32), ExchangeRateError> {
    // The SRC keeps a cache of exchange rates
    if let Some(tup) = get_cached_rate(time_secs_since_epoch) {
        return Ok(tup);
    }
    let icp_asset = Asset {
        class: AssetClass::Cryptocurrency,
        symbol: String::from("ICP"),
    };
    let xdr_asset = Asset {
        class: AssetClass::FiatCurrency,
        // The computed "CXDR" symbol is more likely to have a value than XDR.
        symbol: String::from("CXDR"),
    };
    let request = GetExchangeRateRequest {
        timestamp: Some(time_secs_since_epoch),
        quote_asset: xdr_asset,
        base_asset: icp_asset,
    };
    let response = ic_cdk::api::call::call_with_payment128::<_, (GetExchangeRateResult,)>(
        Principal::from_text(EXCHANGE_RATE_CANISTER_PRINCIPAL_STR).unwrap(),
        "get_exchange_rate",
        (request,),
        10_000_000_000,
    )
    .await
    .expect("Failed to call ExchangeRateCanister");
    match response.0 {
        GetExchangeRateResult::Ok(ExchangeRate {
            metadata: ExchangeRateMetadata { decimals, .. },
            rate,
            ..
        }) => {
            cache_rate(time_secs_since_epoch, rate, decimals);
            Ok((rate, decimals))
        }
        GetExchangeRateResult::Err(e) => Err(e),
    }
}

pub async fn convert_icp_to_cycles(
    amount: Tokens,
    source: Subaccount,
) -> Result<u128, ExecuteProposalError> {
    // Transfer the ICP from the SRC to the CMC. The second fee is for the notify top-up.
    let transfer_to_cmc_result = transfer_to_cmc(amount - DEFAULT_FEE - DEFAULT_FEE, source).await;
    let Ok(block_index) = transfer_to_cmc_result else {
        let e = transfer_to_cmc_result.unwrap_err();
        println!("Transfer from SRC to CMC failed: {:?}", e);
        return Err(ExecuteProposalError::TransferSrcToCmcError(e.to_string()));
    };

    // Notify CMC about the top-up. This is what triggers the exchange from ICP to cycles.
    let notify_top_up_result = notify_top_up(block_index).await;
    let Ok(actual_cycles) = notify_top_up_result else {
        let e = notify_top_up_result.unwrap_err();
        println!("Notify top-up failed: {:?}", e);
        return Err(ExecuteProposalError::NotifyTopUpError(format!("{:?}", e)));
    };
    Ok(actual_cycles)
}

/// Used for polling for the subnet creation proposal.
pub async fn get_create_subnet_proposal() -> Result<(), String> {
    // TODO
    let request = ListProposalInfo {
        include_reward_status: vec![],
        omit_large_fields: Some(true),
        before_proposal: None,
        limit: 100,
        exclude_topic: vec![],
        include_all_manage_neuron_proposals: Some(true),
        // We only want ProposalStatus::Adopted = 3
        include_status: vec![3],
    };
    let ListProposalInfoResponse { proposal_info: _ } =
        ic_cdk::call::<_, (ListProposalInfoResponse,)>(
            Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap(),
            "list_proposals",
            (request,),
        )
        .await
        .expect("Failed to call GovernanceCanister")
        .0;
    todo!()
}

pub async fn check_subaccount_balance(subaccount: Subaccount) -> Tokens {
    ic_cdk::call::<_, (Tokens,)>(
        MAINNET_LEDGER_CANISTER_ID,
        "account_balance",
        (AccountBalanceArgs {
            account: AccountIdentifier::new(&ic_cdk::id(), &subaccount),
        },),
    )
    .await
    .expect("Failed to call LedgerCanister")
    .0
}
