//! Venue enumerations.
//!
//! # The wire representation is asymmetric
//!
//! The venue states that request fields carry the **integer** value while REST and WebSocket
//! response fields carry the corresponding **status string**. Each type here therefore
//! serializes as an integer and deserializes from either form: the outbound direction must
//! match what the signature commits to, and the inbound direction has to accept what the
//! venue actually sends.
//!
//! # Values are not interchangeable across types
//!
//! Several names appear in more than one enumeration with different numbers - `EXPIRED` is
//! 6 in [`OrderStatus`] but 7 in [`ExecutionType`], and [`OrderStatus`] jumps from 6 to 10.
//! Sharing one integer type across both would compile and be wrong, so they are distinct
//! types with no conversion between them.
//!
//! # Some variants exist but cannot be used
//!
//! The venue documents variants it does not yet accept in order placement: `FOK`,
//! [`PositionSide::Long`] / [`PositionSide::Short`], and [`TriggerType::LastPrice`] /
//! [`TriggerType::IndexPrice`]. They are represented so responses parse, and
//! `is_supported_for_placement` marks them so a caller can refuse early rather than
//! discovering it as a venue rejection.

use std::fmt::Display;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Declares an enum with the venue's dual wire representation.
///
/// Generates the integer and string mappings together so the two cannot drift apart, which
/// is the failure this shape is guarding against - a hand-written pair would let one side be
/// updated without the other.
macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $( $(#[$vmeta:meta])* $variant:ident = $int:literal => $text:literal ),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        impl $name {
            /// The integer form used in request payloads.
            #[must_use]
            pub const fn as_int(self) -> u8 {
                match self { $( Self::$variant => $int ),+ }
            }

            /// The status string used in responses.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $text ),+ }
            }

            /// Parses the integer form.
            #[must_use]
            pub const fn from_int(value: u8) -> Option<Self> {
                match value { $( $int => Some(Self::$variant), )+ _ => None }
            }

            /// Parses the status string.
            #[must_use]
            pub fn from_str_value(value: &str) -> Option<Self> {
                match value { $( $text => Some(Self::$variant), )+ _ => None }
            }

            /// Every variant, for exhaustive checks in tests.
            #[must_use]
            pub const fn all() -> &'static [Self] {
                &[ $( Self::$variant ),+ ]
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_u8(self.as_int())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                use serde::de::Error as _;
                match serde_json::Value::deserialize(deserializer)? {
                    serde_json::Value::String(text) => Self::from_str_value(&text).ok_or_else(|| {
                        D::Error::custom(format!(
                            "unknown {} status string: {text:?}", stringify!($name)
                        ))
                    }),
                    serde_json::Value::Number(number) => {
                        let raw = number.as_u64().and_then(|v| u8::try_from(v).ok());
                        raw.and_then(Self::from_int).ok_or_else(|| {
                            D::Error::custom(format!(
                                "unknown {} integer value: {number}", stringify!($name)
                            ))
                        })
                    }
                    other => Err(D::Error::custom(format!(
                        "expected {} as string or integer, was {other}", stringify!($name)
                    ))),
                }
            }
        }
    };
}

wire_enum! {
    /// Order direction.
    OrderSide {
        Buy = 1 => "BUY",
        Sell = 2 => "SELL",
    }
}

wire_enum! {
    /// Order type.
    OrderType {
        Limit = 1 => "LIMIT",
        Market = 2 => "MARKET",
    }
}

wire_enum! {
    /// How long an order rests.
    TimeInForce {
        /// Rests until cancelled.
        Gtc = 1 => "GTC",
        /// Fill or kill. Documented as **not supported yet**.
        Fok = 2 => "FOK",
        /// Immediate or cancel. Required for market orders.
        Ioc = 3 => "IOC",
        /// Post only: expires if it would fill immediately.
        Gtx = 4 => "GTX",
    }
}

impl TimeInForce {
    /// Whether the venue currently accepts this value when placing an order.
    #[must_use]
    pub const fn is_supported_for_placement(self) -> bool {
        !matches!(self, Self::Fok)
    }
}

