//! Python bindings for the SoDEX client factories.

use pyo3::pymethods;

use crate::{
    config::SODEX,
    factories::{SodexDataClientFactory, SodexExecutionClientFactory},
};

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl SodexDataClientFactory {
    /// Factory for creating SoDEX data clients.
    #[new]
    const fn py_new() -> Self {
        Self::new()
    }

    /// Registry key this factory answers to.
    ///
    /// One key covers both engines: the configuration's `market` decides which venue the
    /// client it builds belongs to, so a node registers this once and creates a spot and a
    /// perps client from it.
    #[pyo3(name = "name")]
    const fn py_name(&self) -> &str {
        SODEX
    }
}

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl SodexExecutionClientFactory {
    /// Factory for creating SoDEX execution clients.
    #[new]
    const fn py_new() -> Self {
        Self::new()
    }

    /// Registry key this factory answers to.
    #[pyo3(name = "name")]
    const fn py_name(&self) -> &str {
        SODEX
    }
}
