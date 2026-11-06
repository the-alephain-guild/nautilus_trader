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

//! Real owner phase delivery retains the original emulated OCO lifecycle.

use rstest::rstest;

#[rstest]
#[ignore = "requires explicit CUSTOS_SOURCE_ROOT and compatible Custos PYO3_PYTHON"]
fn test_cross_repo_owner_cancel_delivery_precedes_oco_continuation() {
    super::hosted_control_cancel::run_control_cancel(true, true);
}

#[rstest]
#[case::queued_pause("retire_queued_paused")]
#[case::queued_stop("retire_queued_stopped")]
#[case::queued_spec("retire_queued_spec")]
#[case::oco_child_retired("retire_after_first")]
#[case::callback_failure("callback_failure")]
#[case::scope_exit_failure("exit_failure")]
#[case::stop_with_queued_work("stop_after_queue")]
#[case::retirement_writer_waits_for_actual_phase("retire_writer_first")]
#[case::foreign_thread_control_is_refused("wrong_thread")]
#[case::capacity_keeps_original_emulator_continuation("capacity_native_continuation")]
#[ignore = "requires explicit CUSTOS_SOURCE_ROOT and compatible Custos PYO3_PYTHON"]
fn test_cross_repo_owner_control_retirement_and_failure_keeps_real_native_history(
    #[case] scenario: &str,
) {
    super::hosted_control_cancel::run_control_scenario(true, true, scenario);
}

#[rstest]
#[case::busy_registered_endpoint("raw_submit_busy")]
#[case::free_registered_endpoint("raw_submit_free")]
#[ignore = "requires explicit CUSTOS_SOURCE_ROOT and compatible Custos PYO3_PYTHON"]
fn test_cross_repo_owner_control_does_not_admit_raw_submit(#[case] scenario: &str) {
    super::hosted_control_cancel::run_control_scenario(true, true, scenario);
}
