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
#![cfg(feature = "python")]

//! Proves the Python bindings are reachable, not merely that they compile.
//!
//! Compiling with the `python` feature says the `#[pyclass]` attributes are well formed. It
//! says nothing about whether the node can find the factory it was handed: that path goes
//! through a global registry keyed by name, and a key that is registered under one spelling
//! and looked up under another fails only at run time, when a strategy tries to start.
//!
//! So this registers the module the way the extension does and then pulls the factories and
//! configs back out through the registry, building a real client from what comes out.

use std::{cell::RefCell, rc::Rc};

use nautilus_common::{
    cache::Cache,
    clock::TestClock,
    live::runner::{replace_data_event_sender, replace_exec_event_sender},
    messages::{DataEvent, ExecutionEvent},
};
use nautilus_model::identifiers::{ClientId, TraderId};
use nautilus_sodex::{
    common::Market,
    config::{SODEX, SODEX_PERPS, SODEX_SPOT, SodexDataClientConfig, SodexExecClientConfig},
    factories::{SodexDataClientFactory, SodexExecutionClientFactory},
    http::Network,
    python,
};
use nautilus_system::get_global_pyo3_registry;
use pyo3::{Py, Python, types::PyModule};
use rstest::rstest;

/// A throwaway key, never registered at the venue. Enough to construct a signer.
const TEST_API_KEY: &str = "0x2ae8be44db8a590d20bffbe3b6872df9b569147d3bf6801a35a28281a4816bbd";
const TEST_ACCOUNT_ID: u64 = 60366;

fn register_sodex_python_module(py: Python<'_>) {
    let module = PyModule::new(py, "sodex").expect("SoDEX module should be created");
    python::sodex(py, &module).expect("SoDEX Python module should register");
}

