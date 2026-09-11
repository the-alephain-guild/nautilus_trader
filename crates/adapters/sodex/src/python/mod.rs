//! Python bindings from `pyo3`.
//!
//! Exposed as `nautilus_trader.adapters.sodex`. Without this layer the adapter is reachable
//! only from Rust, which would leave a Python strategy unable to trade this venue at all.
//!
//! Two things are registered: the classes a Python config names directly, and the extractors
//! the node uses to turn those Python objects back into the Rust trait objects it runs. The
//! second half is easy to forget - the classes import fine without it, and the failure only
//! appears when the node tries to build a client and cannot recognize the factory it was
//! handed.

pub mod config;
pub mod enums;
pub mod factories;

use nautilus_common::factories::{ClientConfig, DataClientFactory, ExecutionClientFactory};
use nautilus_core::python::{to_pyruntime_err, to_pyvalue_err};
use nautilus_model::identifiers::Venue;
use nautilus_system::get_global_pyo3_registry;
use pyo3::prelude::*;

use crate::{
    common::Market,
    config::{SODEX, SODEX_PERPS, SODEX_SPOT, SodexDataClientConfig, SodexExecClientConfig},
    factories::{SodexDataClientFactory, SodexExecutionClientFactory},
    http::Network,
};

#[expect(clippy::needless_pass_by_value)]
fn extract_sodex_data_factory(
    py: Python<'_>,
    factory: Py<PyAny>,
) -> PyResult<Box<dyn DataClientFactory>> {
    match factory.extract::<SodexDataClientFactory>(py) {
        Ok(f) => Ok(Box::new(f)),
        Err(e) => Err(to_pyvalue_err(format!(
            "Failed to extract SodexDataClientFactory: {e}"
        ))),
    }
}

#[expect(clippy::needless_pass_by_value)]
fn extract_sodex_exec_factory(
    py: Python<'_>,
    factory: Py<PyAny>,
) -> PyResult<Box<dyn ExecutionClientFactory>> {
    match factory.extract::<SodexExecutionClientFactory>(py) {
        Ok(f) => Ok(Box::new(f)),
        Err(e) => Err(to_pyvalue_err(format!(
            "Failed to extract SodexExecutionClientFactory: {e}"
        ))),
    }
}

#[expect(clippy::needless_pass_by_value)]
fn extract_sodex_data_config(py: Python<'_>, config: Py<PyAny>) -> PyResult<Box<dyn ClientConfig>> {
    match config.extract::<SodexDataClientConfig>(py) {
        Ok(c) => Ok(Box::new(c)),
        Err(e) => Err(to_pyvalue_err(format!(
            "Failed to extract SodexDataClientConfig: {e}"
        ))),
    }
}

#[expect(clippy::needless_pass_by_value)]
fn extract_sodex_exec_config(py: Python<'_>, config: Py<PyAny>) -> PyResult<Box<dyn ClientConfig>> {
    match config.extract::<SodexExecClientConfig>(py) {
        Ok(c) => Ok(Box::new(c)),
        Err(e) => Err(to_pyvalue_err(format!(
            "Failed to extract SodexExecClientConfig: {e}"
        ))),
    }
}

/// Exposed through `nautilus_trader.adapters.sodex`.
///
/// # Errors
///
/// Returns an error if any bindings fail to register with the Python module.
#[pymodule]
pub fn sodex(_: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(stringify!(SODEX), SODEX)?;
    // Two venues, because spot and perps differ in ways no parameter papers over. A Python
    // config picks one per client and the strategy's instrument ids must carry the matching
    // venue, so both names are exported rather than left to be spelled by hand.
    m.add(stringify!(SODEX_SPOT), SODEX_SPOT)?;
    m.add(stringify!(SODEX_PERPS), SODEX_PERPS)?;
    m.add("SODEX_SPOT_VENUE", Venue::from(SODEX_SPOT))?;
    m.add("SODEX_PERPS_VENUE", Venue::from(SODEX_PERPS))?;

    m.add_class::<Market>()?;
    m.add_class::<Network>()?;
    m.add_class::<SodexDataClientConfig>()?;
    m.add_class::<SodexDataClientFactory>()?;
    m.add_class::<SodexExecClientConfig>()?;
    m.add_class::<SodexExecutionClientFactory>()?;

    let registry = get_global_pyo3_registry();

    if let Err(e) =
        registry.register_factory_extractor(SODEX.to_string(), extract_sodex_data_factory)
    {
        return Err(to_pyruntime_err(format!(
            "Failed to register SoDEX data factory extractor: {e}"
        )));
    }

    if let Err(e) =
        registry.register_exec_factory_extractor(SODEX.to_string(), extract_sodex_exec_factory)
    {
        return Err(to_pyruntime_err(format!(
            "Failed to register SoDEX exec factory extractor: {e}"
        )));
    }

    if let Err(e) = registry.register_config_extractor(
        "SodexDataClientConfig".to_string(),
        extract_sodex_data_config,
    ) {
        return Err(to_pyruntime_err(format!(
            "Failed to register SoDEX data config extractor: {e}"
        )));
    }

    if let Err(e) = registry.register_config_extractor(
        "SodexExecClientConfig".to_string(),
        extract_sodex_exec_config,
    ) {
        return Err(to_pyruntime_err(format!(
            "Failed to register SoDEX exec config extractor: {e}"
        )));
    }

    Ok(())
}
