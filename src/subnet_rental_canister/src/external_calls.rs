use std::time::Duration;

use candid::Principal;
use ic_cdk::api::call::call_with_payment128;
use ic_ledger_types::{
    transfer, AccountIdentifier, Memo, Subaccount, Tokens, TransferArgs, TransferError,
    DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_CYCLES_MINTING_CANISTER_ID,
    MAINNET_LEDGER_CANISTER_ID,
};

use crate::external_canister_interfaces::exchange_rate_canister::{
    Asset, AssetClass, ExchangeRate, ExchangeRateError, ExchangeRateMetadata,
    GetExchangeRateRequest, GetExchangeRateResult, EXCHANGE_RATE_CANISTER_PRINCIPAL_STR,
};

use crate::external_canister_interfaces::governance_canister::{
    ListProposalInfo, ListProposalInfoResponse, ProposalInfo, GOVERNANCE_CANISTER_PRINCIPAL_STR,
};
use crate::external_types::{
    IcpXdrConversionRate, IcpXdrConversionRateResponse, NotifyError, NotifyTopUpArg,
    SetAuthorizedSubnetworkListArgs,
};
use crate::MEMO_TOP_UP_CANISTER;
// use ic_cdk::println;

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
    // chance of synchronous errors is small on NNS subnet
    .expect("Failed to call CMC");
}

pub async fn delist_principal(_subnet_id: candid::Principal, user: &Principal) {
    // TODO: if we allow multiple subnets per user:
    // first read the current list,
    // remove this subnet from the list and then
    // re-whitelist the principal for the remaining list
    ic_cdk::call::<_, ()>(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        "set_authorized_subnetwork_list",
        (SetAuthorizedSubnetworkListArgs {
            who: Some(*user),
            subnets: vec![],
        },),
    )
    .await
    // chance of synchronous errors is small on NNS subnet
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
    .expect("Failed to call CMC") // TODO: handle error
    .0
    // TODO: In the canister logs, the CMC claims that the burning of ICPs failed, but the cycles are minted anyway.
    // It states that the "transfer fee should be 0.00010000 Token", but that fee is hardcoded to
    // (ZERO)[https://sourcegraph.com/github.com/dfinity/ic@8126ad2fab0196908d9456a65914a3e05179ac4b/-/blob/rs/nns/cmc/src/main.rs?L1835]
    // in the CMC, and cannot be changed from outside. What's going on here?
}

pub async fn transfer_to_cmc(amount: Tokens) -> Result<u64, TransferError> {
    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        TransferArgs {
            to: AccountIdentifier::new(
                &MAINNET_CYCLES_MINTING_CANISTER_ID,
                &Subaccount::from(ic_cdk::id()),
            ),
            fee: DEFAULT_FEE,
            from_subaccount: None,
            amount,
            memo: MEMO_TOP_UP_CANISTER,
            created_at_time: None, // TODO: set for deduplication
        },
    )
    .await
    // chance of synchronous errors is small on NNS subnet
    .expect("Failed to call ledger canister")
}

/// Transfer ICP from user-derived subaccount to SRC default subaccount
pub async fn transfer_to_src_main(
    source: Subaccount,
    amount: Tokens,
) -> Result<u64, TransferError> {
    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        TransferArgs {
            to: AccountIdentifier::new(&ic_cdk::id(), &DEFAULT_SUBACCOUNT),
            fee: DEFAULT_FEE,
            from_subaccount: Some(source),
            amount,
            memo: Memo(0),
            created_at_time: None, // TODO: set for deduplication
        },
    )
    .await
    // chance of synchronous errors is small on NNS subnet
    .expect("Failed to call ledger canister")
}

pub async fn get_exchange_rate_cycles_per_e8s() -> u64 {
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
    // chance of synchronous errors is small on NNS subnet
    .expect("Failed to call CMC")
    .0;

    xdr_permyriad_per_icp
}

