//! Order execution.
//!
//! Translation between Nautilus orders and venue requests lives in [`parse`], separated from
//! [`client`] so it can be exercised without a connection.

pub mod client;
pub mod parse;
pub mod reports;

pub use client::SodexExecutionClient;
pub use parse::{
    OrderConversionError, OrderSpec, is_fill, map_client_order_id, map_order_status,
    map_order_type, map_side, map_time_in_force, to_perps_order, to_spot_order,
};
pub use reports::{ReportError, fill_report, order_status_report};
