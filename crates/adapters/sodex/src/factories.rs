//! Factories that build this adapter's clients from configuration.
//!
//! One factory per client kind, not one per engine. Spot and perps are separate venues, but
//! they are selected by a field on the configuration rather than by which factory was
//! registered, so a node registers each factory once and creates both clients from it.

use std::{any::Any, cell::RefCell, rc::Rc};

use nautilus_common::{
    cache::CacheView,
    clients::{DataClient, ExecutionClient},
    clock::Clock,
    factories::{ClientConfig, DataClientFactory, ExecutionClientFactory},
};
use nautilus_core::time::get_atomic_clock_realtime;
use nautilus_live::ExecutionClientCore;
use nautilus_model::{
    enums::{AccountType, OmsType},
    identifiers::{AccountId, ClientId, TraderId},
};

use crate::{
    common::Market,
    config::{SODEX, SodexDataClientConfig, SodexExecClientConfig, venue_for},
    data::SodexDataClient,
    execution::SodexExecutionClient,
};

impl ClientConfig for SodexDataClientConfig {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ClientConfig for SodexExecClientConfig {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// How the engine settles at each SoDEX venue.
///
/// Spot holds balances and has no positions to net; perps documents one-way mode as the only
/// position side accepted when placing orders, which is netting rather than hedging.
const fn account_type_for(market: Market) -> AccountType {
    match market {
        Market::Spot => AccountType::Cash,
        Market::Perps => AccountType::Margin,
    }
}

/// Builds SoDEX market data clients.
#[derive(Debug, Clone, Default)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.sodex", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.sodex")
)]
pub struct SodexDataClientFactory;

impl SodexDataClientFactory {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DataClientFactory for SodexDataClientFactory {
    fn create(
        &self,
        name: &str,
        config: &dyn ClientConfig,
        _cache: CacheView,
        _clock: Rc<RefCell<dyn Clock>>,
    ) -> anyhow::Result<Box<dyn DataClient>> {
        let config = config
            .as_any()
            .downcast_ref::<SodexDataClientConfig>()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "expected a SodexDataClientConfig for {}, was {config:?}",
                    self.name()
                )
            })?
            .clone();

        Ok(Box::new(SodexDataClient::new(
            ClientId::from(name),
            config,
        )?))
    }

    fn name(&self) -> &'static str {
        SODEX
    }

    fn config_type(&self) -> &'static str {
        "SodexDataClientConfig"
    }
}

/// Builds SoDEX execution clients.
#[derive(Debug, Clone, Default)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.sodex", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.sodex")
)]
pub struct SodexExecutionClientFactory;

impl SodexExecutionClientFactory {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ExecutionClientFactory for SodexExecutionClientFactory {
    fn create(
        &self,
        trader_id: TraderId,
        name: &str,
        config: &dyn ClientConfig,
        cache: CacheView,
    ) -> anyhow::Result<Box<dyn ExecutionClient>> {
        let config = config
            .as_any()
            .downcast_ref::<SodexExecClientConfig>()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "expected a SodexExecClientConfig for {}, was {config:?}",
                    self.name()
                )
            })?
            .clone();

        let venue = venue_for(config.market);
        // The venue's own numeric id, so an account event can be traced back to the account
        // that produced it without consulting the configuration again.
        let account_id = AccountId::from(format!("{venue}-{}", config.resolve_account_id()?));

        let core = ExecutionClientCore::new(
            trader_id,
            ClientId::from(name),
            venue,
            OmsType::Netting,
            account_id,
            account_type_for(config.market),
            // No single base currency: balances are held per coin on both engines.
            None,
            cache,
        );

        Ok(Box::new(SodexExecutionClient::new(
            core,
            config,
            get_atomic_clock_realtime(),
        )?))
    }

    fn name(&self) -> &'static str {
        SODEX
    }

    fn config_type(&self) -> &'static str {
        "SodexExecClientConfig"
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn spot_settles_in_cash_and_perps_on_margin() {
        // Spot holds coin balances with nothing to margin; perps posts collateral. Declaring
        // the wrong one would have the engine computing account state it cannot back.
        assert_eq!(account_type_for(Market::Spot), AccountType::Cash);
        assert_eq!(account_type_for(Market::Perps), AccountType::Margin);
    }

    #[rstest]
    fn both_factories_register_under_one_key() {
        // The engine is chosen by the config's market field, not by which factory was
        // registered, so a node wires this adapter once and gets both venues.
        assert_eq!(SodexDataClientFactory::new().name(), SODEX);
        assert_eq!(SodexExecutionClientFactory::new().name(), SODEX);
    }

    #[rstest]
    fn each_factory_names_the_config_it_accepts() {
        assert_eq!(
            SodexDataClientFactory::new().config_type(),
            "SodexDataClientConfig"
        );
        assert_eq!(
            SodexExecutionClientFactory::new().config_type(),
            "SodexExecClientConfig"
        );
    }
}
