//! Market data.
//!
//! Conversion from the venue's wire types into Nautilus data lives in [`parse`], separated
//! from [`client`] so it can be exercised without a socket.

pub mod book;
pub mod client;
pub mod history;
pub mod parse;
pub mod tickers;
pub mod trades;

pub use book::{RpcOrderBook, fetch_order_book, parse_order_book};
pub use client::SodexDataClient;
pub use history::{
    BarRequest, HistoryError, RpcKline, drop_forming_tail, fetch_bars, finalize_bars, max_limit,
};
pub use parse::{
    BarMappingError, bar_type_for, interval_to_spec, map_aggressor_side, parse_bar,
    parse_completed_bar, parse_quote, parse_trade, spec_to_interval,
};
pub use tickers::{
    RpcTicker, TickerError, fetch_tickers, parse_funding_rate, parse_index_price, parse_mark_price,
    ticker_for,
};
