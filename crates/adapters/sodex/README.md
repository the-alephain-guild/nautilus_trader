# nautilus-sodex

[![build](https://github.com/nautechsystems/nautilus_trader/actions/workflows/build.yml/badge.svg?branch=master)](https://github.com/nautechsystems/nautilus_trader/actions/workflows/build.yml)
[![Documentation](https://img.shields.io/docsrs/nautilus-sodex)](https://docs.rs/nautilus-sodex/latest/nautilus_sodex/)
[![crates.io version](https://img.shields.io/crates/v/nautilus-sodex.svg)](https://crates.io/crates/nautilus-sodex)
![license](https://img.shields.io/github/license/nautechsystems/nautilus_trader?color=blue)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?logo=discord&logoColor=white)](https://discord.gg/NautilusTrader)

[NautilusTrader](https://nautilustrader.io) integration adapter for the SoDEX exchange.

SoDEX is an on-chain orderbook DEX on ValueChain. Orders are authenticated with an offline
EIP-712 signature over plain REST rather than by broadcasting transactions, so the integration
behaves more like a centralized venue than an AMM.

## Two venues, not one

Spot and perps are modeled as separate venues, `SODEX_SPOT` and `SODEX_PERPS`, because they
differ in ways no parameter papers over: the signing domain, the batch endpoint, the action names
hashed into signatures, the API key set, the order item shape, the balances, and the reference
price used for limit bounds. A client binds to one engine through its configuration's `market`,
and instrument ids must carry the matching venue.

## Credentials

Market data needs none; the venue serves it unsigned. Execution credentials resolve from the
configuration or, preferably, from `SODEX_ACCOUNT_ID`, `SODEX_API_KEY_NAME` and
`SODEX_API_PRIVATE_KEY`.

The key used at runtime is a registered API key, never the master wallet. The master wallet owns
the account and can authorize withdrawals; it signs only `addAPIKey`, `revokeAPIKey` and
`approveBuilderFee`, and belongs offline.

## Features

- Instrument listing with a periodic reload, published to the engine's cache.
- Historical klines, with the still-forming final bar removed.
- `candle`, `trade` and `ticker` streams. The venue publishes no order book channel.
- Limit and market order submission and cancellation, signed with EIP-712.
- Account state by currency, order status reports, and commissions at the venue's own
  maker/taker rates, so the engine's inferred fills carry real fees.
- Client factories, configurations and Python bindings for both engines.

## Feature flags

This crate provides feature flags to control source code inclusion during compilation:

- `extension-module`: Builds as a Python extension module.
- `high-precision` (default): Enables
  [high-precision mode](https://nautilustrader.io/docs/nightly/getting_started/installation/#precision-mode)
  to use 128-bit value types. Default here rather than opt-in because the venue settles on-chain
  and quotes token amounts at 18 decimals.
- `python`: Enables Python bindings from [PyO3](https://pyo3.rs).

## Documentation

See [the docs](https://docs.rs/nautilus-sodex) for more detailed usage. The crate-level
documentation also records the venue contracts that only live observation established, several of
which contradict or are absent from the venue's own documentation.

## License

The source code for NautilusTrader is available on GitHub under the [GNU Lesser General Public License v3.0](https://www.gnu.org/licenses/lgpl-3.0.en.html).

---

NautilusTrader™ is developed and maintained by Nautech Systems, a technology
company specializing in the development of high-performance trading systems.
For more information, visit <https://nautilustrader.io>.

Use of this software is subject to the [Disclaimer](https://nautilustrader.io/legal/disclaimer/).

<img src="https://github.com/nautechsystems/nautilus_trader/raw/develop/assets/nautilus-logo-white.png" alt="logo" width="300" height="auto"/>

© 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
