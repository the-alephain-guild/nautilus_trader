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

//! Native factories and client config extraction.

use crate::{
    config::{PAPERTRADE, PapertradeDataClientConfig, PapertradeExecutionClientConfig},
    data::PapertradeDataClient,
    execution::PapertradeExecutionClient,
};
use nautilus_common::{
    cache::CacheView,
    clients::{DataClient, ExecutionClient},
    clock::Clock,
    factories::{ClientConfig, DataClientFactory, ExecutionClientFactory},
};
use nautilus_live::ExecutionClientCore;
use nautilus_model::{
    enums::{AccountType, OmsType},
    identifiers::{ClientId, TraderId, Venue},
    types::Currency,
};
use std::{any::Any, cell::RefCell, rc::Rc};

impl ClientConfig for PapertradeDataClientConfig {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl ClientConfig for PapertradeExecutionClientConfig {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.papertrade", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.papertrade")
)]
pub struct PapertradeDataClientFactory;
impl DataClientFactory for PapertradeDataClientFactory {
    fn name(&self) -> &'static str {
        PAPERTRADE
    }
    fn config_type(&self) -> &'static str {
        "PapertradeDataClientConfig"
    }
    fn create(
        &self,
        name: &str,
        config: &dyn ClientConfig,
        _cache: CacheView,
        _clock: Rc<RefCell<dyn Clock>>,
    ) -> anyhow::Result<Box<dyn DataClient>> {
        let config = config
            .as_any()
            .downcast_ref::<PapertradeDataClientConfig>()
            .ok_or_else(|| anyhow::anyhow!("Expected PapertradeDataClientConfig"))?
            .clone();
        Ok(Box::new(PapertradeDataClient::new(
            ClientId::from(name),
            config,
        )?))
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.papertrade", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.papertrade")
)]
pub struct PapertradeExecutionClientFactory;
impl ExecutionClientFactory for PapertradeExecutionClientFactory {
    fn name(&self) -> &'static str {
        PAPERTRADE
    }
    fn config_type(&self) -> &'static str {
        "PapertradeExecutionClientConfig"
    }
    fn create(
        &self,
        trader_id: TraderId,
        name: &str,
        config: &dyn ClientConfig,
        cache: CacheView,
    ) -> anyhow::Result<Box<dyn ExecutionClient>> {
        let config = config
            .as_any()
            .downcast_ref::<PapertradeExecutionClientConfig>()
            .ok_or_else(|| anyhow::anyhow!("Expected PapertradeExecutionClientConfig"))?
            .clone();
        let core = ExecutionClientCore::new(
            trader_id,
            ClientId::from(name),
            Venue::from(PAPERTRADE),
            OmsType::Hedging,
            config.account_id,
            AccountType::Margin,
            Some(Currency::USDC()),
            cache,
        );
        Ok(Box::new(PapertradeExecutionClient::new(core, config)?))
    }
}
