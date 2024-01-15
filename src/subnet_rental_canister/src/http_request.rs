use std::time::Duration;

use candid::CandidType;
use ic_cdk::query;
use serde::Deserialize;

use crate::{list_rental_agreements, list_rental_conditions, RENTAL_AGREEMENTS, TRILLION};

const HTML_HEAD: &str =
    r#"<!DOCTYPE html><html lang="en"><head><title>Subnet Rental Canister</title></head>"#;

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
        r#"<body><h1>Rental Agreements</h1><table border="1"><tr><th>Subnet ID</th><th>Renter</th><th>Allowed Principals</th><th>Billing Period (days)</th><th>Initial Rental Period (days)</th><th>Daily Cost (XDR)</th><th>Creation Date</th><th>Status</th></tr>"#,
    );
    for agreement in rental_agreements {
        html.push_str("<tr>");
        html.push_str(&format!(
            "<td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:?}</td><td>{}</td>",
            agreement.subnet_id.0,
            agreement.user.0,
            agreement
                .principals
                .iter()
                .map(|p| p.0.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            agreement.rental_conditions.billing_period_days,
            agreement.rental_conditions.initial_rental_period_days,
            agreement.rental_conditions.daily_cost_cycles / TRILLION,
            Duration::from_nanos(agreement.creation_date),
            "Healthy"
        ));
        html.push_str("</tr>");
    }
    html.push_str("</table></body></html>");
    html
}

fn generate_rental_conditions_html() -> String {
    let rental_conditions = list_rental_conditions();

    let mut html = String::new();
    html.push_str(HTML_HEAD);
    html.push_str(
        r#"<body><h1>Subnets for Rent</h1><table border="1"><tr><th>Subnet ID</th><th>Daily Cost (XDR)</th><th>Minimal Rental Period (days)</th><th>Billing Period (days)</th><th>Status</th></tr>"#,
    );
    for (subnet_id, conditions) in rental_conditions {
        html.push_str("<tr>");
        let rented = RENTAL_AGREEMENTS.with(|map| map.borrow().contains_key(&subnet_id));
        html.push_str(&format!(
            "<td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>",
            subnet_id.0,
            conditions.daily_cost_cycles / TRILLION,
            conditions.initial_rental_period_days,
            conditions.billing_period_days,
            if rented { "Rented" } else { "Available" }
        ));
        html.push_str("</tr>");
    }
    html.push_str("</table></body></html>");
    html
}