wire_enum! {
    /// Lifecycle state of an order.
    ///
    /// Distinct from [`ExecutionType`]: the two share names but not values.
    OrderStatus {
        New = 1 => "NEW",
        PartiallyFilled = 2 => "PARTIALLY_FILLED",
        Filled = 3 => "FILLED",
        Canceled = 4 => "CANCELED",
        Rejected = 5 => "REJECTED",
        Expired = 6 => "EXPIRED",
        /// Perps only, for triggered TP/SL orders. Note the gap: 6 to 10.
        Triggered = 10 => "TRIGGERED",
    }
}

wire_enum! {
    /// What happened to an order in an execution report.
    ///
    /// Distinct from [`OrderStatus`]: `EXPIRED` is 7 here and 6 there.
    ExecutionType {
        New = 1 => "NEW",
        PartiallyFilled = 2 => "PARTIALLY_FILLED",
        Filled = 3 => "FILLED",
        Canceled = 4 => "CANCELED",
        Rejected = 5 => "REJECTED",
        /// Size reduced only.
        Modified = 6 => "MODIFIED",
        Expired = 7 => "EXPIRED",
        /// Both size and price changed.
        Replaced = 8 => "REPLACED",
    }
}

wire_enum! {
    /// Order modifier. Perps only.
    OrderModifier {
        Normal = 1 => "NORMAL",
        Stop = 2 => "STOP",
        /// The primary leg of a TP/SL group.
        Bracket = 3 => "BRACKET",
        /// A stop leg attached to a TP/SL group.
        AttachedStop = 4 => "ATTACHED_STOP",
    }
}

wire_enum! {
    /// Margin mode. Perps only.
    MarginMode {
        Isolated = 1 => "ISOLATED",
        Cross = 2 => "CROSS",
    }
}

wire_enum! {
    /// Position side. Perps only.
    PositionSide {
        /// One-way mode, the only value accepted when placing orders.
        Both = 1 => "BOTH",
        /// Hedge mode. Documented as **not supported in order placement yet**.
        Long = 2 => "LONG",
        /// Hedge mode. Documented as **not supported in order placement yet**.
        Short = 3 => "SHORT",
    }
}

impl PositionSide {
    /// Whether the venue currently accepts this value when placing an order.
    #[must_use]
    pub const fn is_supported_for_placement(self) -> bool {
        matches!(self, Self::Both)
    }
}

wire_enum! {
    /// Which side of a stop a trigger represents. Perps only.
    StopType {
        StopLoss = 1 => "STOP_LOSS",
        TakeProfit = 2 => "TAKE_PROFIT",
    }
}

wire_enum! {
    /// Which price feed arms a stop. Perps only.
    TriggerType {
        /// Documented as **not supported in order placement yet**.
        LastPrice = 1 => "LAST_PRICE",
        MarkPrice = 2 => "MARK_PRICE",
        /// Documented as **not supported in order placement yet**.
        IndexPrice = 3 => "INDEX_PRICE",
    }
}

impl TriggerType {
    /// Whether the venue currently accepts this value when placing an order.
    #[must_use]
    pub const fn is_supported_for_placement(self) -> bool {
        matches!(self, Self::MarkPrice)
    }
}

wire_enum! {
    /// Signature family, mirroring the byte prefixed to a signature.
    SignatureType {
        /// Engine-specific domain (`spot`/`futures`); prefix `0x01`.
        Eip712 = 1 => "EIP712",
        /// Universal domain for cross-engine user actions; prefix `0x02`.
        Eip712Universal = 2 => "EIP712_UNIVERSAL",
    }
}

/// Permissions withheld from an API key.
///
/// The venue's encoding is inverted from the intuitive reading: a bit set to `1` means the
/// permission is **disabled**, and omitting the field enables everything. A caller that reads
/// the mask as "bits I am granting" would build a key with precisely the permissions it meant
/// to withhold, so the type is named for what it actually expresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisabledPermissions(u64);

