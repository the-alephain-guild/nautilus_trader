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

//! Canonical replay capability tests over a tempdir-backed redb run.

#[cfg(feature = "persistence")]
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use indexmap::IndexMap;
use nautilus_core::{UnixNanos, time::get_atomic_clock_static};
#[cfg(feature = "persistence")]
use nautilus_event_store::{
    CatalogReplayData, CatalogReplayRecord, CatalogSliceCoverage, CatalogSlicePlan,
    CatalogSliceQuery, DataClass, DataMarkerCapture, DataMarkerClass, DataMarkerConfig,
    DataMarkerExtractorRegistry, MarkerBackend, MarkerManifest, MarkerReader, MarkerWriter,
    MarkerWriterConfig, RedbMarkerBackend, ReplayCatalog, join_at_entry,
};
use nautilus_event_store::{
    EntryDraft, EventStore, EventStoreWriter, Headers, RedbBackend, RegisteredComponents,
    ReplaySeqRange, ReplayTimeRange, RunManifest, RunStatus, Topic, WriterConfig,
    load_forensics_replay_inputs, noop_halt, open_event_store_replay_source,
    plan_forensics_replay_inputs,
};
#[cfg(feature = "persistence")]
use nautilus_model::{data::QuoteTick, data::stubs::quote_ethusdt_binance};
use rstest::rstest;
use tempfile::TempDir;
use ustr::Ustr;

const INSTANCE_ID: &str = "trader-001";

fn manifest(run_id: &str) -> RunManifest {
    RunManifest {
        run_id: run_id.to_string(),
        parent_run_id: None,
        instance_id: INSTANCE_ID.to_string(),
        binary_hash: "deadbeef".to_string(),
        schema_version: 1,
        crate_versions: "feedface".to_string(),
        feature_flags: Vec::new(),
        adapter_versions: IndexMap::new(),
        config_hash: "cafebabe".to_string(),
        registered_components: RegisteredComponents::default(),
        seed: None,
        start_ts_init: UnixNanos::from(0),
        end_ts_init: None,
        high_watermark: 0,
        status: RunStatus::Running,
    }
}

fn entry_draft(
    topic: &'static str,
    payload_type: &'static str,
    payload: &'static [u8],
    ts_init: u64,
) -> EntryDraft {
    EntryDraft {
        headers: Headers::empty(),
        topic: Topic::from(topic),
        payload_type: Ustr::from(payload_type),
        payload: Bytes::from_static(payload),
        ts_init: UnixNanos::from(ts_init),
        index_keys: Vec::new(),
    }
}

fn run_ended_draft() -> EntryDraft {
    EntryDraft {
        headers: Headers::empty(),
        topic: Topic::from("run.lifecycle.RunEnded"),
        payload_type: Ustr::from("RunEnded"),
        payload: Bytes::new(),
        ts_init: UnixNanos::from(130),
        index_keys: Vec::new(),
    }
}

fn seed_sealed_run(tmp: &TempDir, run_id: &str) -> u64 {
    let mut backend = RedbBackend::new(tmp.path());
    backend.open_run(manifest(run_id)).expect("open run");

    let writer = EventStoreWriter::spawn(
        Box::new(backend),
        get_atomic_clock_static(),
        noop_halt(),
        WriterConfig::default(),
    )
    .expect("spawn");

    writer
        .submit(entry_draft(
            "run.lifecycle.RunStarted",
            "RunStarted",
            b"started",
            100,
        ))
        .expect("submit started");
    writer
        .submit(entry_draft(
            "exec.command.SubmitOrder",
            "SubmitOrder",
            b"submit",
            80,
        ))
        .expect("submit command");
    writer
        .submit(entry_draft(
            "events.order.OrderFilled.S-001",
            "OrderFilled",
            b"filled",
            120,
        ))
        .expect("submit event");

    writer.close(run_ended_draft()).expect("close")
}

#[cfg(feature = "persistence")]
struct StubReplayCatalog {
    records: Vec<CatalogReplayRecord>,
    plan_queries: Vec<CatalogSliceQuery>,
    load_plans: Vec<CatalogSlicePlan>,
}

#[cfg(feature = "persistence")]
impl StubReplayCatalog {
    fn new(records: Vec<CatalogReplayRecord>) -> Self {
        Self {
            records,
            plan_queries: Vec::new(),
            load_plans: Vec::new(),
        }
    }
}

#[cfg(feature = "persistence")]
impl ReplayCatalog for StubReplayCatalog {
    type Error = String;

    fn plan_slice(
        &mut self,
        query: &CatalogSliceQuery,
    ) -> Result<CatalogSliceCoverage, Self::Error> {
        self.plan_queries.push(query.clone());
        Ok(CatalogSliceCoverage::from_files(vec![format!(
            "{}/{}",
            query.data_cls,
            query.identifiers.join(",")
        )]))
    }

    fn load_slice(
        &mut self,
        plan: &CatalogSlicePlan,
    ) -> Result<Vec<CatalogReplayRecord>, Self::Error> {
        self.load_plans.push(plan.clone());
        Ok(self
            .records
            .iter()
            .filter(|record| catalog_record_matches_query(record, &plan.query))
            .cloned()
            .collect())
    }
}

#[cfg(feature = "persistence")]
fn catalog_record_matches_query(record: &CatalogReplayRecord, query: &CatalogSliceQuery) -> bool {
    record.data_cls == query.data_cls
        && record
            .identifier
            .as_ref()
            .is_some_and(|identifier| query.identifiers.contains(identifier))
        && record.ts_init >= query.start
        && record.ts_init <= query.end
}

