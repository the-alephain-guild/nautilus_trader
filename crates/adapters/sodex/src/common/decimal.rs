//! Normalization of the venue's decimal strings.
//!
//! The venue emits amounts at on-chain token precision - a volume comes back as
//! `"0.001390000000000000"`, eighteen decimal places. Nautilus's fixed-point types cap
//! precision at [`FIXED_PRECISION`] and reject anything longer, so every decimal string
//! crossing this boundary is normalized first.
//!
//! Two steps, in order, because they have different costs:
//!
//! 1. **Strip trailing zeros.** Lossless, and enough on its own for the common case: those
//!    eighteen places are almost always padding.
//! 2. **Round what remains.** Only reached when a value carries more than
//!    [`FIXED_PRECISION`] *significant* decimals, which no tick or step size on this venue
//!    does - the finest observed is `0.00001`.
//!
//! Doing this at the boundary rather than at each call site is what keeps the rest of the
//! adapter free of precision defense.

use std::str::FromStr;

use nautilus_model::types::fixed::FIXED_PRECISION;
use rust_decimal::Decimal;

/// Why a decimal string could not be normalized.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a decimal number: {0:?}")]
pub struct DecimalError(pub String);

/// Rewrites a venue decimal string so its precision fits the engine's fixed-point types.
///
/// # Errors
///
/// Returns [`DecimalError`] if the input is not a decimal number.
pub fn normalize(raw: &str) -> Result<String, DecimalError> {
    let value = Decimal::from_str(raw).map_err(|_| DecimalError(raw.to_string()))?;

    // `normalize` drops trailing zeros; most venue values need nothing further.
    let trimmed = value.normalize();

    let capped = if trimmed.scale() > u32::from(FIXED_PRECISION) {
        trimmed.round_dp(u32::from(FIXED_PRECISION))
    } else {
        trimmed
    };

    Ok(capped.to_string())
}

/// Rewrites a venue decimal string at exactly `precision` decimal places.
///
/// [`normalize`] makes a value representable; this makes two values *comparable*. Nautilus
/// rejects a quote whose bid and ask carry different precisions, and the venue happily sends
/// a bid of `"77378.5"` beside an ask of `"77379"` - the same tick size, written two ways.
/// Pinning both to the instrument's declared precision is what keeps that from being read as
/// a malformed quote.
///
/// Rounding is half-even and explicit. Excess places beyond the instrument's own precision
/// are below its tick size and cannot describe a real price.
///
/// # Errors
///
/// Returns [`DecimalError`] if the input is not a decimal number, or if `precision` exceeds
/// what the engine's fixed-point types can hold.
pub fn normalize_to(raw: &str, precision: u8) -> Result<String, DecimalError> {
    if precision > FIXED_PRECISION {
        return Err(DecimalError(format!(
            "{raw:?} requested at {precision} decimal places, beyond the engine's {FIXED_PRECISION}"
        )));
    }

    let mut value = Decimal::from_str(raw).map_err(|_| DecimalError(raw.to_string()))?;
    value = value.round_dp(u32::from(precision));
    // `round_dp` bounds the scale but does not set it, so a whole number stays scale 0 and
    // would parse back at precision 0. `rescale` pads it out to the declared precision.
    value.rescale(u32::from(precision));

    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn strips_on_chain_padding_without_changing_the_value() {
        // The case that broke a live fetch: an 18-decimal volume that is really 5 decimals.
        assert_eq!(normalize("0.001390000000000000").unwrap(), "0.00139");
        assert_eq!(normalize("4.12298000").unwrap(), "4.12298");
    }

    #[rstest]
    fn leaves_already_short_values_alone() {
        assert_eq!(normalize("91976").unwrap(), "91976");
        assert_eq!(normalize("0.00001").unwrap(), "0.00001");
        assert_eq!(normalize("1").unwrap(), "1");
    }

    #[rstest]
    fn result_never_exceeds_the_engine_precision() {
        // The property that matters: whatever comes in, the output can be parsed by the
        // engine's fixed-point types.
        for raw in [
            "0.001390000000000000",
            "0.123456789012345678",
            "1234.000000000000000001",
            "0.000000000000000001",
        ] {
            let normalized = normalize(raw).unwrap();
            let scale = Decimal::from_str(&normalized).unwrap().scale();
            assert!(
                scale <= u32::from(FIXED_PRECISION),
                "{raw} normalized to {normalized} with scale {scale}"
            );
        }
    }

    #[rstest]
    fn rounds_only_when_significant_digits_actually_exceed_the_cap() {
        // Rounding is lossy, so it must not trigger on padding alone. This value has more
        // significant decimals than the cap and is the one case that legitimately rounds.
        let rounded = normalize("0.123456789012345678").unwrap();
        let scale = Decimal::from_str(&rounded).unwrap().scale();

        assert_eq!(scale, u32::from(FIXED_PRECISION));
        assert!(rounded.starts_with("0.1234567"));
    }

    #[rstest]
    fn a_value_below_the_representable_minimum_becomes_zero_not_an_error() {
        // 1e-18 cannot be represented; rounding yields zero. Surfacing that as a parse
        // failure would drop an entire series over one dust-sized field.
        let normalized = normalize("0.000000000000000001").unwrap();

        assert_eq!(Decimal::from_str(&normalized).unwrap(), Decimal::ZERO);
    }

    #[rstest]
    fn negative_values_survive() {
        // Funding rates and PnL can be negative; normalization must not mangle the sign.
        assert_eq!(normalize("-0.0000125000").unwrap(), "-0.0000125");
    }

    #[rstest]
    fn non_numeric_input_is_reported_rather_than_defaulted() {
        assert!(normalize("").is_err());
        assert!(normalize("abc").is_err());
        assert!(normalize("1.2.3").is_err());
    }
}

#[cfg(test)]
mod precision_tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn a_whole_number_is_padded_out_to_the_declared_precision() {
        // Without this, an ask of "77379" parses at precision 0 while a bid of "77378.5"
        // parses at 1, and the engine rejects the pair as inconsistent.
        assert_eq!(normalize_to("77379", 1).unwrap(), "77379.0");
        assert_eq!(normalize_to("77379", 0).unwrap(), "77379");
    }

    #[rstest]
    fn both_sides_of_a_quote_end_up_at_one_precision() {
        assert_eq!(normalize_to("77378.5", 1).unwrap(), "77378.5");
        assert_eq!(normalize_to("77379", 1).unwrap(), "77379.0");
    }

    #[rstest]
    fn on_chain_padding_is_cut_to_the_instrument_precision() {
        assert_eq!(normalize_to("0.001390000000000000", 5).unwrap(), "0.00139");
    }

    #[rstest]
    fn places_below_the_tick_are_rounded_away() {
        assert_eq!(normalize_to("77378.567", 2).unwrap(), "77378.57");
    }

    #[rstest]
    fn a_precision_the_engine_cannot_hold_is_refused() {
        // Silently clamping would produce a value that claims a precision it does not have.
        assert!(normalize_to("1.0", FIXED_PRECISION + 1).is_err());
    }

    #[rstest]
    fn a_non_number_is_still_refused() {
        assert!(normalize_to("not-a-number", 2).is_err());
    }
}
