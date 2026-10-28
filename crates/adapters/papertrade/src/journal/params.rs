// Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
// Licensed under the GNU Lesser General Public License Version 3.0.
// https://www.gnu.org/licenses/lgpl-3.0.en.html

use alloy_primitives::{B256, keccak256};
use anyhow::Result;
use nautilus_core::Params;

/// Hashes the parameter value actually accepted by Strategy::submit_order.
pub(crate) fn submission_params_keccak256(params: Option<&Params>) -> Result<B256> {
    let mut value = serde_json::to_value(params.filter(|params| !params.is_empty()))?;
    value.sort_all_objects();
    Ok(keccak256(serde_json::to_vec(&value)?))
}