#[cfg(feature = "persistence")]
fn marker_manifest(run_id: &str) -> MarkerManifest {
    MarkerManifest {
        run_id: run_id.to_string(),
        enabled_classes: vec![DataClass::Quote],
        high_fidelity: false,
        snapshot_count: 0,
        hifi_count: 0,
        gap_count: 0,
        dict_count: 0,
        status: RunStatus::Running,
    }
}

#[cfg(feature = "persistence")]
fn marker_config() -> DataMarkerConfig {
    DataMarkerConfig {
        classes: vec![DataMarkerClass::Quote],
        high_fidelity: Vec::new(),
        safety_flush_interval: Duration::from_secs(1),
        channel_capacity: 64,
    }
}

#[cfg(feature = "persistence")]
fn quote_tick(ts_init: u64) -> QuoteTick {
    let mut quote = quote_ethusdt_binance();
    quote.ts_event = UnixNanos::from(ts_init);
    quote.ts_init = UnixNanos::from(ts_init);
    quote
}

#[cfg(feature = "persistence")]
fn catalog_quote_record(ts_init: u64) -> CatalogReplayRecord {
    CatalogReplayRecord::from_data(CatalogReplayData::Quote(quote_tick(ts_init)))
}

#[rstest]
fn forensics_replay_loads_entries_from_sealed_redb_run() {
    let tmp = TempDir::new().expect("tempdir");
    let run_id = "run-forensics-replay";
    let final_hwm = seed_sealed_run(&tmp, run_id);

    let (manifest, reader) =
        open_event_store_replay_source(tmp.path().to_path_buf(), INSTANCE_ID, run_id)
            .expect("open replay source");
    let replay_range = ReplaySeqRange::new(1, final_hwm);
    let plan = plan_forensics_replay_inputs(&reader, replay_range).expect("plan replay");
    let loaded = load_forensics_replay_inputs(&reader, &plan).expect("load replay");

    assert_eq!(final_hwm, 4);
    assert_eq!(manifest.status, RunStatus::Ended);
    assert_eq!(manifest.high_watermark, final_hwm);
    assert_eq!(plan.requested_range, replay_range);
    assert_eq!(plan.event_range, Some(replay_range));
    assert_eq!(plan.event_count, 4);
    assert_eq!(
        plan.event_time_range,
        Some(ReplayTimeRange::new(
            UnixNanos::from(80),
            UnixNanos::from(130),
        )),
    );
    assert!(plan.catalog_slices.is_empty());

    let seqs: Vec<u64> = loaded.entries.iter().map(|entry| entry.seq).collect();
    let payload_types: Vec<&str> = loaded
        .entries
        .iter()
        .map(|entry| entry.payload_type.as_str())
        .collect();

    assert_eq!(seqs, vec![1, 2, 3, 4]);
    assert_eq!(
        payload_types,
        vec!["RunStarted", "SubmitOrder", "OrderFilled", "RunEnded"],
    );
    assert!(
        loaded
            .entries
            .iter()
            .all(|entry| entry.recompute_hash() == entry.entry_hash),
        "loaded entries must preserve their canonical hashes",
    );
    assert!(loaded.catalog_slices.is_empty());
}

#[cfg(feature = "persistence")]
#[rstest]
fn marker_cursor_join_loads_catalog_records_from_durable_sidecar() {
    let tmp = TempDir::new().expect("tempdir");
    let run_id = "run-marker-catalog-join";
    let marker_path = tmp
        .path()
        .join(INSTANCE_ID)
        .join(format!("{run_id}.markers.redb"));
    let mut backend = RedbMarkerBackend::new(&marker_path);
    backend
        .open_run(marker_manifest(run_id))
        .expect("open marker run");

    let writer = MarkerWriter::spawn(
        Box::new(backend),
        get_atomic_clock_static(),
        MarkerWriterConfig {
            channel_capacity: 64,
            max_batch: 1,
            max_latency: Duration::from_millis(1),
        },
    )
    .expect("spawn marker writer");
    let submit_counter = Arc::new(AtomicU64::new(0));
    let mut capture = DataMarkerCapture::new(
        DataMarkerExtractorRegistry::default_registry(&[DataClass::Quote]),
        writer,
        Arc::clone(&submit_counter),
        &marker_config(),
    );
    let topic = Topic::from("data.quotes.BINANCE.ETHUSDT-PERP");

    capture.observe_publish(topic, &quote_tick(10), UnixNanos::from(10));
    capture.observe_publish(topic, &quote_tick(20), UnixNanos::from(20));
    submit_counter.store(7, Ordering::Release);
    capture.on_entry_submitted(UnixNanos::from(25));
    capture.close();

    let marker = RedbMarkerBackend::open_read_only_file(&marker_path).expect("open marker sidecar");
    assert_eq!(
        marker.manifest().expect("marker manifest").status,
        RunStatus::Ended,
    );
    let reader = MarkerReader::new(Box::new(marker));
    let mut catalog = StubReplayCatalog::new(vec![
        catalog_quote_record(10),
        catalog_quote_record(20),
        catalog_quote_record(30),
    ]);

    let joined = join_at_entry(&reader, &mut catalog, 7).expect("join marker cursor");

    assert_eq!(joined.len(), 1);
    assert_eq!(joined[0].entry.data_cls, DataClass::Quote);
    assert_eq!(joined[0].entry.identifier, "ETHUSDT-PERP.BINANCE");
    assert_eq!(joined[0].cursor.count, 2);
    assert_eq!(joined[0].cursor.ts_init_hi, UnixNanos::from(20));
    assert!(!joined[0].candidate);
    assert_eq!(
        joined[0]
            .records
            .iter()
            .map(|record| record.ts_init)
            .collect::<Vec<_>>(),
        vec![UnixNanos::from(10), UnixNanos::from(20)],
    );
    assert_eq!(catalog.plan_queries.len(), 1);
    assert_eq!(catalog.plan_queries[0].data_cls, "quotes");
    assert_eq!(
        catalog.plan_queries[0].identifiers,
        vec!["ETHUSDT-PERP.BINANCE".to_string()],
    );
    assert_eq!(catalog.plan_queries[0].end, UnixNanos::from(20));
    assert_eq!(catalog.load_plans.len(), 1);
}

