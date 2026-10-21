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

//! Typed intents and signing for the official Papertrade relayer wire protocol.

use std::fmt;

use alloy::{
    signers::{SignerSync, local::PrivateKeySigner},
    sol_types::{Eip712Domain, SolStruct, eip712_domain},
};
use alloy_primitives::{Address, B256, address, keccak256};
use anyhow::{Result, ensure};
use rust_decimal::Decimal;
use serde_json::{Value, json};

pub const CHAIN_ID: u64 = 999;
pub const EXCHANGE: Address = address!("6cd5661646289fb6e65ea5c032310fded797d0a2");
const INTENT_LIFETIME_SECS: u64 = 3_600;
const SESSION_LIFETIME_SECS: u64 = 30 * 24 * 3_600;

// Field names, order, and widths are signed protocol bytes, including the unusual
// exchange domain name "String"; changing them invalidates the signature.
alloy::sol! {
    #[derive(Debug)]
    struct OpenPosition {
        address user;
        uint32 instrumentId;
        bool isLong;
        uint128 size;
        uint32 leverage;
        bool useDebt;
        uint256 nonce;
        uint64 deadline;
    }

    #[derive(Debug)]
    struct Close {
        address user;
        uint256[] positionIds;
        uint256 nonce;
        uint64 deadline;
    }

    #[derive(Debug)]
    struct RegisterSessionKey {
        address user;
        address key;
        uint64 expiry;
        address feeAddress;
        uint256 nonce;
        uint64 deadline;
    }

    #[derive(Debug)]
    struct CancelIntent {
        address user;
        bytes32 intentId;
        uint256 deadline;
    }
}

/// Converts collateral margin to the protocol's 18-decimal integer without rounding.
///
/// This is collateral margin, not base-asset quantity or the collateral ERC-20's units.
pub fn margin_to_raw(margin: Decimal) -> Result<u128> {
    let margin = margin.normalize();
    ensure!(margin > Decimal::ZERO, "Margin must be positive");
    ensure!(margin.scale() <= 18, "Margin exceeds 18 decimal places");
    let mantissa = u128::try_from(margin.mantissa())?;
    mantissa
        .checked_mul(10u128.pow(18 - margin.scale()))
        .ok_or_else(|| anyhow::anyhow!("Margin exceeds uint128"))
}

/// A signed request with its deterministic identifier, suitable for durable journaling.
///
/// Persist this request before sending. On an uncertain submission, retain and reconcile
/// this exact identifier instead of allocating a new nonce and signing another trade.
#[derive(Clone)]
pub struct SignedIntent {
    id: B256,
    action: &'static str,
    deadline: u64,
    cancellation: bool,
    body: Value,
}

impl fmt::Debug for SignedIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignedIntent")
            .field("id", &self.id)
            .field("action", &self.action)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl SignedIntent {
    pub fn id(&self) -> B256 {
        self.id
    }

    pub fn action(&self) -> &'static str {
        self.action
    }

    pub fn deadline(&self) -> u64 {
        self.deadline
    }

    pub fn path(&self) -> String {
        if self.cancellation {
            format!("/intents/{:#x}/cancel", self.id)
        } else {
            "/intents".to_owned()
        }
    }

    /// Returns signature-bearing JSON; callers must not log this value.
    pub fn body(&self) -> &Value {
        &self.body
    }
}

/// Signs Papertrade intents with an explicitly supplied master or registered session key.
///
/// Nonce allocation is owned by the caller and must be durable and coordinated across
/// processes and frontends sharing the same account/contract nonce namespace.
pub struct PapertradeSigner {
    signer: PrivateKeySigner,
    exchange_domain: Eip712Domain,
    cancel_domain: Eip712Domain,
}

impl fmt::Debug for PapertradeSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PapertradeSigner")
            .field("address", &self.signer.address())
            .finish_non_exhaustive()
    }
}

impl PapertradeSigner {
    /// Uses the documented mainnet proxy, not its upgradeable implementation address.
    pub fn new(signer: PrivateKeySigner) -> Self {
        Self {
            signer,
            exchange_domain: eip712_domain! {
                name: "String",
                version: "1",
                chain_id: CHAIN_ID,
                verifying_contract: EXCHANGE,
            },
            cancel_domain: eip712_domain! {
                name: "PaperTrade Intent Cancel",
                version: "2",
                chain_id: CHAIN_ID,
                verifying_contract: EXCHANGE,
            },
        }
    }

    pub fn address(&self) -> Address {
        self.signer.address()
    }

