// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

use nautilus_model::data::BarType;
use nautilus_papertrade::{
    api::PapertradeApi,
    reference_bars::{ReferenceDailyWindow, decode_daily_bars, fetch_daily_bars},
    wire::{Market, TradingState},
};
use parking_lot::Mutex;
use rstest::rstest;
use serde_json::{Value, json};
use std::{str::FromStr, sync::Arc};

const DAY: u64 = 86_400_000;
fn market() -> Market {
    let trading: TradingState =
        serde_json::from_str(include_str!("../test_data/live_trading.json")).unwrap();
    trading
        .instruments
        .into_iter()
        .find(|market| market.symbol == "BTC")
        .unwrap()
}
fn bar_type() -> BarType {
    BarType::from_str("BTC-USD-PERP.PAPERTRADE-1-DAY-LAST-EXTERNAL").unwrap()
}
fn window() -> ReferenceDailyWindow {
    ReferenceDailyWindow::new(100 * DAY, 103 * DAY, 103 * DAY + 500).unwrap()
}
fn candle(day: u64) -> Value {
    json!({"t":day*DAY,"T":(day+1)*DAY-1,"s":"BTC","i":"1d",
        "o":"25000.1","h":"26000.2","l":"24000.0","c":"25500.0","v":"0.0","n":0})
}
fn rows() -> Value {
    json!([candle(100), candle(101), candle(102)])
}

#[test]
fn daily_bars_preserve_provider_prices_and_open_close_timestamp_semantics() {
    let bars = decode_daily_bars(&rows(), &market(), bar_type(), window()).unwrap();
    assert_eq!(bars.len(), 3);
    assert_eq!(bars[0].open.to_string(), "25000.1");
    assert_eq!(bars[0].ts_event.as_u64(), 100 * DAY * 1_000_000);
    assert_eq!(bars[0].ts_init.as_u64(), 101 * DAY * 1_000_000);
    assert_eq!(bars[2].ts_init.as_u64(), 103 * DAY * 1_000_000);
    assert_eq!(bars[0].volume.as_decimal(), rust_decimal::Decimal::ZERO);
}

#[test]
fn unordered_complete_source_is_returned_in_canonical_time_order() {
    let data = json!([candle(102), candle(100), candle(101)]);
    let bars = decode_daily_bars(&data, &market(), bar_type(), window()).unwrap();
    assert!(
        bars.windows(2)
            .all(|pair| pair[0].ts_event < pair[1].ts_event)
    );
}

#[rstest]
#[case("missing")]
#[case("duplicate")]
#[case("forming")]
#[case("symbol")]
#[case("interval")]
#[case("close_boundary")]
#[case("zero_price")]
#[case("price_precision")]
#[case("price_long_tail")]
#[case("volume_positive_underflow")]
#[case("volume_negative_underflow")]
#[case("scientific_price")]
#[case("negative_volume")]
#[case("numeric_price")]
#[case("inverted_ohlc")]
#[case("empty")]
fn incomplete_or_malformed_source_never_becomes_a_complete_daily_window(#[case] fault: &str) {
    let mut data = rows();
    match fault {
        "missing" => {
            data.as_array_mut().unwrap().remove(1);
        }
        "duplicate" => {
            data[1] = candle(100);
        }
        "forming" => {
            data.as_array_mut().unwrap().push(candle(103));
        }
        "symbol" => {
            data[1]["s"] = json!("ETH");
        }
        "interval" => {
            data[1]["i"] = json!("1h");
        }
        "close_boundary" => {
            data[1]["T"] = json!(102 * DAY - 2);
        }
        "zero_price" => {
            data[1]["l"] = json!("0");
        }
        "price_precision" => {
            data[1]["o"] = json!("25000.11");
        }
        "price_long_tail" => {
            data[1]["o"] = json!("25000.1000000000000000000000000000001");
        }
        "volume_positive_underflow" => {
            data[1]["v"] = json!("0.0000000000000000000000000000001");
        }
        "volume_negative_underflow" => {
            data[1]["v"] = json!("-0.0000000000000000000000000000001");
        }
        "scientific_price" => {
            data[1]["o"] = json!("2.5e4");
        }
        "negative_volume" => {
            data[1]["v"] = json!("-1");
        }
        "numeric_price" => {
            data[1]["o"] = json!(25000.1);
        }
        "inverted_ohlc" => {
            data[1]["h"] = json!("20000.0");
        }
        "empty" => {
            data = json!([]);
        }
        _ => unreachable!(),
    }
    assert!(
        decode_daily_bars(&data, &market(), bar_type(), window()).is_err(),
        "{fault}"
    );
}