#[rstest]
fn runtime_report_wire_reopens_redb_and_replays_into_fresh_kernel() {
    use std::{cell::RefCell, rc::Rc, sync::Arc};

    use nautilus_common::{
        enums::Environment,
        messages::ExecutionReport,
        msgbus::{self, BusTap, MessagingSwitchboard, TypedHandler},
        owner_delivery::OwnerDelivery,
    };
    use nautilus_core::UUID4;
    use nautilus_event_store::{BusCaptureAdapter, default_registry};
    use nautilus_model::{
        accounts::{AccountAny, CashAccount},
        enums::{
            AccountType, LiquiditySide, OmsType, OrderSide, OrderStatus, OrderType, PositionSide,
            TimeInForce,
        },
        events::{
            AccountState, OrderAccepted, OrderEventAny, OrderFilled, PositionChanged,
            PositionEvent, PositionOpened,
        },
        identifiers::{AccountId, ClientId, ClientOrderId, TradeId, TraderId, Venue, VenueOrderId},
        instruments::{Instrument, stubs::audusd_sim},
        orders::{Order, OrderAny, builder::OrderTestBuilder, stubs::TestOrderEventStubs},
        reports::{ExecutionMassStatus, FillReport, OrderStatusReport, PositionStatusReport},
        types::{AccountBalance, Currency, Money, Price, Quantity},
    };
    use nautilus_system::kernel::NautilusKernel;

    #[derive(serde::Deserialize)]
    struct BundledReport {
        order_report: OrderStatusReport,
        fill_reports: Vec<FillReport>,
    }

    fn report_wire(report: &ExecutionReport) -> Vec<u8> {
        match report {
            ExecutionReport::Order(value) => rmp_serde::to_vec_named(&**value).unwrap(),
            ExecutionReport::Fill(value) => rmp_serde::to_vec_named(&**value).unwrap(),
            ExecutionReport::Position(value) => rmp_serde::to_vec_named(&**value).unwrap(),
            ExecutionReport::MassStatus(value) => rmp_serde::to_vec_named(&**value).unwrap(),
            ExecutionReport::OrderWithFills(order, fills) => {
                #[derive(serde::Serialize)]
                struct Wire<'a> {
                    order_report: &'a OrderStatusReport,
                    fill_reports: &'a [FillReport],
                }
                rmp_serde::to_vec_named(&Wire {
                    order_report: order,
                    fill_reports: fills,
                })
                .unwrap()
            }
        }
    }

    fn fresh_kernel(order: &OrderAny) -> NautilusKernel {
        let kernel = NautilusKernel::builder(
            "durable-report-replay".into(),
            TraderId::from("REPLAY-001"),
            Environment::Backtest,
        )
        .build()
        .expect("fresh actual Kernel and Portfolio");
        let account_id = AccountId::from("SIM-001");
        kernel
            .cache
            .borrow_mut()
            .add_instrument(audusd_sim().into())
            .unwrap();
        let state = AccountState::new(
            account_id,
            AccountType::Cash,
            vec![AccountBalance::new(
                Money::from("1000000 USD"),
                Money::from("0 USD"),
                Money::from("1000000 USD"),
            )],
            vec![],
            true,
            UUID4::new(),
            0.into(),
            0.into(),
            Some(Currency::USD()),
        );
        kernel
            .cache
            .borrow_mut()
            .add_account(CashAccount::new(state, true, false).into())
            .unwrap();
        kernel
            .exec_engine
            .borrow_mut()
            .register_oms_type(order.strategy_id(), OmsType::Netting);
        kernel
            .cache
            .borrow_mut()
            .add_order(order.clone(), None, None, false)
            .unwrap();
        kernel
            .exec_engine
            .borrow_mut()
            .process(&TestOrderEventStubs::submitted(order, account_id));
        kernel
    }

    fn business_snapshot(
        kernel: &NautilusKernel,
        id: ClientOrderId,
    ) -> (Vec<Vec<u8>>, Vec<u8>, Vec<u8>, Vec<OrderFilled>) {
        let cache = kernel.cache.borrow();
        let order = cache.order(&id).unwrap();
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(order.quantity(), Quantity::from(100_000));
        assert_eq!(order.filled_qty(), Quantity::from(100_000));
        assert_eq!(order.trade_ids().len(), 2);
        assert_eq!(
            order.commissions().get(&Currency::USD()),
            Some(&Money::from("0.20 USD"))
        );
        let fills: Vec<_> = order
            .events()
            .into_iter()
            .filter_map(|event| match event {
                OrderEventAny::Filled(fill) => Some(fill.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            fills.iter().map(|fill| fill.trade_id).collect::<Vec<_>>(),
            vec![
                TradeId::from("DURABLE-TRADE-A"),
                TradeId::from("DURABLE-TRADE-B")
            ]
        );
        let positions = cache.positions(
            None,
            Some(&audusd_sim().id()),
            None,
            Some(&AccountId::from("SIM-001")),
            None,
        );
        assert_eq!(positions.len(), 1);
        let position = &positions[0];
        assert_eq!(position.side, PositionSide::Long);
        assert_eq!(position.quantity, Quantity::from(100_000));
        assert_eq!(position.avg_px_open, 1.0);
        assert_eq!(position.commissions(), vec![Money::from("0.20 USD")]);
        let AccountAny::Cash(account) = cache.account_owned(&AccountId::from("SIM-001")).unwrap()
        else {
            panic!("real cash account")
        };
        // Two buys of 50,000 at 1 USD consume 100,000 USD principal and 0.20 USD fees.
        assert_eq!(
            account.base.base_balance_total(Some(Currency::USD())),
            Some(Money::from("899999.80 USD"))
        );
        assert_eq!(
            account.base.commission(&Currency::USD()),
            Some(Money::from("0.20 USD"))
        );
        (
            order
                .events()
                .iter()
                .map(|event| rmp_serde::to_vec_named(event).unwrap())
                .collect(),
            rmp_serde::to_vec_named(&account).unwrap(),
            rmp_serde::to_vec_named(&**position).unwrap(),
            fills,
        )
    }

    let instrument = audusd_sim();
    let id = ClientOrderId::from("DURABLE-REPORT-ORDER");
    let account_id = AccountId::from("SIM-001");
    let venue_id = VenueOrderId::from("V-DURABLE-REPORT");
    let order = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(TraderId::from("REPLAY-001"))
        .instrument_id(instrument.id())
        .client_order_id(id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .price(Price::from("1.00000"))
        .build();
    let accepted = OrderStatusReport::new(
        account_id,
        instrument.id(),
        Some(id),
        venue_id,
        Some(OrderSide::Buy),
        OrderType::Limit,
        TimeInForce::Gtc,
        OrderStatus::Accepted,
        Quantity::from(100_000),
        Quantity::from(0),
        10.into(),
        10.into(),
        10.into(),
        None,
    )
    .with_price(Price::from("1.00000"));
    let mut filled = accepted.clone();
    filled.report_id = UUID4::new();
    filled.order_status = OrderStatus::Filled;
    filled.filled_qty = Quantity::from(100_000);
    filled.avg_px = Some("1.00000".parse().unwrap());
    filled.ts_last = 12.into();
    let fills: Vec<_> = ["DURABLE-TRADE-A", "DURABLE-TRADE-B"]
        .into_iter()
        .enumerate()
        .map(|(index, trade)| {
            FillReport::new(
                account_id,
                instrument.id(),
                venue_id,
                TradeId::from(trade),
                OrderSide::Buy,
                Quantity::from(50_000),
                Price::from("1.00000"),
                Money::from("0.10 USD"),
                LiquiditySide::Maker,
                Some(id),
                None,
                (11 + index as u64).into(),
                (11 + index as u64).into(),
                None,
            )
        })
        .collect();
    let position = PositionStatusReport::new(
        account_id,
        instrument.id(),
        PositionSide::Long,
        Quantity::from(100_000),
        13.into(),
        13.into(),
        None,
        None,
        Some("1.00000".parse().unwrap()),
    );
    let mut mass = ExecutionMassStatus::new(
        ClientId::from("SIM"),
        account_id,
        Venue::from("SIM"),
        14.into(),
        None,
    );
    mass.add_order_reports(vec![filled.clone()]);
    mass.add_fill_reports(fills.clone());
    mass.add_position_reports(vec![position.clone()]);
    let reports = vec![
        ExecutionReport::Order(Box::new(accepted)),
        ExecutionReport::OrderWithFills(Box::new(filled), fills.clone()),
        ExecutionReport::Fill(Box::new(fills[0].clone())),
        ExecutionReport::Position(Box::new(position)),
        ExecutionReport::MassStatus(Box::new(mass.clone())),
    ];
    let tmp = TempDir::new().unwrap();
    let run_id = "runtime-report-durable-wire";
    let mut backend = RedbBackend::new(tmp.path());
    backend.open_run(manifest(run_id)).unwrap();
    let writer = Arc::new(
        EventStoreWriter::spawn(
            Box::new(backend),
            get_atomic_clock_static(),
            noop_halt(),
            WriterConfig::default(),
        )
        .unwrap(),
    );
    let registry = Arc::new(default_registry());
    let adapter = Arc::new(BusCaptureAdapter::new(
        writer.clone(),
        registry.clone(),
        noop_halt(),
    ));
    let mut kernel = fresh_kernel(&order);
    let tap: Rc<dyn BusTap> = Rc::new(super::capture::HeadersAwareAdapterTap {
        adapter: adapter.clone(),
        registry,
    });
    msgbus::set_bus_tap(tap);
    let actual_positions = Rc::new(RefCell::new(Vec::new()));
    let position_handler = TypedHandler::from({
        let actual = actual_positions.clone();
        move |event: &PositionEvent| actual.borrow_mut().push(event.clone())
    });
    msgbus::subscribe_position_events("events.position.*".into(), position_handler.clone(), None);
    let endpoint = MessagingSwitchboard::exec_engine_reconcile_execution_report();
    let owner = OwnerDelivery::new();
    let binding = owner.bind().unwrap();
    for report in &reports {
        msgbus::send_execution_report(endpoint, report.clone());
        assert_eq!(owner.failure(), None);
        assert_eq!(owner.pending(), 0);
    }
    let first = business_snapshot(&kernel, id);
    assert_eq!(owner.retire(), Ok(()));
    drop(binding);
    msgbus::clear_bus_tap();
    drop(adapter);
    let hwm = Arc::try_unwrap(writer)
        .expect("all tap/writer references released")
        .close(run_ended_draft())
        .expect("actual writer seal and join");
    msgbus::unsubscribe_position_events("events.position.*".into(), &position_handler);
    let actual_positions = actual_positions.borrow().clone();
    kernel.dispose();
    drop(kernel);

    // The writer and its redb backend are closed before this independent file reader opens.
    let (sealed, reader) =
        open_event_store_replay_source(tmp.path().to_path_buf(), INSTANCE_ID, run_id).unwrap();
    assert_eq!(sealed.status, RunStatus::Ended);
    assert_eq!(sealed.high_watermark, hwm);
    let plan = plan_forensics_replay_inputs(&reader, ReplaySeqRange::new(1, hwm)).unwrap();
    let loaded = load_forensics_replay_inputs(&reader, &plan).unwrap();
    assert_eq!(loaded.entries.len(), hwm as usize);
    assert!(
        loaded
            .entries
            .iter()
            .all(|entry| entry.recompute_hash() == entry.entry_hash)
    );
    let envelopes: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.topic.as_ref() == endpoint.as_str())
        .collect();
    assert_eq!(envelopes.len(), 5);
    let decoded: Vec<_> = envelopes
        .iter()
        .map(|entry| match entry.payload_type.as_str() {
            "OrderStatusReport" => ExecutionReport::Order(Box::new(
                rmp_serde::from_slice::<OrderStatusReport>(&entry.payload).unwrap(),
            )),
            "OrderWithFills" => {
                let bundled: BundledReport = rmp_serde::from_slice(&entry.payload).unwrap();
                ExecutionReport::OrderWithFills(
                    Box::new(bundled.order_report),
                    bundled.fill_reports,
                )
            }
            "FillReport" => ExecutionReport::Fill(Box::new(
                rmp_serde::from_slice::<FillReport>(&entry.payload).unwrap(),
            )),
            "PositionStatusReport" => ExecutionReport::Position(Box::new(
                rmp_serde::from_slice::<PositionStatusReport>(&entry.payload).unwrap(),
            )),
            "ExecutionMassStatus" => ExecutionReport::MassStatus(Box::new(
                rmp_serde::from_slice::<ExecutionMassStatus>(&entry.payload).unwrap(),
            )),
            other => panic!("unexpected captured report type {other}"),
        })
        .collect();
    for (actual, expected) in decoded.iter().zip(&reports) {
        assert_eq!(report_wire(actual), report_wire(expected));
    }
    for (topic, expected) in [
        ("reconciliation.raw.OrderStatusReport", 3),
        ("reconciliation.raw.FillReport", 5),
        ("reconciliation.raw.PositionStatusReport", 2),
    ] {
        assert_eq!(
            loaded
                .entries
                .iter()
                .filter(|entry| entry.topic.as_ref() == topic)
                .count(),
            expected
        );
    }
    for (index, envelope) in envelopes.iter().enumerate() {
        let next = envelopes.get(index + 1).map_or(hwm + 1, |entry| entry.seq);
        assert!(loaded.entries.iter().any(|entry| entry.seq > envelope.seq
            && entry.seq < next
            && entry.topic.as_ref().starts_with("reconciliation.raw.")));
    }
    let captured_fills: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.payload_type.as_str() == "OrderFilled")
        .map(|entry| rmp_serde::from_slice::<OrderFilled>(&entry.payload).unwrap())
        .collect();
    assert_eq!(captured_fills, first.3);
    assert_eq!(
        loaded
            .entries
            .iter()
            .filter(|entry| entry.payload_type.as_str() == "OrderAccepted")
            .count(),
        1
    );
    let accepted_entry = loaded
        .entries
        .iter()
        .find(|entry| entry.payload_type.as_str() == "OrderAccepted")
        .unwrap();
    let captured_accepted: OrderAccepted = rmp_serde::from_slice(&accepted_entry.payload).unwrap();
    assert!(
        first.0.contains(
            &rmp_serde::to_vec_named(&OrderEventAny::Accepted(captured_accepted)).unwrap()
        )
    );
    let opened: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.payload_type.as_str() == "PositionOpened")
        .map(|entry| rmp_serde::from_slice::<PositionOpened>(&entry.payload).unwrap())
        .collect();
    let changed: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.payload_type.as_str() == "PositionChanged")
        .map(|entry| rmp_serde::from_slice::<PositionChanged>(&entry.payload).unwrap())
        .collect();
    assert_eq!(actual_positions.len(), 2);
    assert_eq!(
        rmp_serde::to_vec_named(&PositionEvent::PositionOpened(opened[0].clone())).unwrap(),
        rmp_serde::to_vec_named(&actual_positions[0]).unwrap()
    );
    assert_eq!(
        rmp_serde::to_vec_named(&PositionEvent::PositionChanged(changed[0].clone())).unwrap(),
        rmp_serde::to_vec_named(&actual_positions[1]).unwrap()
    );
    assert_eq!(opened.len(), 1);
    assert_eq!(changed.len(), 1);
    assert_eq!(opened[0].quantity, Quantity::from(50_000));
    assert_eq!(changed[0].quantity, Quantity::from(100_000));

    let mut replay_kernel = fresh_kernel(&order);
    let replay_owner = OwnerDelivery::new();
    let replay_binding = replay_owner.bind().unwrap();
    for report in &decoded {
        msgbus::send_execution_report(endpoint, report.clone());
        assert_eq!(replay_owner.failure(), None);
        assert_eq!(replay_owner.pending(), 0);
    }
    let replayed = business_snapshot(&replay_kernel, id);
    for (actual, original) in replayed.3.iter().zip(&first.3) {
        assert_ne!(
            actual.event_id, original.event_id,
            "fresh canonical identities are not wire replays"
        );
        assert_eq!(actual.trader_id, original.trader_id);
        assert_eq!(actual.strategy_id, original.strategy_id);
        assert_eq!(actual.account_id, original.account_id);
        assert_eq!(actual.instrument_id, original.instrument_id);
        assert_eq!(actual.client_order_id, original.client_order_id);
        assert_eq!(actual.venue_order_id, original.venue_order_id);
        assert_eq!(actual.position_id, original.position_id);
        assert_eq!(actual.trade_id, original.trade_id);
        assert_eq!(actual.order_side, original.order_side);
        assert_eq!(actual.last_qty, original.last_qty);
        assert_eq!(actual.last_px, original.last_px);
        assert_eq!(actual.currency, original.currency);
        assert_eq!(actual.commission, original.commission);
        assert_eq!(actual.liquidity_side, original.liquidity_side);
        assert_eq!(actual.ts_event, original.ts_event);
        assert_eq!(actual.reconciliation, original.reconciliation);
    }
    for report in &decoded {
        msgbus::send_execution_report(endpoint, report.clone());
    }
    let duplicate = business_snapshot(&replay_kernel, id);
    assert_eq!(duplicate.0, replayed.0);
    assert_eq!(duplicate.1, replayed.1);
    assert_eq!(duplicate.2, replayed.2);
    assert_eq!(replay_owner.failure(), None);
    assert_eq!(replay_owner.pending(), 0);
    assert_eq!(replay_owner.retire(), Ok(()));
    drop(replay_binding);
    replay_kernel.dispose();
}

