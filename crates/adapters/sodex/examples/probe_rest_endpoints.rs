//! Finds the venue's account, order and fill read endpoints by asking it which paths exist.
//!
//! These three reads are what stands between this adapter and unattended running. Nautilus
//! builds reconciliation on `generate_order_status_reports`, `generate_fill_reports` and
//! `generate_position_status_reports`, all of which are request/response - so the account
//! WebSocket channel is a latency optimization, not a prerequisite. What *is* a prerequisite is
//! knowing the paths, and guessing them is the mistake this integration already paid for once:
//! a wrong path produced an error that read like a credential problem.
//!
//! # The router is per method, which the control caught
//!
//! A first version of this probe sent only `GET` and concluded that almost nothing existed. Its
//! positive control refuted that: `GET /trade/orders` also answered `404 page not found`, and
//! that path certainly exists - orders are placed on it with `POST`. The gateway registers
//! routes per method and does not return `405`, so a `404` means "this method is not routed
//! here" and says nothing about the path.
//!
//! So each path is asked with each method, and a path counts as served when **any** method
//! answers something other than `404`.
//!
//! # Why the writes here cannot write
//!
//! Every request is unsigned. Signed endpoints reject on the missing signature before any
//! business logic runs, so an unsigned `POST` or `DELETE` is inert - it can only produce the
//! error that tells us the route exists. The candidate list is also read-shaped on purpose: no
//! bulk-mutation path (`cancel-all`, `close-all`, `schedule-cancel`) is probed at all, because
//! an endpoint that acts on no parameters would not be protected by an empty body.
//!
//! ```text
//! cargo run -p nautilus-sodex --example probe_rest_endpoints
//! SODEX_MARKET=perps cargo run -p nautilus-sodex --example probe_rest_endpoints
//! ```

use std::{collections::BTreeMap, env};

use nautilus_network::http::{HttpClient, Method};
use nautilus_sodex::{common::Market, http::Network};

/// Resources to ask about under the address-parameterized account namespace.
///
/// This namespace is where the lead was. One endpoint in it is already documented and working -
/// `GET /{engine}/accounts/{address}/api-keys` - and its shape explains why a bare `/accounts`
/// or `/accounts/balances` answers 404: the path carries the address as a segment. The earlier
/// sweep missed it by probing the namespace without the parameter.
const ACCOUNT_RESOURCES: &[&str] = &[
    // The documented one, as the positive control for this whole namespace.
    "api-keys",
    // Balances and holdings.
    "balances",
    "balance",
    "assets",
    "info",
    "summary",
    "overview",
    "detail",
    "",
    // Orders and fills.
    "orders",
    "open-orders",
    "order-history",
    "orders/open",
    "orders/history",
    "fills",
    "trades",
    "my-trades",
    "trade-history",
    "executions",
    // Perps concerns.
    "positions",
    "leverage",
    "margin",
    "margin-mode",
];

/// Paths to ask about, relative to the market base (`/api/v1/{spot,perps}`).
const CANDIDATE_PATHS: &[&str] = &[
    // Known to exist on POST and DELETE, as the positive control.
    "/trade/orders",
    // Open and historical orders.
    "/trade/orders/open",
    "/trade/orders/history",
    "/trade/orders/active",
    "/trade/orders/query",
    "/trade/order",
    "/trade/openOrders",
    "/trade/orderHistory",
    "/orders",
    "/orders/open",
    "/orders/history",
    // Fills.
    "/trade/fills",
    "/trade/myTrades",
    "/trade/trades",
    "/trade/executions",
    "/fills",
    "/myTrades",
    "/userTrades",
    // Account and balances.
    "/account",
    "/account/info",
    "/account/balance",
    "/account/balances",
    "/account/assets",
    "/accounts",
    "/balance",
    "/balances",
    "/assets",
    "/user",
    "/user/info",
    "/wallet",
    // Positions and leverage.
    "/positions",
    "/position",
    "/account/positions",
    "/trade/positions",
    "/leverage",
    // Kebab-case, which is the convention the venue's own paths use
    // (`/accounts/api-keys`, `/trade/orders/schedule-cancel`). The first sweep asked in
    // camelCase throughout and so could not have found these.
    "/trade/open-orders",
    "/trade/order-history",
    "/trade/my-trades",
    "/trade/trade-history",
    "/trade/order-detail",
    "/trade/orders/schedule-cancel",
    // Market data, as further controls.
    "/markets/symbols",
];