#[rstest]
#[case(100*DAY+1,103*DAY,103*DAY)]
#[case(100*DAY,103*DAY+1,104*DAY)]
#[case(103*DAY,103*DAY,103*DAY)]
#[case(104*DAY,103*DAY,105*DAY)]
#[case(100*DAY,104*DAY,103*DAY)]
#[case(0,5001*DAY,5001*DAY)]
#[case(0,DAY,5002*DAY)]
fn invalid_or_unavailable_window_is_rejected_before_any_request(
    #[case] start: u64,
    #[case] end: u64,
    #[case] now: u64,
) {
    assert!(ReferenceDailyWindow::new(start, end, now).is_err());
}

#[rstest]
#[case("BTC-USD-PERP.PAPERTRADE-1-HOUR-LAST-EXTERNAL")]
#[case("BTC-USD-PERP.PAPERTRADE-1-DAY-MID-EXTERNAL")]
#[case("BTC-USD-PERP.PAPERTRADE-1-DAY-LAST-INTERNAL")]
#[case("ETH-USD-PERP.PAPERTRADE-1-DAY-LAST-EXTERNAL")]
fn unsupported_bar_semantics_are_not_silently_substituted(#[case] name: &str) {
    assert!(
        decode_daily_bars(
            &rows(),
            &market(),
            BarType::from_str(name).unwrap(),
            window()
        )
        .is_err()
    );
}

#[test]
fn terminal_market_does_not_supply_live_strategy_daily_bars() {
    let mut market = market();
    market.terminal = true;
    assert!(decode_daily_bars(&rows(), &market, bar_type(), window()).is_err());
}

#[tokio::test]
async fn daily_request_uses_the_public_reference_endpoint_and_exact_closed_range() {
    use axum::{Json, Router, extract::State, routing::post};
    async fn reply(
        State(requests): State<Arc<Mutex<Vec<Value>>>>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        requests.lock().push(body);
        Json(rows())
    }
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route("/", post(reply))
        .with_state(requests.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let api = PapertradeApi::new(url.parse().unwrap(), url.parse().unwrap(), 1)
        .unwrap()
        .with_reference(url.parse().unwrap());
    let bars = fetch_daily_bars(&api, &market(), bar_type(), window())
        .await
        .unwrap();
    assert_eq!(bars.len(), 3);
    assert_eq!(
        requests.lock().as_slice(),
        &[
            json!({"type":"candleSnapshot","req":{"coin":"BTC","interval":"1d","startTime":100*DAY,"endTime":103*DAY-1}})
        ]
    );
    task.abort();
}

#[tokio::test]
async fn reference_http_failure_is_not_an_authoritative_empty_history() {
    use axum::{Json, Router, http::StatusCode, routing::post};
    let app = Router::new().route(
        "/",
        post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, Json(rows())) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let api = PapertradeApi::new(url.parse().unwrap(), url.parse().unwrap(), 1)
        .unwrap()
        .with_reference(url.parse().unwrap());
    assert!(
        fetch_daily_bars(&api, &market(), bar_type(), window())
            .await
            .is_err()
    );
    task.abort();
}

#[tokio::test]
#[ignore = "reads current public reference APIs; run explicitly"]
async fn public_reference_supplies_the_closed_two_thousand_day_warmup_window() {
    let api = PapertradeApi::new(
        "https://exchange.papertrade.xyz/".parse().unwrap(),
        "https://rpc.hyperliquid.xyz/evm".parse().unwrap(),
        1,
    )
    .unwrap();
    let trading = api.trading().await.unwrap();
    let market = trading
        .instruments
        .iter()
        .find(|market| market.symbol == "BTC")
        .unwrap();
    let now = nautilus_core::time::get_atomic_clock_realtime()
        .get_time_ns()
        .as_u64()
        / 1_000_000;
    let end = now / DAY * DAY;
    let window = ReferenceDailyWindow::new(end - 2000 * DAY, end, now).unwrap();
    let bars = fetch_daily_bars(&api, market, bar_type(), window)
        .await
        .unwrap();
    assert_eq!(bars.len(), 2000);
    assert_eq!(
        bars.first().unwrap().ts_event.as_u64(),
        window.start_ms() * 1_000_000
    );
    assert_eq!(bars.last().unwrap().ts_init.as_u64(), end * 1_000_000);
    println!(
        "public_reference_closed_daily_bars={} first_open_ms={} last_close_ms={}",
        bars.len(),
        window.start_ms(),
        end
    );
}