/// Query the ICP/XDR exchange rate at the given time
pub async fn get_exchange_rate_icp_per_xdr_at_time(time: u64) -> Result<f64, ExchangeRateError> {
    let icp_asset = Asset {
        class: AssetClass::Cryptocurrency,
        symbol: String::from("ICP"),
    };
    let xdr_asset = Asset {
        class: AssetClass::FiatCurrency,
        // the computed "CXDR" symbol is more likely to have a value than XDR.
        symbol: String::from("CXDR"),
    };
    // order: BaseAsset/QuoteAsset
    let request = GetExchangeRateRequest {
        timestamp: Some(time),
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
    // chance of synchronous errors is small on NNS subnet
    .expect("Failed to call ExchangeRateCanister");
    match response.0 {
        GetExchangeRateResult::Ok(ExchangeRate {
            metadata: ExchangeRateMetadata { decimals, .. },
            rate,
            ..
        }) => {
            // The rate is a scaled integer. The scaling factor is 10^decimals.
            let scale = u64::pow(10, decimals);
            let res = rate as f64 / scale as f64;
            Ok(res)
        }
        GetExchangeRateResult::Err(e) => Err(e),
    }
}

pub async fn convert_icp_to_cycles(amount: Tokens) -> Result<u128, String> {
    // Transfer the ICP from the SRC to the CMC. The second fee is for the notify top-up.
    let transfer_to_cmc_result = transfer_to_cmc(amount - DEFAULT_FEE - DEFAULT_FEE).await;
    let Ok(block_index) = transfer_to_cmc_result else {
        let err = transfer_to_cmc_result.unwrap_err();
        println!("Transfer from SRC to CMC failed: {:?}", err);
        // TODO: event
        // return Err(ExecuteProposalError::TransferSrcToCmcError(err));
        return Err(String::new());
    };

    // Notify CMC about the top-up. This is what triggers the exchange from ICP to cycles.
    let notify_top_up_result = notify_top_up(block_index).await;
    let Ok(actual_cycles) = notify_top_up_result else {
        let err = notify_top_up_result.unwrap_err();
        println!("Notify top-up failed: {:?}", err);
        // TODO: event
        // return Err(ExecuteProposalError::NotifyTopUpError(err));
        return Err(String::new());
    };
    Ok(actual_cycles)
}

/// Get the proposal id and the proposal creation time in seconds.
pub async fn get_current_proposal_info() -> Result<(u64, u64), String> {
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
    let ListProposalInfoResponse { proposal_info } =
        ic_cdk::call::<_, (ListProposalInfoResponse,)>(
            Principal::from_text(GOVERNANCE_CANISTER_PRINCIPAL_STR).unwrap(),
            "list_proposals",
            (request,),
        )
        .await
        // chance of synchronous errors is small on NNS subnet
        .expect("Failed to call GovernanceCanister")
        .0;
    // with the correct, unique topic and the correct status, there should be exactly one result:
    if proposal_info.len() < 1 {
        return Err("Found no active proposals, expected one".to_string());
    } else if proposal_info.len() > 1 {
        return Err("Found several matching proposals, expected one".to_string());
    }
    let proposal_info = proposal_info.get(0).unwrap();
    let proposal_id = proposal_info.id.unwrap().id;
    let proposal_creation_time_seconds = proposal_info.proposal_timestamp_seconds;
    Ok((proposal_id, proposal_creation_time_seconds))
}

// Since we might need to call a future several times, we need to pass a future generator.
pub async fn call_with_retry<Fut, R, E>(f: impl Fn() -> Fut) -> Result<R, E>
where
    Fut: std::future::Future<Output = Result<R, E>>,
{
    let mut attempts = 3;
    loop {
        attempts -= 1;
        let result: Result<R, E> = f().await;
        if result.is_ok() || attempts <= 0 {
            return result;
        }
        std::thread::sleep(Duration::from_millis(330));
    }
}
