//! Batched order acknowledgements.
//!
//! Order, cancel and replace all submit a batch and receive an array of per-order results.
//! The venue documents one case where that array does **not** line up with the request:
//!
//! > Some errors are a deterministic function of the payload itself, and these are instead
//! > returned earlier as part of pre-validation. In this case, only one error is returned
//! > for the entire payload […] the response could be duplicated n times before being sent
//! > to the callback function, as the whole batch was rejected for this same reason.
//!
//! Indexing the response positionally would therefore mislabel every order after the first
//! whenever a batch is rejected wholesale - orders 2..n would silently inherit no status at
//! all. [`align_batch`] performs the documented fan-out so callers always get exactly one
//! outcome per submitted order.

use serde::{Deserialize, Serialize};

/// Per-order outcome from a batched trading request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OrderAck {
    /// `0` on success; any other value indicates this order failed.
    pub code: i32,
    #[serde(rename = "clOrdID")]
    pub cl_ord_id: String,
    /// Present only when `code` is non-zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Venue-assigned order id. Present only when `code` is zero.
    #[serde(rename = "orderID", skip_serializing_if = "Option::is_none")]
    pub order_id: Option<u64>,
}

impl OrderAck {
    /// Success code used throughout the REST API.
    pub const OK: i32 = 0;

    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.code == Self::OK
    }
}

/// Why a batch response could not be aligned with its request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AlignError {
    #[error("submitted an empty batch")]
    EmptyRequest,
    #[error("venue returned no acknowledgements for {submitted} submitted orders")]
    EmptyResponse { submitted: usize },
    #[error(
        "venue returned {returned} acknowledgements for {submitted} submitted orders, which is \
         neither one-per-order nor a single whole-batch rejection"
    )]
    LengthMismatch { submitted: usize, returned: usize },
}

/// Pairs each submitted client order id with its outcome.
///
/// Handles the two shapes the venue can return: one acknowledgement per order, or a single
/// whole-batch rejection that applies to all of them.
///
/// # Errors
///
/// Returns [`AlignError`] when the response matches neither shape, rather than guessing.
/// A silent mismatch here would attribute one order's fate to another.
pub fn align_batch(
    submitted: &[String],
    mut acks: Vec<OrderAck>,
) -> Result<Vec<OrderAck>, AlignError> {
    if submitted.is_empty() {
        return Err(AlignError::EmptyRequest);
    }

    match acks.len() {
        0 => Err(AlignError::EmptyResponse {
            submitted: submitted.len(),
        }),
        n if n == submitted.len() => Ok(acks),
        1 => {
            // Whole-batch rejection. Fan the single reason out, restoring each order's own
            // client id so callers can still key results by the id they submitted.
            let reason = acks.remove(0);
            if reason.is_success() {
                // One success for a multi-order batch is not a shape the venue documents;
                // fanning a success out would invent order ids that were never assigned.
                return Err(AlignError::LengthMismatch {
                    submitted: submitted.len(),
                    returned: 1,
                });
            }
            Ok(submitted
                .iter()
                .map(|cl_ord_id| OrderAck {
                    cl_ord_id: cl_ord_id.clone(),
                    ..reason.clone()
                })
                .collect())
        }
        n => Err(AlignError::LengthMismatch {
            submitted: submitted.len(),
            returned: n,
        }),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn ok(cl_ord_id: &str, order_id: u64) -> OrderAck {
        OrderAck {
            code: OrderAck::OK,
            cl_ord_id: cl_ord_id.to_string(),
            error: None,
            order_id: Some(order_id),
        }
    }

    fn rejected(cl_ord_id: &str, message: &str) -> OrderAck {
        OrderAck {
            code: -1,
            cl_ord_id: cl_ord_id.to_string(),
            error: Some(message.to_string()),
            order_id: None,
        }
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_string()).collect()
    }

    #[rstest]
    fn per_order_response_passes_through_unchanged() {
        let submitted = ids(&["a", "b"]);
        let acks = vec![ok("a", 1), rejected("b", "insufficient margin")];

        let aligned = align_batch(&submitted, acks.clone()).unwrap();

        assert_eq!(aligned, acks);
    }

    #[rstest]
    fn whole_batch_rejection_fans_out_to_every_order() {
        // The documented case: three orders submitted, one pre-validation error returned.
        let submitted = ids(&["a", "b", "c"]);
        let acks = vec![rejected("a", "nonce too old")];

        let aligned = align_batch(&submitted, acks).unwrap();

        assert_eq!(aligned.len(), 3);
        assert!(aligned.iter().all(|ack| !ack.is_success()));
        assert!(
            aligned
                .iter()
                .all(|ack| ack.error.as_deref() == Some("nonce too old"))
        );
    }

    #[rstest]
    fn fanned_out_acks_keep_their_own_client_order_ids() {
        // Copying the rejection verbatim would stamp every entry with the first order's id,
        // making results impossible to key back to what was submitted.
        let submitted = ids(&["alpha", "beta", "gamma"]);
        let acks = vec![rejected("alpha", "batch too large")];

        let aligned = align_batch(&submitted, acks).unwrap();

        let recovered: Vec<&str> = aligned.iter().map(|a| a.cl_ord_id.as_str()).collect();
        assert_eq!(recovered, vec!["alpha", "beta", "gamma"]);
    }

    #[rstest]
    fn single_ack_for_single_order_is_not_treated_as_a_fan_out() {
        let submitted = ids(&["solo"]);
        let acks = vec![ok("solo", 42)];

        let aligned = align_batch(&submitted, acks).unwrap();

        assert_eq!(aligned.len(), 1);
        assert_eq!(aligned[0].order_id, Some(42));
    }

    #[rstest]
    fn lone_success_for_a_multi_order_batch_is_refused() {
        // Fanning a success out would fabricate order ids for orders the venue never
        // acknowledged. Better to surface the anomaly than to invent state.
        let submitted = ids(&["a", "b"]);
        let acks = vec![ok("a", 1)];

        assert!(matches!(
            align_batch(&submitted, acks),
            Err(AlignError::LengthMismatch {
                submitted: 2,
                returned: 1
            })
        ));
    }

    #[rstest]
    fn partial_response_is_an_error_not_a_guess() {
        let submitted = ids(&["a", "b", "c"]);
        let acks = vec![ok("a", 1), ok("b", 2)];

        assert!(matches!(
            align_batch(&submitted, acks),
            Err(AlignError::LengthMismatch {
                submitted: 3,
                returned: 2
            })
        ));
    }

    #[rstest]
    fn empty_response_is_distinguishable_from_a_mismatch() {
        let submitted = ids(&["a"]);

        assert!(matches!(
            align_batch(&submitted, vec![]),
            Err(AlignError::EmptyResponse { submitted: 1 })
        ));
    }

    #[rstest]
    fn empty_request_is_rejected() {
        assert_eq!(
            align_batch(&[], vec![]).unwrap_err(),
            AlignError::EmptyRequest
        );
    }

    #[rstest]
    fn ack_parses_both_success_and_failure_shapes() {
        let success: OrderAck =
            serde_json::from_str(r#"{"code":0,"clOrdID":"my-order-1","orderID":1234}"#).unwrap();
        assert!(success.is_success());
        assert_eq!(success.order_id, Some(1234));
        assert!(success.error.is_none());

        let failure: OrderAck = serde_json::from_str(
            r#"{"code":21104,"clOrdID":"my-order-1","error":"invalid nonce"}"#,
        )
        .unwrap();
        assert!(!failure.is_success());
        assert_eq!(failure.error.as_deref(), Some("invalid nonce"));
        assert!(failure.order_id.is_none());
    }
}
