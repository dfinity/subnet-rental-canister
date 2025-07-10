use crate::canister_state::{cache_rate, get_cached_rate};
use crate::external_types::{NotifyError, NotifyTopUpArg, SetAuthorizedSubnetworkListArgs};
use crate::{ExecuteProposalError, MEMO_TOP_UP_CANISTER};
use candid::Principal;
use ic_cdk::{call::Call, println};
use ic_ledger_types::{
    transfer, AccountBalanceArgs, AccountIdentifier, Memo, Subaccount, Tokens, TransferArgs,
    TransferError, DEFAULT_FEE, DEFAULT_SUBACCOUNT, MAINNET_CYCLES_MINTING_CANISTER_ID,
    MAINNET_LEDGER_CANISTER_ID,
};
use ic_xrc_types::{
    Asset, AssetClass, ExchangeRate, ExchangeRateError, ExchangeRateMetadata,
    GetExchangeRateRequest, GetExchangeRateResult,
};

pub const EXCHANGE_RATE_CANISTER_ID: Principal =
    Principal::from_slice(b"\x00\x00\x00\x00\x02\x10\x00\x01\x01\x01"); // uf6dk-hyaaa-aaaaq-qaaaq-cai

pub async fn set_authorized_subnetwork_list(user: &Principal, subnet_id: &Principal) {
    Call::unbounded_wait(
        MAINNET_CYCLES_MINTING_CANISTER_ID,
        "set_authorized_subnetwork_list",
    )
    .with_arg(SetAuthorizedSubnetworkListArgs {
        who: Some(*user),
        subnets: vec![*subnet_id], // TODO: Add to the current list, don't overwrite
    })
    .await
    .expect("Failed to call CMC");
}

async fn notify_top_up(block_index: u64) -> Result<u128, NotifyError> {
    Call::unbounded_wait(MAINNET_CYCLES_MINTING_CANISTER_ID, "notify_top_up")
        .with_arg(NotifyTopUpArg {
            block_index,
            canister_id: ic_cdk::api::canister_self(),
        })
        .await
        .expect("Failed to call CMC")
        .candid()
        .expect("Failed to decode result")
}

async fn transfer_to_cmc(amount: Tokens, source: Subaccount) -> Result<u64, TransferError> {
    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        &TransferArgs {
            to: AccountIdentifier::new(
                &MAINNET_CYCLES_MINTING_CANISTER_ID,
                &Subaccount::from(ic_cdk::api::canister_self()),
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

pub async fn refund_user(user_principal: Principal, amount: Tokens) -> Result<u64, TransferError> {
    transfer(
        MAINNET_LEDGER_CANISTER_ID,
        &TransferArgs {
            to: AccountIdentifier::new(&user_principal, &DEFAULT_SUBACCOUNT),
            fee: DEFAULT_FEE,
            from_subaccount: Some(Subaccount::from(user_principal)),
            amount,
            memo: Memo(0),
            created_at_time: None,
        },
    )
    .await
    .expect("Failed to call ledger canister")
}

/// Query the XDR/ICP exchange rate at the given time in seconds since epoch.
/// Returns (rate, decimals), where the rate is scaled by 10^decimals.
/// This function attempts to read from the global RATES cache and updates it.
pub async fn get_exchange_rate_icp_per_xdr_at_time(
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
    let response: GetExchangeRateResult =
        Call::unbounded_wait(EXCHANGE_RATE_CANISTER_ID, "get_exchange_rate")
            .with_arg(request)
            .with_cycles(1_000_000_000)
            .await
            .expect("Failed to call ExchangeRateCanister")
            .candid()
            .expect("Failed to decode result");

    match response {
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

/// Converts ICP from a user's SRC subaccount to cycles.
/// Returns the actual amount of cycles created.
pub async fn convert_icp_to_cycles(
    amount: Tokens,
    source: Subaccount,
) -> Result<u128, ExecuteProposalError> {
    // Transfer the ICP from the SRC to the CMC.
    let transfer_to_cmc_result = transfer_to_cmc(amount - DEFAULT_FEE, source).await;
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

/// Check balance of a user's SRC subaccount.
pub async fn check_subaccount_balance(subaccount: Subaccount) -> Tokens {
    Call::unbounded_wait(MAINNET_LEDGER_CANISTER_ID, "account_balance")
        .with_arg(AccountBalanceArgs {
            account: AccountIdentifier::new(&ic_cdk::api::canister_self(), &subaccount),
        })
        .await
        .expect("Failed to call LedgerCanister")
        .candid()
        .expect("Failed to decode result")
}
