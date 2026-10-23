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

use alloy_primitives::B256;
use rstest::rstest;
#[path = "support/scan.rs"]
mod scan;
use scan::{ScanCoverage, ScanEvent};

fn page(from: u64, to: u64, intents: &[B256], bound: u64) -> ScanEvent {
    ScanEvent::Logs {
        from,
        to,
        intents: intents.to_vec(),
        bound,
        transactions: Some(Vec::new()),
    }
}

#[rstest]
#[case::missing_middle(false)]
#[case::wrong_intent(true)]
fn rejects_gap_even_when_first_last_and_page_sizes_are_valid(#[case] wrong_intent: bool) {
    let a = B256::repeat_byte(1);
    let b = B256::repeat_byte(2);
    let mut audit = ScanCoverage::new([(a, 999), (b, 1499)]);
    let mut events = vec![
        page(1000, 1999, &[a, b], 5000),
        page(1500, 2499, &[a, b], 5000),
    ];
    audit
        .verify(&events, &[(a, 2499, false), (b, 2499, false)], 5000)
        .unwrap();
    if wrong_intent {
        events.push(page(2500, 3499, &[a], 5000));
        events.push(page(3500, 4499, &[a], 5000));
    }
    events.push(page(4500, 5000, &[a, b], 5000));
    events.push(page(5001, 5500, &[a, b], 5500));
    let error = audit
        .verify(&events, &[(a, 2499, false), (b, 5500, true)], 5500)
        .unwrap_err();
    assert!(error.to_string().contains("Unscanned gap"));
}

#[rstest]
#[case::logs_unavailable(false)]
#[case::receipt_unavailable(true)]
fn failed_reads_cannot_certify_a_cursor(#[case] log_succeeded: bool) {
    let id = B256::repeat_byte(1);
    let tx = B256::repeat_byte(3);
    let mut audit = ScanCoverage::new([(id, 999)]);
    let mut events = vec![
        ScanEvent::Logs {
            from: 1000,
            to: 1999,
            intents: vec![id],
            bound: 2999,
            transactions: log_succeeded.then_some(vec![tx]),
        },
        ScanEvent::Receipt {
            hash: tx,
            available: false,
        },
    ];
    // A later successful receipt belongs to another page, not the failed page.
    events.push(page(2000, 2999, &[id], 2999));
    events.push(ScanEvent::Receipt {
        hash: tx,
        available: true,
    });
    assert!(
        audit
            .verify(&events, &[(id, 1999, false)], 2999)
            .unwrap_err()
            .to_string()
            .contains("Unscanned gap")
    );
}

#[rstest]
fn enforces_request_bound_cursor_bound_expiry_and_monotonic_progress() {
    let id = B256::repeat_byte(1);
    let mut audit = ScanCoverage::new([(id, 999)]);
    let events = vec![page(1000, 1999, &[id], 1999)];
    assert!(
        audit
            .verify(&[page(1000, 1999, &[id], 1500)], &[(id, 999, false)], 1999)
            .unwrap_err()
            .to_string()
            .contains("Log page exceeds source bound")
    );
    assert!(
        audit
            .verify(&events, &[(id, 2000, false)], 1999)
            .unwrap_err()
            .to_string()
            .contains("Cursor exceeds source bound")
    );
    assert!(
        audit
            .verify(&events, &[(id, 1999, true)], 2000)
            .unwrap_err()
            .to_string()
            .contains("Expired before source bound")
    );
    audit.verify(&events, &[(id, 1999, false)], 2000).unwrap();
    assert!(
        audit
            .verify(&events, &[(id, 999, false)], 2000)
            .unwrap_err()
            .to_string()
            .contains("Cursor regressed")
    );
}

#[rstest]
fn accepts_overlap_partial_commits_receipts_and_growth_across_checkpoints() {
    let a = B256::repeat_byte(1);
    let b = B256::repeat_byte(2);
    let tx = B256::repeat_byte(3);
    let mut audit = ScanCoverage::new([(a, 999), (b, 1499)]);
    let mut events = vec![page(1000, 1999, &[a, b], 2499)];
    audit
        .verify(&events, &[(a, 1999, false), (b, 1499, false)], 2499)
        .unwrap();
    events.extend([
        ScanEvent::Logs {
            from: 1500,
            to: 2499,
            intents: vec![a, b],
            bound: 2499,
            transactions: Some(vec![tx]),
        },
        ScanEvent::Receipt {
            hash: tx,
            available: true,
        },
    ]);
    audit
        .verify(&events, &[(a, 2499, false), (b, 2499, false)], 2499)
        .unwrap();
    events.push(page(2500, 3499, &[b], 3499));
    audit
        .verify(&events, &[(a, 2499, false), (b, 3499, true)], 3499)
        .unwrap();
}