#[rstest]
fn cash_fill_void_wire_reopens_redb_and_replays_actual_apply_into_fresh_kernel() {
    use std::{rc::Rc, sync::Arc};

    use nautilus_common::{
        enums::Environment,
        messages::ExecutionReport,
        msgbus::{self, BusTap, MessagingSwitchboard},
        owner_delivery::OwnerDelivery,
    };
    use nautilus_core::UUID4;
    use nautilus_event_store::{BusCaptureAdapter, default_registry};
    use nautilus_model::{
        accounts::{Account, AccountAny, CashAccount},
        enums::{
            AccountType, LiquiditySide, OmsType, OrderSide, OrderStatus, OrderType, TimeInForce,
        },
        events::{AccountState, OrderEventAny, OrderFillVoided, OrderFilled},
        identifiers::{AccountId, ClientOrderId, TradeId, TraderId, VenueOrderId},
        instruments::{Instrument, stubs::audusd_sim},
        orders::{Order, OrderAny, builder::OrderTestBuilder, stubs::TestOrderEventStubs},
        reports::{FillReport, OrderStatusReport},
        types::{AccountBalance, Currency, Money, Price, Quantity},
    };
    use nautilus_system::kernel::NautilusKernel;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct BundledReport {
        order_report: OrderStatusReport,
        fill_reports: Vec<FillReport>,
    }

    fn wire(report: &ExecutionReport) -> Vec<u8> {
        match report {
            ExecutionReport::Order(report) => rmp_serde::to_vec_named(&**report).unwrap(),
            ExecutionReport::Fill(report) => rmp_serde::to_vec_named(&**report).unwrap(),
            ExecutionReport::OrderWithFills(report, fills) => {
                rmp_serde::to_vec_named(&BundledReport {
                    order_report: (**report).clone(),
                    fill_reports: fills.clone(),
                })
                .unwrap()
            }
            _ => unreachable!("this complete Cash history uses order, fill, and snapshot reports"),
        }
    }

    fn fresh_kernel(order: &OrderAny) -> NautilusKernel {
        let kernel = NautilusKernel::builder(
            "cash-void-durable-replay".into(),
            TraderId::from("REPLAY-001"),
            Environment::Backtest,
        )
        .build()
        .expect("fresh real Kernel and Portfolio with no previous apply basis");
        let account_id = AccountId::from("SIM-001");
        let mut cache = kernel.cache.borrow_mut();
        cache.add_instrument(audusd_sim().into()).unwrap();
        let state = AccountState::new(
            account_id,
            AccountType::Cash,
            vec![AccountBalance::new(
                Money::from("1000000 USD"),
                Money::from("0 USD"),
                Money::from("1000000 USD"),
            )],
            vec![],
            true,
            UUID4::new(),
            0.into(),
            0.into(),
            Some(Currency::USD()),
        );
        cache
            .add_account(CashAccount::new(state, true, false).into())
            .unwrap();
        cache.add_order(order.clone(), None, None, false).unwrap();
        drop(cache);
        let mut engine = kernel.exec_engine.borrow_mut();
        engine.register_oms_type(order.strategy_id(), OmsType::Netting);
        engine.process(&TestOrderEventStubs::submitted(order, account_id));
        drop(engine);
        kernel
    }

    fn assert_account(kernel: &NautilusKernel, total: &str, locked: &str, fee: &str) {
        let AccountAny::Cash(account) = kernel
            .cache
            .borrow()
            .account_owned(&AccountId::from("SIM-001"))
            .unwrap()
        else {
            panic!("derived single-currency Cash account");
        };
        let total = Money::from(total);
        let locked = Money::from(locked);
        assert_eq!(account.balance_total(None), Some(total));
        assert_eq!(account.balance_locked(None), Some(locked));
        assert_eq!(account.balance_free(None), Some(total - locked));
        assert_eq!(
            account
                .commission(&Currency::USD())
                .unwrap_or_else(|| Money::from("0 USD")),
            Money::from(fee)
        );
    }

    fn final_snapshot(kernel: &NautilusKernel, id: ClientOrderId) -> Vec<Vec<u8>> {
        assert_account(kernel, "949999.95 USD", "150000 USD", "0.05 USD");
        let cache = kernel.cache.borrow();
        let order = cache.order(&id).unwrap();
        assert_eq!(order.status(), OrderStatus::PartiallyFilled);
        assert_eq!(order.filled_qty(), Quantity::from(50_000));
        assert_eq!(order.voided_qty(), Quantity::from(175_000));
        assert_eq!(order.trade_ids(), vec![&TradeId::from("CASH-REPLAY-A")]);
        assert_eq!(
            order.commissions().get(&Currency::USD()),
            Some(&Money::from("0.05 USD"))
        );
        let fills: Vec<_> = order
            .events()
            .into_iter()
            .filter_map(|event| match event {
                OrderEventAny::Filled(fill) => Some(fill.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            fills.iter().map(|fill| fill.trade_id).collect::<Vec<_>>(),
            vec![
                TradeId::from("CASH-REPLAY-A"),
                TradeId::from("CASH-REPLAY-B"),
                TradeId::from("CASH-REPLAY-CHILD"),
            ],
        );
        let voids: Vec<_> = order
            .events()
            .into_iter()
            .filter_map(|event| match event {
                OrderEventAny::FillVoided(voided) => Some(voided.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            voids
                .iter()
                .map(|event| (event.trade_id, event.voided_qty, event.commission_voided))
                .collect::<Vec<_>>(),
            vec![
                (
                    TradeId::from("CASH-REPLAY-B"),
                    Quantity::from(100_000),
                    Some(Money::from("0.10 USD"))
                ),
                (
                    TradeId::from("CASH-REPLAY-CHILD"),
                    Quantity::from(25_000),
                    Some(Money::from("0.10 USD"))
                ),
                (
                    TradeId::from("CASH-REPLAY-A"),
                    Quantity::from(50_000),
                    Some(Money::from("0.05 USD"))
                ),
            ]
        );
        let positions = cache.positions(
            None,
            Some(&audusd_sim().id()),
            None,
            Some(&AccountId::from("SIM-001")),
            None,
        );
        assert_eq!(positions.len(), 1);
        let position = &positions[0];
        assert_eq!(position.quantity, Quantity::from(50_000));
        assert_eq!(position.commissions(), vec![Money::from("0.05 USD")]);
        let mut effective = fills[0].clone();
        effective.last_qty = Quantity::from(50_000);
        effective.commission = Some(Money::from("0.05 USD"));
        assert_eq!(position.events, vec![effective]);
        let mut snapshot: Vec<_> = order
            .events()
            .iter()
            .map(|event| rmp_serde::to_vec_named(event).unwrap())
            .collect();
        snapshot.push(rmp_serde::to_vec_named(&**position).unwrap());
        snapshot.push(
            rmp_serde::to_vec_named(&cache.account_owned(&AccountId::from("SIM-001")).unwrap())
                .unwrap(),
        );
        snapshot
    }

    let instrument = audusd_sim();
    let account_id = AccountId::from("SIM-001");
    let id = ClientOrderId::from("CASH-REPLAY-ORDER");
    let venue_id = VenueOrderId::from("V-CASH-REPLAY");
    let order = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(TraderId::from("REPLAY-001"))
        .instrument_id(instrument.id())
        .client_order_id(id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from(200_000))
        .price(Price::from("1.00000"))
        .build();
    let accepted = OrderStatusReport::new(
        account_id,
        instrument.id(),
        Some(id),
        venue_id,
        Some(OrderSide::Buy),
        OrderType::Limit,
        TimeInForce::Gtc,
        OrderStatus::Accepted,
        Quantity::from(200_000),
        Quantity::zero(0),
        10.into(),
        10.into(),
        10.into(),
        None,
    )
    .with_price(Price::from("1.00000"));
    let fills: Vec<_> = [
        ("CASH-REPLAY-A", 100_000),
        ("CASH-REPLAY-B", 100_000),
        ("CASH-REPLAY-CHILD", 25_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (trade, quantity))| {
        let timestamp = [11_u64, 12, 15][index];
        FillReport::new(
            account_id,
            instrument.id(),
            venue_id,
            TradeId::from(trade),
            OrderSide::Buy,
            Quantity::from(quantity),
            Price::from("1.00000"),
            Money::from("0.10 USD"),
            LiquiditySide::Maker,
            Some(id),
            None,
            timestamp.into(),
            timestamp.into(),
            None,
        )
    })
    .collect();
    let mut partial = accepted.clone();
    partial.report_id = UUID4::new();
    partial.order_status = OrderStatus::PartiallyFilled;
    partial.filled_qty = Quantity::from(100_000);
    partial.ts_last = 14.into();
    let mut corrected = partial.clone();
    corrected.report_id = UUID4::new();
    corrected.filled_qty = Quantity::from(50_000);
    corrected.ts_last = 16.into();
    // These are the complete real reports, including the first void before the intervening fill.
    // Replaying only the final account snapshot could not rebuild actual per-fill apply evidence.
    let reports = vec![
        ExecutionReport::Order(Box::new(accepted)),
        ExecutionReport::Fill(Box::new(fills[0].clone())),
        ExecutionReport::Fill(Box::new(fills[1].clone())),
        ExecutionReport::OrderWithFills(Box::new(partial), vec![]),
        ExecutionReport::Fill(Box::new(fills[2].clone())),
        ExecutionReport::OrderWithFills(Box::new(corrected), vec![]),
    ];
    // Principal and fee oracles are independent of the correction implementation.
    let expected = [
        ("1000000 USD", "200000 USD", "0 USD"),
        ("899999.90 USD", "100000 USD", "0.10 USD"),
        ("799999.80 USD", "0 USD", "0.20 USD"),
        ("899999.90 USD", "100000 USD", "0.10 USD"),
        ("874999.80 USD", "75000 USD", "0.20 USD"),
        ("949999.95 USD", "150000 USD", "0.05 USD"),
    ];
    let tmp = TempDir::new().unwrap();
    let run_id = "cash-void-durable-wire";
    let mut backend = RedbBackend::new(tmp.path());
    backend.open_run(manifest(run_id)).unwrap();
    let writer = Arc::new(
        EventStoreWriter::spawn(
            Box::new(backend),
            get_atomic_clock_static(),
            noop_halt(),
            WriterConfig::default(),
        )
        .unwrap(),
    );
    let registry = Arc::new(default_registry());
    let adapter = Arc::new(BusCaptureAdapter::new(
        writer.clone(),
        registry.clone(),
        noop_halt(),
    ));
    let mut kernel = fresh_kernel(&order);
    let tap: Rc<dyn BusTap> = Rc::new(super::capture::HeadersAwareAdapterTap {
        adapter: adapter.clone(),
        registry,
    });
    msgbus::set_bus_tap(tap);
    let endpoint = MessagingSwitchboard::exec_engine_reconcile_execution_report();
    let owner = OwnerDelivery::new();
    let binding = owner.bind().unwrap();
    for (report, (total, locked, fee)) in reports.iter().zip(expected) {
        msgbus::send_execution_report(endpoint, report.clone());
        assert_eq!(owner.failure(), None);
        assert_eq!(owner.pending(), 0);
        assert_account(&kernel, total, locked, fee);
    }
    let first = final_snapshot(&kernel, id);
    assert_eq!(owner.retire(), Ok(()));
    drop(binding);
    msgbus::clear_bus_tap();
    drop(adapter);
    let hwm = Arc::try_unwrap(writer)
        .expect("all capture references released")
        .close(run_ended_draft())
        .unwrap();
    kernel.dispose();
    drop(kernel);

    let (sealed, reader) =
        open_event_store_replay_source(tmp.path().to_path_buf(), INSTANCE_ID, run_id).unwrap();
    assert_eq!(sealed.status, RunStatus::Ended);
    assert_eq!(sealed.high_watermark, hwm);
    let plan = plan_forensics_replay_inputs(&reader, ReplaySeqRange::new(1, hwm)).unwrap();
    let loaded = load_forensics_replay_inputs(&reader, &plan).unwrap();
    assert_eq!(loaded.entries.len(), hwm as usize);
    assert!(
        loaded
            .entries
            .iter()
            .all(|entry| entry.recompute_hash() == entry.entry_hash)
    );
    let envelopes: Vec<_> = loaded
        .entries
        .iter()
        .filter(|entry| entry.topic.as_ref() == endpoint.as_str())
        .collect();
    assert_eq!(envelopes.len(), reports.len());
    let decoded: Vec<_> = envelopes
        .iter()
        .map(|entry| match entry.payload_type.as_str() {
            "OrderStatusReport" => ExecutionReport::Order(Box::new(
                rmp_serde::from_slice::<OrderStatusReport>(&entry.payload).unwrap(),
            )),
            "FillReport" => ExecutionReport::Fill(Box::new(
                rmp_serde::from_slice::<FillReport>(&entry.payload).unwrap(),
            )),
            "OrderWithFills" => {
                let bundle: BundledReport = rmp_serde::from_slice(&entry.payload).unwrap();
                ExecutionReport::OrderWithFills(Box::new(bundle.order_report), bundle.fill_reports)
            }
            other => panic!("unexpected Cash replay report {other}"),
        })
        .collect();
    for (actual, original) in decoded.iter().zip(&reports) {
        assert_eq!(wire(actual), wire(original));
    }
    let captured: Vec<_> = loaded
        .entries
        .iter()
        .filter_map(|entry| match entry.payload_type.as_str() {
            "OrderFilled" => Some(OrderEventAny::Filled(
                rmp_serde::from_slice::<OrderFilled>(&entry.payload).unwrap(),
            )),
            "OrderFillVoided" => Some(OrderEventAny::FillVoided(
                rmp_serde::from_slice::<OrderFillVoided>(&entry.payload).unwrap(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(captured.len(), 6);
    for event in &captured {
        assert!(first.contains(&rmp_serde::to_vec_named(event).unwrap()));
    }

    let mut replay_kernel = fresh_kernel(&order);
    let replay_owner = OwnerDelivery::new();
    let replay_binding = replay_owner.bind().unwrap();
    for (report, (total, locked, fee)) in decoded.iter().zip(expected) {
        msgbus::send_execution_report(endpoint, report.clone());
        assert_eq!(replay_owner.failure(), None);
        assert_eq!(replay_owner.pending(), 0);
        assert_account(&replay_kernel, total, locked, fee);
    }
    let replayed = final_snapshot(&replay_kernel, id);
    let replayed_events: Vec<_> = replay_kernel
        .cache
        .borrow()
        .order(&id)
        .unwrap()
        .events()
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                OrderEventAny::Filled(_) | OrderEventAny::FillVoided(_)
            )
        })
        .cloned()
        .collect();
    assert_eq!(replayed_events.len(), captured.len());
    for (actual, original) in replayed_events.iter().zip(&captured) {
        match (actual, original) {
            (OrderEventAny::Filled(actual), OrderEventAny::Filled(original)) => {
                assert_ne!(actual.event_id, original.event_id);
                assert_eq!(actual.last_qty.precision, original.last_qty.precision);
                assert_eq!(actual.last_px.precision, original.last_px.precision);
                let mut normalized = actual.clone();
                normalized.event_id = original.event_id;
                assert_eq!(normalized, *original);
            }
            (OrderEventAny::FillVoided(actual), OrderEventAny::FillVoided(original)) => {
                assert_ne!(actual.event_id, original.event_id);
                assert_eq!(actual.voided_qty.precision, original.voided_qty.precision);
                assert_eq!(actual.last_px.precision, original.last_px.precision);
                let mut normalized = actual.clone();
                normalized.event_id = original.event_id;
                assert_eq!(normalized, *original);
            }
            _ => panic!("fresh Kernel must preserve the actual fill/void history order"),
        }
    }
    // Redelivering actual original fill and final correction reports cannot book or refund again.
    for report in [
        &decoded[1],
        &decoded[2],
        &decoded[4],
        &decoded[5],
        &decoded[5],
    ] {
        msgbus::send_execution_report(endpoint, report.clone());
        assert_eq!(replay_owner.failure(), None);
        assert_eq!(replay_owner.pending(), 0);
    }
    assert_eq!(final_snapshot(&replay_kernel, id), replayed);
    assert_eq!(replay_owner.retire(), Ok(()));
    drop(replay_binding);
    replay_kernel.dispose();
}
