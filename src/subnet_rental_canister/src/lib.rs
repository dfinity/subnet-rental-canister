use candid::{CandidType, Decode, Deserialize, Encode, Principal as PrincipalImpl};
use ic_cdk::{api::call::CallResult, call, init, query, update};
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    DefaultMemoryImpl, StableBTreeMap, Storable,
};
use serde::Serialize;
use std::{borrow::Cow, cell::RefCell, collections::HashMap, time::Duration};

const _LEDGER_ID: &str = "todo";
const CMC_ID: &str = "rkp4c-7iaaa-aaaaa-aaaca-cai";
// The canister_id of the SRC
const _SRC_PRINCIPAL: &str = "src_principal";
// During billing, the cost in cycles is fixed, but the cost in ICP depends on the exchange rate
const _XDR_COST_PER_DAY: u64 = 1;
const E8S: u64 = 100_000_000;
const MAX_PRINCIPAL_SIZE: u32 = 29;
const HTML_HEAD: &str =
    r#"<!DOCTYPE html><html lang="en"><head><title>Subnet Rental Canister</title></head>"#;

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    // Memory region 0
    static RENTAL_AGREEMENTS: RefCell<StableBTreeMap<Principal, RentalAgreement, VirtualMemory<DefaultMemoryImpl>>> =
        RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    /// Hardcoded subnets and their rental conditions.
    static SUBNETS: RefCell<HashMap<Principal, RentalConditions>> = HashMap::from([
        (
            Principal(
                candid::Principal::from_text(
                    "bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe",
                )
                .unwrap(),
            ),
            RentalConditions {
                daily_cost_e8s: 333 * E8S,
                minimal_rental_period_days: 365,
            },
        ),
        (
            Principal(
                candid::Principal::from_text(
                    "fuqsr-in2lc-zbcjj-ydmcw-pzq7h-4xm2z-pto4i-dcyee-5z4rz-x63ji-nae",
                )
                .unwrap(),
            ),
            RentalConditions {
                daily_cost_e8s: 100 * E8S,
                minimal_rental_period_days: 183,
            },
        ),
    ]).into();
}

type SubnetId = Principal;

#[derive(
    Debug, Clone, Copy, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, CandidType, Hash,
)]
pub struct Principal(PrincipalImpl);

impl From<PrincipalImpl> for Principal {
    fn from(value: PrincipalImpl) -> Self {
        Self(value)
    }
}

impl Storable for Principal {
    const BOUND: Bound = Bound::Bounded {
        max_size: MAX_PRINCIPAL_SIZE,
        is_fixed_size: false,
    };
    fn to_bytes(&self) -> std::borrow::Cow<[u8]> {
        Cow::Owned(self.0.as_slice().to_vec())
    }

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Self(PrincipalImpl::try_from_slice(bytes.as_ref()).unwrap())
    }
}

#[derive(CandidType)]
pub enum ExecuteProposalError {
    Failure(String),
}

/// Set of conditions for a specific subnet up for rent.
#[derive(Debug, Clone, Copy, CandidType, Deserialize)]
pub struct RentalConditions {
    daily_cost_e8s: u64,
    minimal_rental_period_days: u64,
}

/// Immutable rental agreement; mutabla data and log events should refer to it via the id.
#[derive(Debug, Clone, CandidType, Deserialize)]
struct RentalAgreement {
    id: u64,
    user: Principal,
    subnet: SubnetId,
    principals: Vec<Principal>,
    refund_subaccount: String,
    initial_period_days: u64,
    initial_period_cost_e8s: u64,
    // nanos since epoch?  TODO: figure out how times are handled in NNS canisters
    // creation_date: Date, // https://time-rs.github.io/book/how-to/create-dates.html date might be resolution enough, because we have no sub-day durations, so timezone offsets should be irrelevant.
    creation_date: u64,
}

impl Storable for RentalAgreement {
    // should be bounded once we replace string with real type
    const BOUND: Bound = Bound::Unbounded;
    fn to_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
}

#[init]
fn init() {
    // Hardcoded rental agreement for testing
    let subnet = Principal(
        candid::Principal::from_text(
            "bkfrj-6k62g-dycql-7h53p-atvkj-zg4to-gaogh-netha-ptybj-ntsgw-rqe",
        )
        .unwrap(),
    );
    let renter = Principal(candid::Principal::from_slice(b"user1"));
    let user = Principal(candid::Principal::from_slice(b"user2"));
    RENTAL_AGREEMENTS.with(|map| {
        map.borrow_mut().insert(
            subnet,
            RentalAgreement {
                id: 120345,
                user: renter,
                subnet,
                principals: vec![renter, user],
                refund_subaccount: "my-wallet-address".to_owned(),
                initial_period_days: 365,
                initial_period_cost_e8s: 333 * 365 * E8S,
                creation_date: 1702394252000000000,
            },
        )
    });
}

#[query]
fn list_subnet_conditions() -> HashMap<SubnetId, RentalConditions> {
    SUBNETS.with(|map| map.borrow().clone())
}

#[query]
fn list_rental_agreements() -> Vec<RentalAgreement> {
    RENTAL_AGREEMENTS.with(|map| map.borrow().iter().map(|(_, v)| v.clone()).collect())
}