impl DisabledPermissions {
    /// Place, replace, modify and cancel orders; schedule cancels; update leverage, margin
    /// and collateral.
    pub const TRADE: u64 = 1;
    /// Cancel orders and schedule cancels.
    pub const CANCEL: u64 = 2;
    /// Withdraw to EVM or the other engine.
    pub const WITHDRAW: u64 = 4;
    /// Sub-account and internal transfers.
    pub const TRANSFER: u64 = 8;

    /// Nothing withheld: every permission enabled.
    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    /// Withholds the given permission.
    #[must_use]
    pub const fn disabling(self, permission: u64) -> Self {
        Self(self.0 | permission)
    }

    /// Whether the given permission is withheld.
    #[must_use]
    pub const fn is_disabled(self, permission: u64) -> bool {
        self.0 & permission != 0
    }

    /// The raw mask for the `permissions` field.
    #[must_use]
    pub const fn as_mask(self) -> u64 {
        self.0
    }

    /// A signing key that may only cancel and nothing else.
    ///
    /// The natural credential for a process that must be able to wind down exposure without
    /// being able to open any.
    #[must_use]
    pub const fn cancel_only() -> Self {
        Self::none()
            .disabling(Self::TRADE)
            .disabling(Self::WITHDRAW)
            .disabling(Self::TRANSFER)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn integer_values_match_the_venue_tables() {
        assert_eq!(OrderSide::Buy.as_int(), 1);
        assert_eq!(OrderSide::Sell.as_int(), 2);
        assert_eq!(OrderType::Limit.as_int(), 1);
        assert_eq!(OrderType::Market.as_int(), 2);
        assert_eq!(TimeInForce::Gtc.as_int(), 1);
        assert_eq!(TimeInForce::Ioc.as_int(), 3);
        assert_eq!(TimeInForce::Gtx.as_int(), 4);
        assert_eq!(OrderModifier::Bracket.as_int(), 3);
        assert_eq!(PositionSide::Both.as_int(), 1);
        assert_eq!(StopType::TakeProfit.as_int(), 2);
        assert_eq!(TriggerType::MarkPrice.as_int(), 2);
    }

    #[rstest]
    fn signature_type_values_match_the_prefix_bytes_used_when_signing() {
        // Cross-check against the signing layer: the venue prefixes 0x01 to exchange-domain
        // signatures and 0x02 to universal-domain ones, and the enum agrees.
        use crate::common::SignatureKind;

        assert_eq!(
            SignatureType::Eip712.as_int(),
            SignatureKind::Exchange.prefix()
        );
        assert_eq!(
            SignatureType::Eip712Universal.as_int(),
            SignatureKind::Universal.prefix()
        );
    }

    #[rstest]
    fn expired_differs_between_order_status_and_execution_type() {
        // The whole reason these are separate types. If one integer type were shared, this
        // discrepancy would silently mislabel expiries in one direction or the other.
        assert_eq!(OrderStatus::Expired.as_int(), 6);
        assert_eq!(ExecutionType::Expired.as_int(), 7);
        assert_ne!(
            OrderStatus::Expired.as_int(),
            ExecutionType::Expired.as_int()
        );
    }

    #[rstest]
    fn order_status_values_are_not_contiguous() {
        // TRIGGERED jumps to 10; anything iterating 1..=n would miss it.
        assert_eq!(OrderStatus::Triggered.as_int(), 10);
        assert_eq!(OrderStatus::from_int(7), None);
        assert_eq!(OrderStatus::from_int(9), None);
        assert_eq!(OrderStatus::from_int(10), Some(OrderStatus::Triggered));
    }

    #[rstest]
    fn every_variant_round_trips_through_both_representations() {
        macro_rules! check {
            ($ty:ty) => {
                for &variant in <$ty>::all() {
                    assert_eq!(<$ty>::from_int(variant.as_int()), Some(variant));
                    assert_eq!(<$ty>::from_str_value(variant.as_str()), Some(variant));
                }
            };
        }

        check!(OrderSide);
        check!(OrderType);
        check!(TimeInForce);
        check!(OrderStatus);
        check!(ExecutionType);
        check!(OrderModifier);
        check!(MarginMode);
        check!(PositionSide);
        check!(StopType);
        check!(TriggerType);
        check!(SignatureType);
    }

    #[rstest]
    fn serializes_as_integer_for_requests() {
        // Requests carry integers, and the signature commits to exactly those bytes.
        assert_eq!(serde_json::to_string(&OrderSide::Buy).unwrap(), "1");
        assert_eq!(serde_json::to_string(&OrderType::Market).unwrap(), "2");
        assert_eq!(serde_json::to_string(&TimeInForce::Ioc).unwrap(), "3");
    }

    #[rstest]
    fn deserializes_from_the_status_strings_responses_carry() {
        assert_eq!(
            serde_json::from_str::<OrderStatus>(r#""PARTIALLY_FILLED""#).unwrap(),
            OrderStatus::PartiallyFilled
        );
        assert_eq!(
            serde_json::from_str::<ExecutionType>(r#""REPLACED""#).unwrap(),
            ExecutionType::Replaced
        );
    }

    #[rstest]
    fn deserializes_from_integers_too() {
        // Responses are documented as strings, but accepting the integer form costs nothing
        // and avoids a hard failure if any endpoint sends the request-side encoding.
        assert_eq!(
            serde_json::from_str::<OrderSide>("2").unwrap(),
            OrderSide::Sell
        );
    }

    #[rstest]
    fn unknown_values_are_rejected_rather_than_defaulted() {
        // Silently mapping an unknown status onto a known one would misreport order state.
        assert!(serde_json::from_str::<OrderStatus>(r#""SETTLED""#).is_err());
        assert!(serde_json::from_str::<OrderStatus>("99").is_err());
        assert!(serde_json::from_str::<OrderSide>("null").is_err());
    }

    #[rstest]
    fn placement_support_matches_the_documented_restrictions() {
        assert!(!TimeInForce::Fok.is_supported_for_placement());
        assert!(TimeInForce::Ioc.is_supported_for_placement());

        assert!(PositionSide::Both.is_supported_for_placement());
        assert!(!PositionSide::Long.is_supported_for_placement());
        assert!(!PositionSide::Short.is_supported_for_placement());

        assert!(TriggerType::MarkPrice.is_supported_for_placement());
        assert!(!TriggerType::LastPrice.is_supported_for_placement());
        assert!(!TriggerType::IndexPrice.is_supported_for_placement());
    }

    #[rstest]
    fn permission_bits_mean_disabled_not_granted() {
        // Reading the mask as "granted" would produce a key holding exactly the powers the
        // caller intended to withhold - including withdrawal.
        let withhold_withdraw =
            DisabledPermissions::none().disabling(DisabledPermissions::WITHDRAW);

        assert!(withhold_withdraw.is_disabled(DisabledPermissions::WITHDRAW));
        assert!(!withhold_withdraw.is_disabled(DisabledPermissions::TRADE));
        assert_eq!(withhold_withdraw.as_mask(), 4);
    }

    #[rstest]
    fn empty_mask_enables_everything() {
        let all_enabled = DisabledPermissions::none();

        assert_eq!(all_enabled.as_mask(), 0);
        for permission in [
            DisabledPermissions::TRADE,
            DisabledPermissions::CANCEL,
            DisabledPermissions::WITHDRAW,
            DisabledPermissions::TRANSFER,
        ] {
            assert!(!all_enabled.is_disabled(permission));
        }
    }

    #[rstest]
    fn cancel_only_key_can_wind_down_but_not_open_or_move_funds() {
        let key = DisabledPermissions::cancel_only();

        assert!(!key.is_disabled(DisabledPermissions::CANCEL));
        assert!(key.is_disabled(DisabledPermissions::TRADE));
        assert!(key.is_disabled(DisabledPermissions::WITHDRAW));
        assert!(key.is_disabled(DisabledPermissions::TRANSFER));
    }
}
