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

//! Papertrade's HyperEVM signed-intent protocol.
//!
//! This crate prepares authenticated relayer requests, not HyperCore exchange actions.
//! It intentionally exposes no Nautilus live client or factory: acknowledgement and even
//! a confirmed intent do not contain sufficient evidence to emit an OrderFilled event.

#![deny(unsafe_code)]
#![deny(missing_debug_implementations)]

pub mod http;
pub mod signing;

pub use http::{IntentReceipt, IntentState, PapertradeRelayerClient, SubmissionOutcome};
pub use signing::{
    CancelIntent, Close, OpenPosition, PapertradeSigner, RegisterSessionKey, SignedIntent,
    margin_to_raw,
};