#[query]
fn http_request(req: HttpRequest) -> HttpResponse {
    match req.url.as_str() {
        "/" => html_ok_response(format!(
            r#"{}<body><h1>Subnet Rental Canister</h1><ul><li><a href="/subnets">Subnets for Rent</a></li><li><a href="/rental_agreements">Rental Agreements</a></li></ul></body></html>"#,
            HTML_HEAD
        )),
        "/subnets" => html_ok_response(generate_rental_conditions_html()),
        "/rental_agreements" => html_ok_response(generate_rental_agreements_html()),
        _ => html_response(404, "Not found".to_string()),
    }
}

/// TODO: Argument should be something like ValidatedSRProposal, created by government canister via
/// SRProposal::validate().
#[update]
async fn on_proposal_accept(
    subnet_id: SubnetId,
    user: Principal,
    _principals: Vec<Principal>,
    _block_index: usize,
    _refund_address: String,
) -> Result<(), ExecuteProposalError> {
    // Assumptions:
    // - A single deposit transaction exists and covers the necessary amount.
    // - The deposit was made to the <subnet_id>-subaccount of the SRC.

    // Collect rental information
    // If the governance canister was able to validate, then this entry must exist, so we can unwrap.
    let RentalConditions {
        daily_cost_e8s,
        minimal_rental_period_days,
    } = SUBNETS.with(|rc| *rc.borrow().get(&subnet_id).unwrap());

    // cost of initial period: TODO: overflows?
    let _initial_cost_e8s = daily_cost_e8s * minimal_rental_period_days;
    // turn this amount of ICP into cycles and burn them.
    // 1. transfer the right amount of ICP to the CMC
    // 2. create NotifyTopUpArg{ block_index, canister_id } from that transaction
    // 3. call CMC with the notify arg to get cycles
    // 4. burn the cycles with the system api
    // 5. set the end date of the initial period
    // 6. fill in the other rental agreement details
    // 7. add it to the rental agreement map

    // Whitelist the principal
    let result: CallResult<()> = call(
        PrincipalImpl::from_text(CMC_ID).unwrap(),
        "set_authorized_subnetwork_list",
        (Some(user), vec![subnet_id]), // TODO: figure out exact semantics of this method.
    )
    .await;
    match result {
        Ok(_) => {}
        // TODO: figure out failure modes of this method and consequences. can this call fail at all? the deposit is gone by now..
        Err((code, msg)) => {
            ic_cdk::println!("Call to CMC failed: {:?}, {}", code, msg);
            return Err(ExecuteProposalError::Failure(
                "Failed to call CMC".to_string(),
            ));
        }
    }

    Ok(())
}

fn html_ok_response(html: String) -> HttpResponse {
    html_response(200, html)
}

fn html_response(status_code: u16, html: String) -> HttpResponse {
    HttpResponse {
        status_code,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: html.as_bytes().to_vec(),
    }
}

fn generate_rental_agreements_html() -> String {
    let rental_agreements = list_rental_agreements();

    let mut html = String::new();
    html.push_str(HTML_HEAD);
    html.push_str(
        r#"<body><h1>Rental Agreements</h1><table border="1"><tr><th>Subnet ID</th><th>Renter</th><th>Allowed Principals</th><th>Refund Address</th><th>Initial Period (days)</th><th>Initial Period Cost (ICP)</th><th>Creation Date</th><th>Status</th></tr>"#,
    );
    for agreement in rental_agreements {
        html.push_str("<tr>");
        html.push_str(&format!(
            "<td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:?}</td><td>{}</td>",
            agreement.subnet.0,
            agreement.user.0,
            agreement
                .principals
                .iter()
                .map(|p| p.0.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            agreement.refund_subaccount,
            agreement.initial_period_days,
            agreement.initial_period_cost_e8s / 100_000_000,
            Duration::from_nanos(agreement.creation_date),
            "Healthy"
        ));
        html.push_str("</tr>");
    }
    html.push_str("</table></body></html>");
    html
}

fn generate_rental_conditions_html() -> String {
    let rental_conditions = list_subnet_conditions();

    let mut html = String::new();
    html.push_str(HTML_HEAD);
    html.push_str(
        r#"<body><h1>Subnets for Rent</h1><table border="1"><tr><th>Subnet ID</th><th>Daily Cost (ICP)</th><th>Minimal Rental Period (days)</th><th>Status</th></tr>"#,
    );
    for (subnet_id, conditions) in rental_conditions {
        html.push_str("<tr>");
        let rented = RENTAL_AGREEMENTS.with(|map| map.borrow().contains_key(&subnet_id));
        html.push_str(&format!(
            "<td>{}</td><td>{}</td><td>{}</td><td>{}</td>",
            subnet_id.0,
            conditions.daily_cost_e8s / 100_000_000,
            conditions.minimal_rental_period_days,
            if rented { "Rented" } else { "Available" }
        ));
        html.push_str("</tr>");
    }
    html.push_str("</table></body></html>");
    html
}

#[derive(CandidType)]
struct HttpResponse {
    status_code: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(CandidType, Deserialize, Debug)]
struct HttpRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
