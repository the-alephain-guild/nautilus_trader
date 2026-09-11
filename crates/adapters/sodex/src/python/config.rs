//! Python bindings for SoDEX configuration.

use nautilus_core::string::secret::SecretString;
use nautilus_model::identifiers::InstrumentId;
use pyo3::pymethods;

use crate::{
    common::Market,
    config::{
        SodexDataClientConfig, SodexExecClientConfig, default_instrument_refresh_mins,
        default_timeout_secs,
    },
    http::Network,
};

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl SodexDataClientConfig {
    /// Configuration for the SoDEX live data client.
    ///
    /// Takes no credentials, because the venue serves market data unsigned — a data-only
    /// deployment holds no secret at all.
    #[new]
    #[pyo3(signature = (
        network = None,
        market = None,
        instrument_ids = None,
        update_instruments_interval_mins = None,
        timeout_secs = None,
    ))]
    fn py_new(
        network: Option<Network>,
        market: Option<Market>,
        instrument_ids: Option<Vec<InstrumentId>>,
        update_instruments_interval_mins: Option<u64>,
        timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            network: network.unwrap_or_default(),
            market: market.unwrap_or_default(),
            instrument_ids: instrument_ids.unwrap_or_default(),
            // An explicit `0` disables the reload; omitting the argument keeps the default
            // rather than disabling it, because silence should not turn a protection off.
            update_instruments_interval_mins: update_instruments_interval_mins
                .map(Some)
                .unwrap_or_else(default_instrument_refresh_mins),
            timeout_secs: timeout_secs.unwrap_or_else(default_timeout_secs),
        }
    }

    #[getter]
    #[pyo3(name = "network")]
    const fn py_network(&self) -> Network {
        self.network
    }

    #[getter]
    #[pyo3(name = "market")]
    const fn py_market(&self) -> Market {
        self.market
    }

    #[getter]
    #[pyo3(name = "timeout_secs")]
    const fn py_timeout_secs(&self) -> u64 {
        self.timeout_secs
    }

    /// The venue these instruments belong to, which is engine-specific.
    #[getter]
    #[pyo3(name = "venue")]
    fn py_venue(&self) -> String {
        self.venue().to_string()
    }
}

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl SodexExecClientConfig {
    /// Configuration for the SoDEX live execution client.
    ///
    /// Every credential may be left unset, in which case it resolves from the environment —
    /// `SODEX_ACCOUNT_ID`, `SODEX_API_KEY_NAME`, `SODEX_API_PRIVATE_KEY`. Preferring that to
    /// passing `api_private_key` here keeps the key out of config files and out of any Python
    /// traceback that prints its arguments.
    ///
    /// The key this takes is a registered **API key**, never the master wallet: the master key
    /// can authorize withdrawals and belongs offline. `wallet_address` is the master wallet's
    /// *address* — public information that signs nothing, and required because the account reads
    /// are addressed by it. A wrong address there answers with an empty account rather than an
    /// error, so the client verifies it against the registered key list at startup.
    #[new]
    #[pyo3(signature = (
        network = None,
        market = None,
        account_id = None,
        api_key_name = None,
        api_private_key = None,
        wallet_address = None,
        update_instruments_interval_mins = None,
        timeout_secs = None,
    ))]
    #[expect(clippy::too_many_arguments)]
    fn py_new(
        network: Option<Network>,
        market: Option<Market>,
        account_id: Option<u64>,
        api_key_name: Option<String>,
        api_private_key: Option<String>,
        wallet_address: Option<String>,
        update_instruments_interval_mins: Option<u64>,
        timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            network: network.unwrap_or_default(),
            market: market.unwrap_or_default(),
            account_id,
            api_key_name,
            api_private_key: api_private_key.map(SecretString::from),
            wallet_address,
            update_instruments_interval_mins: update_instruments_interval_mins
                .map(Some)
                .unwrap_or_else(default_instrument_refresh_mins),
            timeout_secs: timeout_secs.unwrap_or_else(default_timeout_secs),
        }
    }

    #[getter]
    #[pyo3(name = "network")]
    const fn py_network(&self) -> Network {
        self.network
    }

    #[getter]
    #[pyo3(name = "market")]
    const fn py_market(&self) -> Market {
        self.market
    }

    #[getter]
    #[pyo3(name = "timeout_secs")]
    const fn py_timeout_secs(&self) -> u64 {
        self.timeout_secs
    }

    /// The venue this client trades on.
    #[getter]
    #[pyo3(name = "venue")]
    fn py_venue(&self) -> String {
        self.venue().to_string()
    }

    /// Whether every credential resolves, from the config or the environment.
    ///
    /// Worth calling at startup: the alternative is discovering a missing key when the first
    /// order is rejected.
    #[pyo3(name = "has_credentials")]
    fn py_has_credentials(&self) -> bool {
        self.has_credentials()
    }
}
