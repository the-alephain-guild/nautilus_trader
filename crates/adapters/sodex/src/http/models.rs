//! Wire types shared by every REST response.

use serde::{Deserialize, Serialize};

/// The envelope every REST endpoint wraps its payload in.
///
/// Per the venue's REST overview the envelope carries `code` for status, `timestamp` for
/// server time in milliseconds, `error` when the request failed, and `data` for the
/// endpoint-specific payload.
///
/// # On deciding success
///
/// Success is `code == 0`: the endpoint reference states the payload is returned "when code
/// is `0`" for every endpoint, and per-order acknowledgements use the same convention
/// (see [`orders::OrderAck::OK`](super::orders::OrderAck::OK)).
///
/// Both signals are checked, and either one alone is enough to fail the request. An `error`
/// with a zero code, or a non-zero code with no message, are both shapes that should not
/// reach a caller as success - treating one signal as authoritative and ignoring the other
/// is how a rejection gets read as an empty result.
///
/// The code is compared leniently against `0` and `"0"` because the envelope carries it as
/// untyped JSON, and a venue that switches between a number and a numeric string should not
/// silently flip every response to "failed".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    /// Venue status code. Retained verbatim for diagnostics.
    pub code: Option<serde_json::Value>,
    /// Server time in milliseconds.
    ///
    /// Worth surfacing rather than discarding: nonces must sit inside a window measured
    /// against venue time, so this is the authoritative reference for detecting clock skew.
    pub timestamp: Option<u64>,
    /// Present when the request failed.
    pub error: Option<String>,
    /// Endpoint-specific payload.
    pub data: Option<T>,
}

impl<T> ApiResponse<T> {
    /// Unwraps the payload, or reports the venue's error.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::Venue`] when `error` is set, and
    /// [`EnvelopeError::MissingData`] when the request succeeded but carried no payload.
    pub fn into_result(self) -> Result<T, EnvelopeError> {
        let code_text = self.code.as_ref().map(ToString::to_string);

        if let Some(message) = self.error {
            return Err(EnvelopeError::Venue {
                code: code_text,
                message,
            });
        }

        if self.code.as_ref().is_some_and(|code| !is_success(code)) {
            return Err(EnvelopeError::Venue {
                code: code_text,
                message: "venue reported a failure code without an error message".to_string(),
            });
        }

        self.data.ok_or(EnvelopeError::MissingData)
    }
}

/// Whether a raw envelope code means success.
fn is_success(code: &serde_json::Value) -> bool {
    match code {
        serde_json::Value::Number(n) => n.as_i64() == Some(0),
        serde_json::Value::String(s) => s == "0",
        _ => false,
    }
}

/// Failures expressed by the response envelope itself, as opposed to transport failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("venue rejected the request{}: {message}", .code.as_deref().map_or(String::new(), |c| format!(" (code {c})")))]
    Venue {
        code: Option<String>,
        message: String,
    },
    #[error("response reported success but carried no data payload")]
    MissingData,
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Payload {
        aid: u64,
    }

    #[rstest]
    fn success_envelope_yields_payload() {
        let raw = r#"{"code":0,"timestamp":1760373925000,"data":{"aid":12345}}"#;
        let parsed: ApiResponse<Payload> = serde_json::from_str(raw).unwrap();

        assert_eq!(parsed.timestamp, Some(1_760_373_925_000));
        assert_eq!(parsed.into_result().unwrap(), Payload { aid: 12345 });
    }

    #[rstest]
    fn error_envelope_surfaces_message_and_code() {
        let raw = r#"{"code":21104,"timestamp":1760373925000,"error":"invalid nonce"}"#;
        let parsed: ApiResponse<Payload> = serde_json::from_str(raw).unwrap();

        let err = parsed.into_result().unwrap_err();
        assert!(matches!(err, EnvelopeError::Venue { .. }));
        assert!(err.to_string().contains("invalid nonce"), "{err}");
        assert!(err.to_string().contains("21104"), "{err}");
    }

    #[rstest]
    fn error_takes_precedence_even_when_data_is_present() {
        // Defensive: if the venue ever returns both, the failure must win rather than a
        // partially-populated payload being handed to a caller as success.
        let raw = r#"{"code":1,"error":"rejected","data":{"aid":1}}"#;
        let parsed: ApiResponse<Payload> = serde_json::from_str(raw).unwrap();

        assert!(matches!(
            parsed.into_result(),
            Err(EnvelopeError::Venue { .. })
        ));
    }

    #[rstest]
    fn success_without_payload_is_distinguishable_from_venue_error() {
        // Some endpoints document "no endpoint-specific data"; callers expecting a payload
        // should see a distinct error rather than a confusing venue message.
        let raw = r#"{"code":0,"timestamp":1760373925000}"#;
        let parsed: ApiResponse<Payload> = serde_json::from_str(raw).unwrap();

        assert_eq!(
            parsed.into_result().unwrap_err(),
            EnvelopeError::MissingData
        );
    }

    #[rstest]
    fn nonzero_code_fails_even_without_an_error_message() {
        // Keying only off `error` would hand this to the caller as MissingData, hiding a
        // rejection behind what looks like an empty result.
        let raw = r#"{"code":21104,"timestamp":1,"data":{"aid":1}}"#;
        let parsed: ApiResponse<Payload> = serde_json::from_str(raw).unwrap();

        let err = parsed.into_result().unwrap_err();
        assert!(matches!(err, EnvelopeError::Venue { .. }));
        assert!(err.to_string().contains("21104"), "{err}");
    }

    #[rstest]
    fn numeric_string_code_is_still_success() {
        let raw = r#"{"code":"0","timestamp":1,"data":{"aid":9}}"#;
        let parsed: ApiResponse<Payload> = serde_json::from_str(raw).unwrap();

        assert_eq!(parsed.into_result().unwrap(), Payload { aid: 9 });
    }

    #[rstest]
    fn absent_code_does_not_by_itself_fail_the_response() {
        // Not every endpoint documents a code; absence must not be read as failure.
        let raw = r#"{"timestamp":1,"data":{"aid":3}}"#;
        let parsed: ApiResponse<Payload> = serde_json::from_str(raw).unwrap();

        assert_eq!(parsed.into_result().unwrap(), Payload { aid: 3 });
    }

    #[rstest]
    fn unknown_envelope_fields_do_not_break_parsing() {
        // The venue can add fields; a strict decoder would turn that into an outage.
        let raw = r#"{"code":0,"timestamp":1,"data":{"aid":7},"newField":"ignored"}"#;
        let parsed: ApiResponse<Payload> = serde_json::from_str(raw).unwrap();

        assert_eq!(parsed.into_result().unwrap(), Payload { aid: 7 });
    }
}
