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

//! ATR-normalized margin strategy for short-horizon binary outcome markets.
//!
//! Lives under `examples/` rather than in the `trading` crate because it consumes an
//! adapter-specific payload (the venue's TWAP stream). The `trading` crate depends on no
//! adapter, and this adapter already depends on `trading` for development targets, so
//! placing the strategy there would invert that direction.

pub(crate) mod config;
pub(crate) mod decision;
pub(crate) mod journal;
pub(crate) mod strategy;

pub(crate) use config::AtrMarginBinaryConfig;
pub(crate) use strategy::AtrMarginBinary;
