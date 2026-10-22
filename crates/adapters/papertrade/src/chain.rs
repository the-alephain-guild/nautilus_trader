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

//! Binds relayer intent IDs to observed BatchExecutor receipt evidence.

use crate::signing::EXCHANGE;
use crate::wire::CloseRecord;
use alloy_primitives::{Address, B256, I256, U256, address, b256};
use anyhow::{Result, ensure};
use serde_json::Value;

pub const BATCH_EXECUTOR: Address = address!("233ff29b239abc1d7748605da9b205c3e008626e");
// Observed on-chain result logs were independently bound to EIP-712 IDs by
// decoding the corresponding signed calldata, not inferred from transaction success.
pub const RESULT_TOPIC: B256 =
    b256!("8a23ef1120f3e42e722a0c0a599a66d55ab2650ecc39716f57e625684e349cad");
const OPEN_TOPIC: B256 = b256!("88451ea1189f73b554f0bf39c09868560ed149b3d788fcc933ff66ba926827f3");
const CLOSE_TOPIC: B256 = b256!("87902ed2451849f465fc4d36a01d7f5c67b53af12f63ee85d2814a2d582f89bf");

pub fn intent_result(receipt: &Value, id: B256) -> Result<Option<bool>> {
    ensure!(receipt["status"] == "0x1", "Receipt is not successful");
    let logs = receipt["logs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing receipt logs"))?;
    let mut result = None;
    for log in logs {
        if log["address"] != serde_json::json!(BATCH_EXECUTOR) {
            continue;
        }
        let topics = log["topics"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid topics"))?;
        if topics.len() != 4
            || topics[0] != serde_json::json!(RESULT_TOPIC)
            || topics[3] != serde_json::json!(id)
        {
            continue;
        }
        ensure!(
            log["removed"] == false
                && log["transactionHash"] == receipt["transactionHash"]
                && log["blockHash"] == receipt["blockHash"],
            "Removed or unbound result log"
        );
        let words = data_words(&log["data"])?;
        ensure!(
            !words.is_empty() && words[0] <= U256::from(1),
            "Invalid batch result flag"
        );
        ensure!(result.is_none(), "Duplicate result for an intent");
        result = Some(words[0] == U256::from(1));
    }
    Ok(result)
}

/// Confirms the exit price, adjusted PnL and user fee against the exchange log.
pub fn verify_close(receipt: &Value, record: &CloseRecord, wallet: Address) -> Result<()> {
    ensure!(
        receipt["transactionHash"] == serde_json::json!(record.tx_hash),
        "Wrong close receipt"
    );
    let logs = receipt["logs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing receipt logs"))?;
    for log in logs {
        if log["address"] != serde_json::json!(EXCHANGE) {
            continue;
        }
        let Some(topics) = log["topics"].as_array() else {
            continue;
        };
        if topics.len() != 3 || topics[0] != serde_json::json!(CLOSE_TOPIC) {
            continue;
        }
        let position: B256 = serde_json::from_value(topics[1].clone())?;
        let user: B256 = serde_json::from_value(topics[2].clone())?;
        if U256::from_be_slice(position.as_slice()) != record.position_id.0
            || &user.as_slice()[12..] != wallet.as_slice()
        {
            continue;
        }
        ensure!(
            log["removed"] == false && log["blockHash"] == receipt["blockHash"],
            "Removed close log"
        );
        let words = data_words(&log["data"])?;
        ensure!(
            words.len() >= 5
                && words[0] == record.exit_price_raw.0
                && I256::from_raw(words[2]).to_string() == record.adjusted_pnl_raw
                && words[4] == record.user_paid_fee_raw.0,
            "Close history disagrees with chain log"
        );
        return Ok(());
    }
    anyhow::bail!("Matching exchange close event not found")
}

pub fn recovered_open(
    receipt: &Value,
    id: B256,
    wallet: Address,
    body: &Value,
) -> Result<crate::wire::VenuePosition> {
    ensure!(
        intent_result(receipt, id)? == Some(true),
        "No successful open result"
    );
    let logs = receipt["logs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing logs"))?;
    let result = logs
        .iter()
        .find(|log| {
            log["address"] == serde_json::json!(BATCH_EXECUTOR)
                && log["topics"][3] == serde_json::json!(id)
        })
        .ok_or_else(|| anyhow::anyhow!("Missing open result"))?;
    let return_words = data_words(&result["data"])?;
    ensure!(
        return_words.len() == 4
            && return_words[1] == U256::from(64)
            && return_words[2] == U256::from(32),
        "Unknown open return-data layout"
    );
    let pid = return_words[3];
    for log in logs {
        if log["address"] != serde_json::json!(EXCHANGE)
            || log["topics"][0] != serde_json::json!(OPEN_TOPIC)
        {
            continue;
        }
        let position: B256 = serde_json::from_value(log["topics"][1].clone())?;
        let user: B256 = serde_json::from_value(log["topics"][2].clone())?;
        if U256::from_be_slice(position.as_slice()) != pid
            || &user.as_slice()[12..] != wallet.as_slice()
        {
            continue;
        }
        ensure!(
            log["removed"] == false && log["blockHash"] == receipt["blockHash"],
            "Removed open log"
        );
        let words = data_words(&log["data"])?;
        let signed = &body["signed"];
        ensure!(
            words.len() == 6
                && signed["instrumentId"] == words[0].to_string()
                && signed["isLong"] == serde_json::json!(words[1] == U256::from(1))
                && signed["useDebt"] == serde_json::json!(words[2] == U256::from(1))
                && signed["size"] == words[4].to_string()
                && signed["leverage"] == words[5].to_string(),
            "Open event disagrees with signed request"
        );
        let stamp = log["blockTimestamp"]
            .as_str()
            .or_else(|| receipt["blockTimestamp"].as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing open block timestamp"))?;
        let seconds = u64::from_str_radix(stamp.trim_start_matches("0x"), 16)?;
        return Ok(crate::wire::VenuePosition {
            position_id: crate::wire::Raw(pid),
            instrument_id: u32::try_from(words[0])?,
            is_long: words[1] == U256::from(1),
            use_debt: words[2] == U256::from(1),
            entry_price: crate::wire::Raw(words[3]),
            margin: crate::wire::Raw(words[4]),
            leverage: u32::try_from(words[5])?,
            bust_price: Default::default(),
            opened_at_ms: seconds
                .checked_mul(1000)
                .ok_or_else(|| anyhow::anyhow!("Timestamp overflow"))?,
            intent_id: Some(id),
        });
    }
    anyhow::bail!("Successful open has no matching exchange event")
}
fn data_words(value: &Value) -> Result<Vec<U256>> {
    let bytes: alloy_primitives::Bytes = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing log data"))?
        .parse()?;
    ensure!(bytes.len().is_multiple_of(32), "Unaligned event data");
    Ok(bytes.chunks(32).map(U256::from_be_slice).collect())
}