    pub fn sign_open(&self, intent: &OpenPosition, now: u64) -> Result<SignedIntent> {
        validate_user_deadline(intent.user, intent.deadline, now)?;
        ensure!(intent.size > 0, "Margin must be positive");
        ensure!(intent.leverage > 0, "Leverage must be positive");
        self.sign(
            intent,
            "openPosition",
            intent.deadline,
            json!({
                "user": intent.user,
                "instrumentId": intent.instrumentId.to_string(),
                "isLong": intent.isLong,
                "size": intent.size.to_string(),
                "leverage": intent.leverage.to_string(),
                "useDebt": intent.useDebt,
                "nonce": intent.nonce.to_string(),
                "deadline": intent.deadline.to_string(),
            }),
        )
    }

    pub fn sign_close(&self, intent: &Close, now: u64) -> Result<SignedIntent> {
        validate_user_deadline(intent.user, intent.deadline, now)?;
        ensure!(
            (1..=12).contains(&intent.positionIds.len()),
            "Close requires between 1 and 12 position IDs"
        );
        let mut ids = intent.positionIds.clone();
        ids.sort_unstable();
        ensure!(
            !ids.windows(2).any(|pair| pair[0] == pair[1]),
            "Close contains duplicate position IDs"
        );
        self.sign(
            intent,
            "closePositions",
            intent.deadline,
            json!({
                "user": intent.user,
                "positionIds": intent.positionIds.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "nonce": intent.nonce.to_string(),
                "deadline": intent.deadline.to_string(),
            }),
        )
    }

    /// Registration requires the master wallet; a session key cannot authorize itself.
    pub fn sign_registration(&self, intent: &RegisterSessionKey, now: u64) -> Result<SignedIntent> {
        validate_user_deadline(intent.user, intent.deadline, now)?;
        ensure!(
            intent.user == self.address(),
            "Registration requires the master wallet"
        );
        ensure!(!intent.key.is_zero(), "Session key must not be zero");
        ensure!(
            intent.expiry > intent.deadline
                && intent.expiry.saturating_sub(now) <= SESSION_LIFETIME_SECS,
            "Session expiry must exceed the intent deadline and be within 30 days"
        );
        self.sign(
            intent,
            "registerSessionKey",
            intent.deadline,
            json!({
                "user": intent.user,
                "key": intent.key,
                "expiry": intent.expiry.to_string(),
                "feeAddress": intent.feeAddress,
                "nonce": intent.nonce.to_string(),
                "deadline": intent.deadline.to_string(),
            }),
        )
    }

    /// Cancels an intent in the relayer queue; this does not reverse a confirmed position.
    pub fn sign_cancel(
        &self,
        intent: &CancelIntent,
        target: &SignedIntent,
        now: u64,
    ) -> Result<SignedIntent> {
        let deadline = u64::try_from(intent.deadline)?;
        validate_user_deadline(intent.user, deadline, now)?;
        ensure!(
            intent.intentId == target.id && !target.cancellation,
            "Invalid cancel target"
        );
        ensure!(
            target.body["signed"]["user"] == json!(intent.user),
            "Cancellation user differs from the target user"
        );
        let digest = intent.eip712_signing_hash(&self.cancel_domain);
        let mut body = json!({
            "user": intent.user,
            "deadline": deadline.to_string(),
        });
        self.add_signature(digest, &mut body)?;
        Ok(SignedIntent {
            id: intent.intentId,
            action: target.action,
            deadline,
            cancellation: true,
            body,
        })
    }

    fn sign<T: SolStruct>(
        &self,
        intent: &T,
        action: &'static str,
        deadline: u64,
        mut fields: Value,
    ) -> Result<SignedIntent> {
        let digest = intent.eip712_signing_hash(&self.exchange_domain);
        self.add_signature(digest, &mut fields)?;
        let mut preimage = Vec::with_capacity(84);
        preimage.extend_from_slice(keccak256(b"papertrade:intent-id:v2").as_slice());
        preimage.extend_from_slice(digest.as_slice());
        preimage.extend_from_slice(self.address().as_slice());
        Ok(SignedIntent {
            id: keccak256(preimage),
            action,
            deadline,
            cancellation: false,
            body: json!({"action": action, "signed": fields}),
        })
    }

    fn add_signature(&self, digest: B256, fields: &mut Value) -> Result<()> {
        let signature = self.signer.sign_hash_sync(&digest)?;
        fields["v"] = json!(if signature.v() { "28" } else { "27" });
        fields["r"] = json!(format!("0x{:064x}", signature.r()));
        fields["s"] = json!(format!("0x{:064x}", signature.s()));
        Ok(())
    }
}

fn validate_user_deadline(user: Address, deadline: u64, now: u64) -> Result<()> {
    ensure!(!user.is_zero(), "User must not be zero");
    ensure!(
        deadline > now && deadline - now <= INTENT_LIFETIME_SECS,
        "Intent deadline must be in the next hour"
    );
    Ok(())
}