fn setup_event_senders() {
    let (data_tx, _data_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
    replace_data_event_sender(data_tx);
    let (exec_tx, _exec_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
    replace_exec_event_sender(exec_tx);
}

#[rstest]
fn test_sodex_python_factories_extract_from_registry() {
    setup_event_senders();
    Python::initialize();

    Python::attach(|py| {
        register_sodex_python_module(py);
        assert_data_factory_extracts_from_python_object(py);
        assert_exec_factory_extracts_from_python_object(py);
        assert_both_engines_reachable_from_one_factory(py);
    });
}

fn assert_data_factory_extracts_from_python_object(py: Python<'_>) {
    let factory = Py::new(py, SodexDataClientFactory::new())
        .expect("factory should convert to Python object")
        .into_any();
    let config = Py::new(
        py,
        SodexDataClientConfig {
            network: Network::Testnet,
            market: Market::Spot,
            instrument_ids: Vec::new(),
            update_instruments_interval_mins: Some(30),
            timeout_secs: 7,
        },
    )
    .expect("config should convert to Python object")
    .into_any();
    let registry = get_global_pyo3_registry();

    let extracted_factory = registry
        .extract_factory(py, factory)
        .expect("data factory should extract");
    let extracted_config = registry
        .extract_config(py, config)
        .expect("data config should extract");
    let sodex_config = extracted_config
        .as_any()
        .downcast_ref::<SodexDataClientConfig>()
        .expect("data config should downcast");
    let cache = Rc::new(RefCell::new(Cache::default()));
    let clock = Rc::new(RefCell::new(TestClock::new()));
    let client = extracted_factory
        .create(
            "SODEX-DATA-EXTRACTED",
            extracted_config.as_ref(),
            cache.into(),
            clock,
        )
        .expect("extracted factory should create data client");

    assert_eq!(extracted_factory.name(), SODEX);
    assert_eq!(extracted_factory.config_type(), "SodexDataClientConfig");
    assert_eq!(sodex_config.network, Network::Testnet);
    assert_eq!(sodex_config.market, Market::Spot);
    // The timeout used to be dropped on the way through: the HTTP builder hardcoded 30 and
    // this field was never read, so a config asking for 7 silently got 30.
    assert_eq!(sodex_config.timeout_secs, 7);
    assert_eq!(sodex_config.update_instruments_interval_mins, Some(30));
    assert_eq!(client.client_id(), ClientId::from("SODEX-DATA-EXTRACTED"));
    assert_eq!(client.venue().map(|v| v.to_string()), Some(SODEX_SPOT.to_string()));
}

fn assert_exec_factory_extracts_from_python_object(py: Python<'_>) {
    let factory = Py::new(py, SodexExecutionClientFactory::new())
        .expect("factory should convert to Python object")
        .into_any();
    let config = Py::new(
        py,
        SodexExecClientConfig {
            network: Network::Testnet,
            market: Market::Perps,
            account_id: Some(TEST_ACCOUNT_ID),
            api_key_name: Some("api-key-01".to_string()),
            api_private_key: Some(TEST_API_KEY.into()),
            update_instruments_interval_mins: None,
            timeout_secs: 11,
        },
    )
    .expect("config should convert to Python object")
    .into_any();
    let registry = get_global_pyo3_registry();

    let extracted_factory = registry
        .extract_exec_factory(py, factory)
        .expect("exec factory should extract");
    let extracted_config = registry
        .extract_config(py, config)
        .expect("exec config should extract");
    let sodex_config = extracted_config
        .as_any()
        .downcast_ref::<SodexExecClientConfig>()
        .expect("exec config should downcast");
    let cache = Rc::new(RefCell::new(Cache::default()));
    let client = extracted_factory
        .create(
            TraderId::from("TRADER-001"),
            "SODEX-EXEC-EXTRACTED",
            extracted_config.as_ref(),
            cache.into(),
        )
        .expect("extracted factory should create execution client");

    assert_eq!(extracted_factory.name(), SODEX);
    assert_eq!(extracted_factory.config_type(), "SodexExecClientConfig");
    assert_eq!(sodex_config.market, Market::Perps);
    assert_eq!(sodex_config.timeout_secs, 11);
    assert_eq!(client.client_id(), ClientId::from("SODEX-EXEC-EXTRACTED"));
    assert_eq!(client.venue().to_string(), SODEX_PERPS);
    // The account id carries the venue's own numeric id, so an account event can be traced
    // back to the account that produced it without re-reading the configuration.
    assert_eq!(
        client.account_id().to_string(),
        format!("{SODEX_PERPS}-{TEST_ACCOUNT_ID}")
    );
}

fn assert_both_engines_reachable_from_one_factory(py: Python<'_>) {
    // One registry key serves both engines, which is the whole reason the configuration
    // carries `market`. If the venue were decided by the factory instead, a node would need
    // two registrations and this would be two keys.
    let registry = get_global_pyo3_registry();
    let cache = Rc::new(RefCell::new(Cache::default()));
    let clock: Rc<RefCell<dyn nautilus_common::clock::Clock>> =
        Rc::new(RefCell::new(TestClock::new()));

    let mut venues = Vec::new();
    for (market, name) in [(Market::Spot, "SODEX-SPOT"), (Market::Perps, "SODEX-PERPS")] {
        let factory = Py::new(py, SodexDataClientFactory::new())
            .expect("factory should convert")
            .into_any();
        let config = Py::new(
            py,
            SodexDataClientConfig {
                network: Network::Testnet,
                market,
                instrument_ids: Vec::new(),
                update_instruments_interval_mins: None,
                timeout_secs: 30,
            },
        )
        .expect("config should convert")
        .into_any();

        let client = registry
            .extract_factory(py, factory)
            .expect("factory should extract")
            .create(
                name,
                registry
                    .extract_config(py, config)
                    .expect("config should extract")
                    .as_ref(),
                Rc::clone(&cache).into(),
                Rc::clone(&clock),
            )
            .expect("factory should create client");

        venues.push(client.venue().map(|v| v.to_string()));
    }

    assert_eq!(
        venues,
        vec![Some(SODEX_SPOT.to_string()), Some(SODEX_PERPS.to_string())]
    );
}
