use candid::Principal;
use ic_ledger_types::{
    transfer, AccountIdentifier, Subaccount, Tokens, TransferArgs, TransferError, DEFAULT_FEE,
    MAINNET_CYCLES_MINTING_CANISTER_ID, MAINNET_LEDGER_CANISTER_ID,
};

use crate::external_canister_interfaces::exchange_rate_canister::{
    Asset, AssetClass, ExchangeRate, ExchangeRateError, ExchangeRateMetadata,
    GetExchangeRateRequest, GetExchangeRateResult, EXCHANGE_RATE_CANISTER_PRINCIPAL_STR,
};

use crate::external_types::{
    IcpXdrConversionRate, IcpXdrConversionRateResponse, NotifyError, NotifyTopUpArg,
    SetAuthorizedSubnetworkListArgs,
};
use crate::MEMO_TOP_UP_CANISTER;
// use ic_cdk::println;

pub async fn whitelist_principals(subnet_id: candid::Principal, principals: &Vec<Principal>) {
    for user in principals {
        ic_cdk::call::<_, ()>(
            MAINNET_CYCLES_MINTING_CANISTER_ID,
            "set_authorized_subnetwork_list",
            (SetAuthorizedSubnetworkListArgs {
                who: Some(*user),
                subnets: vec![subnet_id], // TODO: Add to the current list, don't overwrite
            },),
        )
        .await
        .expect("Failed to call CMC"); // TODO: handle error
    }
}

pub async fn delist_principals(_subnet_id: candid::Principal, principals: &Vec<candid::Principal>) {
    // TODO: if we allow multiple subnets per user:
    // first read the current list,
    // remove this subnet from the list and then
    // re-whitelist the principal for the remaining list
    for user in principals {
        ic_cdk::call::<_, ()>(
            MAINNET_CYCLES_MINTING_CANISTER_ID,
            "set_authorized_subnetwork_list",
            (SetAuthorizedSubnetworkListArgs {
                who: Some(*user),
                subnets: vec![],
            },),
        )
        .await
        .expect("Failed to call CMC"); // TODO: handle error
    }
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
    .expect("Failed to call ledger canister") // TODO: handle error
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
    .expect("Failed to call CMC") // TODO: handle error
    .0;

    xdr_permyriad_per_icp
}

/// Query the BaseAsset/QuoteAsset, XDR/ICP, exchange rate at the given time
pub async fn get_exchange_rate_cycles_per_e8s_at_time(time: u64) -> Result<f64, ExchangeRateError> {
    let icp_asset = Asset {
        class: AssetClass::Cryptocurrency,
        symbol: String::from("ICP"),
    };
    let xdr_asset = Asset {
        class: AssetClass::FiatCurrency,
        symbol: String::from("XDR"),
    };
    let request = GetExchangeRateRequest {
        timestamp: Some(time),
        quote_asset: icp_asset,
        base_asset: xdr_asset,
    };
    let response = ic_cdk::call::<_, (GetExchangeRateResult,)>(
        Principal::from_text(EXCHANGE_RATE_CANISTER_PRINCIPAL_STR).unwrap(),
        "get_exchange_rate",
        (request,),
    )
    .await
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