/// Methods to ask with. None of these can act, because none is signed.
const METHODS: &[(&str, Method)] = &[
    ("GET", Method::GET),
    ("POST", Method::POST),
    ("DELETE", Method::DELETE),
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let network = match env::var("SODEX_NETWORK").as_deref() {
        Ok("mainnet") => Network::Mainnet,
        _ => Network::Testnet,
    };
    let market = match env::var("SODEX_MARKET").as_deref() {
        Ok("perps") => Market::Perps,
        _ => Market::Spot,
    };
    let base = network.market_base(market);

    let http = HttpClient::builder().timeout_secs(20).build()?;
    println!("probing {base} - unsigned, so no request here can act");
    println!();

    // path -> method -> (status, body excerpt)
    let mut served: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unserved: Vec<String> = Vec::new();

    for path in CANDIDATE_PATHS {
        let mut hits = Vec::new();

        for (label, method) in METHODS {
            let headers = std::collections::HashMap::from([
                ("Accept".to_string(), "application/json".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ]);
            // An empty JSON object: enough to be a well-formed body, not enough to be an action.
            let body = (*method != Method::GET).then(|| b"{}".to_vec());

            let response = http
                .request(
                    method.clone(),
                    format!("{base}{path}"),
                    None,
                    Some(headers),
                    body,
                    None,
                    None,
                )
                .await?;

            let status = response.status.as_u16();
            if status == 404 {
                continue;
            }
            let body = String::from_utf8_lossy(&response.body);
            hits.push(format!("{label} {status} {:.2000}", body.trim()));
        }

        if hits.is_empty() {
            unserved.push((*path).to_string());
        } else {
            served.insert((*path).to_string(), hits);
        }
    }

    // The address-parameterized namespace, which the flat sweep structurally cannot reach.
    let address = env::var("SODEX_ADDRESS")
        .unwrap_or_else(|_| "0x766a478C89E5E9354b7a23922De18da6A5163b00".to_string());
    let account_id = env::var("SODEX_ACCOUNT_ID").unwrap_or_else(|_| "60366".to_string());

    for key in [address.as_str(), account_id.as_str()] {
        for resource in ACCOUNT_RESOURCES {
            let path = if resource.is_empty() {
                format!("/accounts/{key}")
            } else {
                format!("/accounts/{key}/{resource}")
            };
            let mut hits = Vec::new();

            for (label, method) in METHODS {
                let headers = std::collections::HashMap::from([
                    ("Accept".to_string(), "application/json".to_string()),
                    ("Content-Type".to_string(), "application/json".to_string()),
                ]);
                let body = (*method != Method::GET).then(|| b"{}".to_vec());

                let response = http
                    .request(
                        method.clone(),
                        format!("{base}{path}"),
                        None,
                        Some(headers),
                        body,
                        None,
                        None,
                    )
                    .await?;

                let status = response.status.as_u16();
                if status == 404 {
                    continue;
                }
                let body = String::from_utf8_lossy(&response.body);
                hits.push(format!("{label} {status} {:.2000}", body.trim()));
            }

            if hits.is_empty() {
                unserved.push(path);
            } else {
                served.insert(path, hits);
            }
        }
    }

    println!("== served (some method answered other than 404) ==");
    for (path, hits) in &served {
        println!("  {path}");
        for hit in hits {
            println!("      {hit}");
        }
    }
    println!();
    println!("== no method answered other than 404 ==");
    for path in &unserved {
        println!("  {path}");
    }
    println!();
    println!(
        "controls: /trade/orders and /accounts/<address>/api-keys must both appear as served,\n\
         or this classification is worthless"
    );

    Ok(())
}
