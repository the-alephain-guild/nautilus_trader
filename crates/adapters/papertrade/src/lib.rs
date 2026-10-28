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

//! Native Papertrade data and execution clients with a durable signed-intent journal.
//!
//! Relayer acknowledgement alone never establishes a fill. Account positions, paginated
//! close history and individually bound on-chain results supply reconciliation evidence.

#![deny(unsafe_code)]
#![deny(missing_debug_implementations)]

#[cfg(test)]
extern crate self as nautilus_papertrade;

pub mod api;
pub mod chain;
pub mod config;
pub mod data;
pub mod execution;
pub mod factories;
pub mod http;
pub mod journal;
#[cfg(feature = "python")]
pub mod python;
mod reconciliation;
pub mod reference_bars;
pub mod reports;
pub mod signing;
pub mod stream;
pub mod wire;

pub use http::{IntentReceipt, IntentState, PapertradeRelayerClient, SubmissionOutcome};
pub use signing::{
    CancelIntent, Close, OpenPosition, PapertradeSigner, RegisterSessionKey, SignedIntent,
    margin_to_raw,
};
