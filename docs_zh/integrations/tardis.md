# Tardis

Tardis 为加密货币市场提供细粒度数据，包括逐笔订单簿快照和更新、
成交、未平仓合约、资金费率、期权摘要和清算数据，覆盖领先的加密交易所。

NautilusTrader 集成了 Tardis API、Tardis Machine WebSocket 服务器以及 Tardis CSV
格式。此适配器（Adapter）的功能包括：

- `TardisCSVDataLoader`：读取 Tardis 格式的 CSV 文件并将其转换为 Nautilus 数据，支持批量加载和内存高效的流式处理路径。
- `TardisMachineClient`：从 Tardis Machine 流式传输实时数据或历史回放（Replay）数据，并将消息转换为 Nautilus 数据。
- `TardisHttpClient`：从 Tardis HTTP API 请求金融工具（Instrument）元数据，并将其解析为 Nautilus 金融工具定义。
- `TardisDataClient`：为 Tardis Machine 数据流提供实时数据客户端。
- `TardisInstrumentProvider`：从 Tardis 元数据 API 加载金融工具定义。
- **数据管道函数**：从 Tardis Machine 回放历史数据，并写入 Nautilus Parquet 目录文件。

:::info
Nautilus 金融工具元数据调用需要 `TARDIS_API_KEY`。Tardis Machine 在请求每月免费首日之外的历史日期时
需要 `TM_API_KEY`。另请参阅[环境变量](#环境变量)。
:::

## 概述

此适配器使用 Rust 实现，并提供可选的 Python 绑定。
它不需要任何外部 Tardis 客户端库依赖。

:::info
**无需**为 `tardis` 执行额外的安装步骤。
适配器的核心组件被编译为静态库，并在构建过程中链接。
:::

## Tardis 文档

Tardis 提供了广泛的用户[文档](https://docs.tardis.dev/)。
我们建议结合本 NautilusTrader 集成指南一并参阅 Tardis 文档。

## 支持的格式

Tardis 提供*标准化*（Normalized）的市场数据，即一种在所有支持的交易所之间保持一致的统一格式。
这种标准化让单个解析器即可处理来自任何 [Tardis 支持的交易所](#交易场所)的数据。
NautilusTrader 在此适配器中不支持交易所原生的 Tardis 市场数据格式。

以下 Tardis Machine 标准化格式受 NautilusTrader 支持。字段结构请参阅官方
[Tardis 数据类型参考](https://docs.tardis.dev/tardis-machine/data-types)。

| Tardis 格式           | Nautilus 数据类型                                                  |
| :------------------ | :------------------------------------------------------------- |
| `book_change`       | `OrderBookDelta`                                               |
| `book_snapshot_*`   | `OrderBookDepth10` 或 `OrderBookDeltas`                         |
| `quote`             | `QuoteTick`                                                    |
| `quote_10s`         | `QuoteTick`                                                    |
| `trade`             | `Trade`                                                        |
| `trade_bar_*`       | `Bar`                                                          |
| `instrument`        | `CurrencyPair`、`CryptoFuture`、`CryptoPerpetual`、`CryptoOption` |
| `derivative_ticker` | `FundingRateUpdate`                                            |
| `option_summary`    | `OptionGreeks`；可选地从 BBO 字段生成 `QuoteTick`                       |
| `disconnect`        | *不适用*                                                          |

**说明：**

- Tardis 将 `quote` 记录为 `book_snapshot_1_0ms` 的别名。
- Tardis 将 `quote_10s` 记录为 `book_snapshot_1_10s` 的别名。
- `quote`、`quote_10s` 和单层快照均被解析为 `QuoteTick`。
- 当相关值发生变化时，Rust 数据客户端还会从 `derivative_ticker` 消息中发出标记价格和指数价格更新。
- Tardis `option_summary` 消息包含最优买价/卖价（best bid/offer）字段。Nautilus 始终将此数据流映射为
  `OptionGreeks`；将 `extract_bbo_as_quotes` 设为 `true` 可同时从这些 BBO 字段发出 `QuoteTick`。

:::info
另请参阅 Tardis 的 [Tardis Machine 快速入门](https://docs.tardis.dev/tardis-machine/quickstart)。
:::

## K 线

适配器将 Tardis 交易 K 线间隔和后缀转换为 Nautilus `BarType`。
包括以下内容：

| Tardis 后缀 | 含义      | Nautilus K 线聚合方式 |
| :-------- | :------ | :--------------- |
| `ms`      | 毫秒      | `MILLISECOND`    |
| `s`       | 秒       | `SECOND`         |
| `m`       | 分钟      | `MINUTE`         |
| `ticks`   | tick 数量 | `TICK`           |
| `vol`     | 成交量大小   | `VOLUME`         |

## 符号体系和标准化

Tardis 集成通过一致地标准化符号，确保与 NautilusTrader 的加密交易所适配器兼容。
通常，NautilusTrader 使用 Tardis 提供的交易所原生命名约定。但对于某些交易所，
原始符号会被调整以符合 Nautilus 符号标准化，如下所述：

### 通用规则

- 所有符号转换为大写。
- 对于某些交易所，市场类型后缀以连字符附加。
- 原始交易所符号保留在 Nautilus 金融工具定义的 `raw_symbol` 字段中。

### 交易所特定标准化

- **Binance**：Nautilus 为所有永续合约符号附加 `-PERP` 后缀。
- **Bybit**：Nautilus 使用产品类别后缀，包括 `-SPOT`、`-LINEAR`、`-INVERSE` 和 `-OPTION`。
- **dYdX**：Nautilus 为所有永续合约符号附加 `-PERP` 后缀。
- **Gate.io**：Nautilus 为所有永续合约符号附加 `-PERP` 后缀。

各交易所的详细符号体系文档：

- [Binance 符号体系](./binance.md#symbology)
- [Bybit 符号体系](./bybit.md#symbology)
- [dYdX 符号体系](./dydx.md#symbology)

## 交易场所

Tardis 上的一些交易所被划分为多个交易场所（Venue）。
下表列出了 Nautilus 交易场所与相应 Tardis 交易所之间的映射：

| Nautilus 交易场所      | Tardis 交易所                                                      |
| :----------------- | :-------------------------------------------------------------- |
| `ASCENDEX`         | `ascendex`                                                      |
| `BINANCE`          | `binance`、`binance-dex`、`binance-futures`、`binance-options`     |
| `BINANCE_DELIVERY` | `binance-delivery`（*币本位合约*）                                     |
| `BINANCE_US`       | `binance-us`                                                    |
| `BITFINEX`         | `bitfinex`、`bitfinex-derivatives`                               |
| `BITFLYER`         | `bitflyer`                                                      |
| `BITGET`           | `bitget`、`bitget-futures`                                       |
| `BITMEX`           | `bitmex`                                                        |
| `BITNOMIAL`        | `bitnomial`                                                     |
| `BITSTAMP`         | `bitstamp`                                                      |
| `BLOCKCHAIN_COM`   | `blockchain-com`                                                |
| `BYBIT`            | `bybit`、`bybit-options`、`bybit-spot`                            |
| `COINBASE`         | `coinbase`                                                      |
| `COINBASE_INTX`    | `coinbase-international`                                        |
| `COINFLEX`         | `coinflex`（*用于历史研究*）                                            |
| `CRYPTO_COM`       | `crypto-com`                                                    |
| `CRYPTOFACILITIES` | `cryptofacilities`                                              |
| `DELTA`            | `delta`                                                         |
| `DERIBIT`          | `deribit`                                                       |
| `DYDX`             | `dydx`                                                          |
| `DYDX_V4`          | `dydx-v4`                                                       |
| `FTX`              | `ftx`、`ftx-us`（*历史研究*）                                          |
| `GATE_IO`          | `gate-io`、`gate-io-futures`                                     |
| `GEMINI`           | `gemini`                                                        |
| `HITBTC`           | `hitbtc`                                                        |
| `HUOBI`            | `huobi`、`huobi-dm`、`huobi-dm-linear-swap`、`huobi-dm-options`    |
| `HUOBI_DELIVERY`   | `huobi-dm-swap`                                                 |
| `HYPERLIQUID`      | `hyperliquid`                                                   |
| `KRAKEN`           | `kraken`                                                        |
| `KUCOIN`           | `kucoin`、`kucoin-futures`                                       |
| `MANGO`            | `mango`                                                         |
| `OKCOIN`           | `okcoin`                                                        |
| `OKEX`             | `okex`、`okex-futures`、`okex-options`、`okex-spreads`、`okex-swap` |
| `PHEMEX`           | `phemex`                                                        |
| `POLONIEX`         | `poloniex`                                                      |
| `SERUM`            | `serum`（*历史研究*）                                                 |
| `STAR_ATLAS`       | `star-atlas`                                                    |
| `UPBIT`            | `upbit`                                                         |
| `WOO_X`            | `woo-x`                                                         |

Tardis 还暴露了一些遗留的 Binance 交易所，例如 `binance-european-options` 和
`binance-jersey`。

## 环境变量

以下环境变量由 Tardis 和 NautilusTrader 使用。

- `TM_API_KEY`：Tardis Machine 的 API 密钥。
- `TARDIS_API_KEY`：NautilusTrader Tardis 客户端的 API 密钥。
- `TARDIS_MACHINE_WS_URL`（可选）：`TardisMachineClient` 的 WebSocket URL。
- `TARDIS_BASE_URL`（可选）：NautilusTrader 中 `TardisHttpClient` 的基础 URL。
- `NAUTILUS_PATH`（可选）：包含 `catalog/` 子目录的父目录，用于回放输出。

Tardis 金融工具元数据 API 需要 bearer-token 授权，且仅对处于活跃状态的 pro 和 business
Tardis 订阅开放。

## 运行 Tardis Machine 历史回放

[Tardis Machine Server](https://docs.tardis.dev/tardis-machine/quickstart) 是一个可本地运行的服务器，
具有内置数据缓存功能。它通过 HTTP 和 WebSocket API 提供逐笔（tick 级）历史数据和整合的实时加密货币市场数据。

你可以使用 Python 或 Rust 执行完整的 Tardis Machine WebSocket 历史数据回放，并将结果输出为
Nautilus Parquet 格式。由于该函数使用 Rust 实现，无论从 Python 还是 Rust 运行，性能都是一致的。

端到端的 `run_tardis_machine_replay` 数据管道函数使用指定的[配置](#配置)执行以下步骤：

- 连接到 Tardis Machine 服务器。
- 从 Tardis 金融工具元数据 API 请求并解析所有必要的金融工具定义。
- 从 Tardis Machine 流式传输指定时间范围内所有请求的金融工具和数据类型。
- 对于每个金融工具、数据类型和日期（UTC），生成一个目录兼容格式的 `.parquet` 文件。
- 从 Tardis Machine 服务器断开连接，并终止程序。

**文件命名约定**

文件按天、按金融工具写入，使用 ISO 8601 时间戳范围：

- **格式**：`{start_timestamp}_{end_timestamp}.parquet`
- **示例**：`2023-10-01T00-00-00-000000000Z_2023-10-01T23-59-59-999999999Z.parquet`
- **结构**：`data/{data_type}/{instrument_id}/{filename}`

此格式与 Nautilus 数据目录的查询、整合和管理操作兼容。

:::note
你可以在不使用 Tardis Machine API 密钥的情况下请求每月第一天的数据。其他日期需要 `TM_API_KEY`。
:::

此过程已针对直接输出到 Nautilus Parquet 数据目录进行了优化。
将 `NAUTILUS_PATH` 设置为包含 `catalog/` 子目录的父目录。Parquet
文件将按数据类型和金融工具分目录写入 `<NAUTILUS_PATH>/catalog/data/` 下。

如果未指定 `output_path` 且 `NAUTILUS_PATH` 未设置，输出将默认使用当前工作目录。

### 步骤

首先，确保 `tardis-machine` Docker 容器正在运行。使用以下命令：

```bash
docker run -p 8000:8000 -p 8001:8001 -e "TM_API_KEY=YOUR_API_KEY" -d tardisdev/tardis-machine
```

此命令在没有持久本地缓存的情况下启动 `tardis-machine` 服务器，这可能影响性能。
为了获得更好的回放性能，请使用持久卷运行它。

### 配置

接下来，确保你有一个可用的配置 JSON 文件。

**配置 JSON 字段**

- `tardis_ws_url`（`str | null`）：Tardis Machine WebSocket URL。默认为
  `TARDIS_MACHINE_WS_URL`。
- `normalize_symbols`（`bool | null`）：应用 Nautilus 符号标准化。默认为 `true`。
- `output_path`（`str | null`）：Parquet 数据的输出目录。默认为 `NAUTILUS_PATH`，
  然后是当前工作目录。
- `book_snapshot_output`（`"deltas" | "depth10" | null`）：快照的输出格式。默认为
  `"deltas"`。
- `extract_bbo_as_quotes`（`bool | null`）：同时从 Tardis Machine `option_summary` 消息中的
  最优买价/卖价字段写入 `QuoteTick` 数据。默认为 `false`。
- `compression`（`"zstd" | "snappy" | "uncompressed" | null`）：Parquet 压缩编解码器。
  默认为 `"zstd"` 级别 3。
- `proxy_url`（`str | null`）：Tardis HTTP 请求的代理 URL。默认不使用代理。
- `options`（`JSON[]`）：必需的回放请求选项对象。

示例配置文件可在 `crates/adapters/tardis/bin/example_config.json` 找到：

```json
{
  "tardis_ws_url": "ws://localhost:8001",
  "output_path": null,
  "options": [
    {
      "exchange": "bitmex",
      "symbols": [
        "xbtusd",
        "ethusd"
      ],
      "data_types": [
        "trade"
      ],
      "from": "2019-10-01",
      "to": "2019-10-02"
    }
  ]
}
```

### 订单簿快照输出

`book_snapshot_output` 配置选项控制 Tardis `book_snapshot_*` 消息的转换和存储方式。

| 值         | Nautilus 类型        | 输出目录                 | 描述                |
| :-------- | :----------------- | :------------------- | :---------------- |
| `deltas`  | `OrderBookDeltas`  | `order_book_deltas/` | 价格层级更新。           |
| `depth10` | `OrderBookDepth10` | `order_book_depths/` | 包含最多 10 个价格层级的快照。 |

**何时使用每种格式：**

- **`deltas`（默认）**：当你需要重建订单簿状态或将快照与 `book_change` 数据组合使用时使用。
  每个价格层级成为一条单独的增量记录。
- **`depth10`**：当策略需要定期的深度快照时使用。每个快照是单条记录，超过 10 个层级的快照
  只保留前 10 个。

**避免文件覆盖：**

当为同一金融工具和日期范围同时下载 `book_snapshot_*` 和 `book_change` 数据时，`depth10`
将快照写入 `order_book_depths/`，从而避免覆盖 `order_book_deltas/`。

使用显式格式的配置示例：

```json
{
  "tardis_ws_url": "ws://localhost:8001",
  "book_snapshot_output": "depth10",
  "options": [
    {
      "exchange": "binance-futures",
      "symbols": ["btcusdt"],
      "data_types": ["book_snapshot_5_100ms", "book_change"],
      "from": "2024-01-01",
      "to": "2024-01-02"
    }
  ]
}
```

### 期权摘要 BBO 提取

当请求 Tardis Machine `option_summary` 数据且回测同时需要期权 BBO 报价时，将
`extract_bbo_as_quotes` 设为 `true`。Nautilus 仍会从每条 `option_summary` 消息写入
`OptionGreeks`。当所有最优买价/卖价字段都存在且数量有效时，它还会为同一金融工具
和时间戳写入一条 `QuoteTick`。

此选项仅适用于 Tardis Machine `option_summary` 回放和流式消息。它不会改变 Tardis CSV 加载。

```json
{
  "tardis_ws_url": "ws://localhost:8001",
  "extract_bbo_as_quotes": true,
  "options": [
    {
      "exchange": "deribit",
      "symbols": ["BTC-28JUN24-70000-C"],
      "data_types": ["option_summary"],
      "from": "2024-01-01",
      "to": "2024-01-02"
    }
  ]
}
```

### Python 回放

要在 Python 中运行回放，创建类似以下的脚本：

```python
import asyncio
from pathlib import Path

from nautilus_trader.core import nautilus_pyo3


async def run():
    config_filepath = Path("YOUR_CONFIG_FILEPATH")
    await nautilus_pyo3.run_tardis_machine_replay(str(config_filepath.resolve()))


if __name__ == "__main__":
    asyncio.run(run())
```

### Rust 回放

要在 Rust 中运行回放，创建类似以下的二进制文件：

```rust
use std::path::PathBuf;

use nautilus_adapters::tardis::replay::run_tardis_machine_replay_from_config;

#[tokio::main]
async fn main() {
    nautilus_common::logging::ensure_logging_initialized();

    let config_filepath = PathBuf::from("YOUR_CONFIG_FILEPATH");
    run_tardis_machine_replay_from_config(&config_filepath).await;
}
```

日志默认为 INFO 级别。要启用调试日志，请导出以下环境变量：

```bash
export NAUTILUS_LOG=debug
```

可在 `crates/adapters/tardis/bin/example_replay.rs` 找到一个可运行的示例二进制文件。

也可以使用 cargo 运行：

```bash
cargo run --bin tardis-replay <path_to_your_config>
```

### 期权链回测目录

期权链回测在 Tardis 回放将数据写入 Nautilus 目录之后才会开始。回测加载器在运行期间
不会请求缺失的 Tardis 数据，因此目录中必须包含：

- 来自 Tardis 金融工具元数据 API 的期权金融工具。
- 来自单层期权订单簿快照、报价数据或 `option_summary` BBO 提取的 `QuoteTick` 数据。
- 来自 Tardis `option_summary` 消息的 `OptionGreeks` 数据。

在 `BacktestDataConfig` 列表中为相同的期权金融工具 ID 同时使用 `QuoteTick` 和
`OptionGreeks`。期权链管理器会将回放的 BBO 和 Greeks 聚合为 `OptionChainSlice`
快照。使用 `snapshot_interval_ms=None` 进行原始发布，或设置以毫秒为单位的间隔来发布
精简后的快照。

策略可以通过价值状态（moneyness）以 ATM 相对或 ATM 百分比的行权价范围来选择合约，
通过 `StrikeRange.delta(target, tolerance)` 按 delta 选择，或通过 `StrikeRange.fixed([...])`
按固定行权价选择。回测中的期权订单撮合是报价驱动的：可成交订单作为 taker 与对手方 BBO
成交，而被动限价单可在后续 BBO 更新穿透该限价时作为 maker 成交。

在模拟交易场所上使用结构化费用模型（例如 `CappedOptionFeeModel` 或
`TieredNotionalOptionFeeModel`）显式配置期权费用。不存在从 Tardis 交易所到费用模型的
自动映射。

### 期权链 CSV 目录转换

对于来自可下载 Tardis CSV 文件的历史期权链，使用
`TardisCSVDataLoader.convert_options_chain_csv(...)` 将 `options_chain` 行转换为
Nautilus 目录数据。此路径不会调用 Tardis Machine 或金融工具元数据 API，因此当你已经
拥有 Tardis CSV 文件，或希望从已下载的数据进行无需 API 密钥的目录初始化时，它非常有用。

转换器为每个选中的行写入 `OptionGreeks`。在默认的 `extract_bbo_as_quotes=True` 下，
完整的最优买价/卖价行也会写入 `QuoteTick`。在期权链回测中请保持启用此选项：仅含 greeks
的目录不提供报价，因此链管理器无法为没有 BBO 数据的行权价发布已填充的 `OptionChainSlice`
快照。

金融工具派生目前支持 Deribit 期权。对于其他期权交易场所，请在转换前设置
`write_instruments=False`，并在回测前通过其他来源加载金融工具。对非 Deribit 文件保持启用该
选项可能会在数据文件已写入目录后失败。请按时间顺序传入每日的 `options_chain` CSV 路径。
`underlyings` 过滤器匹配诸如 `["BTC-"]` 这样的符号前缀。设置 `snapshot_interval_ms`
可在每个输入文件内保留每个金融工具每个间隔的最后一行，或使用 `None` 写入每个选中的行。
进行精简时，每个文件内的行必须按 `local_timestamp` 排序。

在加载器上提供显式的 `price_precision` 和 `size_precision`，以获得确定性的报价元数据。
推断的精度可能随着后续行的读取而增加，因此文件中较早写入的数据可能保留较低精度的元数据。

```python
from pathlib import Path

from nautilus_trader.adapters.tardis import TardisCSVDataLoader


loader = TardisCSVDataLoader(
    price_precision=4,
    size_precision=1,
)
loader.convert_options_chain_csv(
    filepaths=[Path("deribit_options_chain_2020-06-08.csv")],
    catalog_path=Path("catalog"),
    underlyings=["BTC-"],
    snapshot_interval_ms=60_000,
)
```

## 加载 Tardis CSV 数据

Tardis 格式的 CSV 数据可以使用 Python 或 Rust 加载。加载器从磁盘读取 CSV 文本数据并将其
解析为 Nautilus 数据。由于加载器使用 Rust 实现，无论从 Python 还是 Rust 运行，性能都保持一致。

你还可以为 `load_*` 函数和方法指定 `limit` 参数来控制加载的最大行数。

:::note
加载混合金融工具的 CSV 文件由于精度要求较为困难，不建议使用。请改用单一金融工具的 CSV 文件。

`load_options_chain`、`stream_options_chain` 和 `convert_options_chain_csv` 方法是例外：
Tardis `options_chain` 文件是混合金融工具的链文件，这些路径会按金融工具分别跟踪精度。
为了获得确定性的输出，仍建议显式指定精度。
:::

### 在 Python 中加载 CSV 数据

你可以使用 `TardisCSVDataLoader` 在 Python 中加载 Tardis 格式的 CSV 数据。
加载数据时，你可以选择性地指定金融工具 ID、价格精度和数量精度。提供金融工具 ID 可以提高加载性能。
价格和数量精度在省略时会从 CSV 中推断，但建议显式指定值以获得确定性输出，尤其是处理大文件时。

创建类似以下的脚本来加载数据：

```python
from pathlib import Path

from nautilus_trader.adapters.tardis import TardisCSVDataLoader
from nautilus_trader.model import InstrumentId


instrument_id = InstrumentId.from_str("BTC-PERPETUAL.DERIBIT")
loader = TardisCSVDataLoader(
    price_precision=1,
    size_precision=0,
    instrument_id=instrument_id,
)

filepath = Path("YOUR_CSV_DATA_PATH")
limit = None

deltas = loader.load_deltas(filepath, limit=limit)
```

### 在 Rust 中加载 CSV 数据

你可以使用 `crates/adapters/tardis/src/csv/mod.rs` 中的加载函数在 Rust 中加载 Tardis 格式的
CSV 数据。加载数据时，你可以选择性地指定金融工具 ID、价格精度和数量精度。提供金融工具 ID
可以提高加载性能。价格和数量精度在省略时会从 CSV 中推断，但建议显式指定值以获得确定性输出。

完整示例请参阅 `crates/adapters/tardis/bin/example_csv.rs`。

使用类似以下的代码来加载数据：

```rust
use std::path::Path;

use nautilus_adapters::tardis;
use nautilus_model::identifiers::InstrumentId;

#[tokio::main]
async fn main() {
    // 可选地指定精度和 CSV 文件路径
    let price_precision = Some(1);
    let size_precision = Some(0);
    let filepath = Path::new("YOUR_CSV_DATA_PATH");

    // 可选地指定金融工具 ID 和/或限制数
    let instrument_id = InstrumentId::from("BTC-PERPETUAL.DERIBIT");
    let limit = None;

    // 根据你的工作流考虑传播任何解析错误
    let _deltas = tardis::csv::load_deltas(
        filepath,
        price_precision,
        size_precision,
        Some(instrument_id),
        limit,
    )
    .unwrap();
}
```

## 流式 Tardis CSV 数据

对于大型 CSV 文件的内存高效处理，Tardis 集成可以按可配置的块（chunk）加载和处理数据，
而不是一次性将整个文件加载到内存中。这对于处理数 GB 的 CSV 文件而不耗尽系统内存非常有用。

Python 流式功能适用于以下高数据量的 CSV 类型：

- 订单簿增量（`stream_deltas`）。
- 报价 tick（`stream_quotes`）。
- 成交 tick（`stream_trades`）。
- 订单簿深度快照（`stream_depth10`）。
- 期权链行（`stream_options_chain`）。

Rust 也为这些 CSV 类型暴露了流式函数，此外还包括批量增量和资金费率。

### 在 Python 中流式 CSV 数据

`TardisCSVDataLoader` 提供以迭代器形式产生数据块的流式方法。每个方法接受一个 `chunk_size`
参数，控制每个块读取多少条记录：

```python
from nautilus_trader.adapters.tardis import TardisCSVDataLoader
from nautilus_trader.model import InstrumentId

instrument_id = InstrumentId.from_str("BTC-PERPETUAL.DERIBIT")
loader = TardisCSVDataLoader(
    price_precision=1,
    size_precision=0,
    instrument_id=instrument_id,
)

filepath = Path("large_trades_file.csv")
chunk_size = 100_000  # 每块处理 100,000 条记录（默认值）

# 按块流式处理成交 tick
for chunk in loader.stream_trades(filepath, chunk_size):
    print(f"Processing chunk with {len(chunk)} trades")
    # 处理每个块——只有当前块在内存中
    for trade in chunk:
        # 你的处理逻辑
        pass
```

### 流式订单簿数据

对于订单簿数据，增量和深度快照均支持流式处理：

```python
# 流式订单簿增量
for chunk in loader.stream_deltas(filepath):
    print(f"Processing {len(chunk)} deltas")
    # 处理增量块

# 流式 depth10 快照（指定层数：5 或 25）
for chunk in loader.stream_depth10(filepath, levels=5):
    print(f"Processing {len(chunk)} depth snapshots")
    # 处理深度块
```

### 流式报价数据

报价数据可以类似地流式处理：

```python
# 流式报价 tick
for chunk in loader.stream_quotes(filepath):
    print(f"Processing {len(chunk)} quotes")
    # 处理报价块
```

### 内存效率优势

流式处理方法提供了显著的内存效率优势：

- **可控的内存使用**：一次只加载一个块到内存中。
- **可扩展的处理**：可以处理大于可用 RAM 的文件。
- **可配置的块大小**：根据系统内存和性能需求调整 `chunk_size`（默认 100,000）。

:::warning
当使用精度推断进行流式处理时，推断的精度可能与批量加载整个文件不同。精度推断在块边界内工作，
不同的块可能包含具有不同精度要求的值。为了确定性的精度行为，请提供显式的 `price_precision`
和 `size_precision` 参数。
:::

### 在 Rust 中流式 CSV 数据

底层流式功能使用 Rust 实现，可以直接使用：

```rust
use std::path::Path;

use nautilus_adapters::tardis::csv::stream_trades;
use nautilus_model::identifiers::InstrumentId;

#[tokio::main]
async fn main() {
    let filepath = Path::new("large_trades_file.csv");
    let chunk_size = 100_000;
    let price_precision = Some(1);
    let size_precision = Some(0);
    let instrument_id = Some(InstrumentId::from("BTC-PERPETUAL.DERIBIT"));

    // 按块流式处理成交
    let stream = stream_trades(
        filepath,
        chunk_size,
        price_precision,
        size_precision,
        instrument_id,
    ).unwrap();

    for chunk_result in stream {
        match chunk_result {
            Ok(chunk) => {
                println!("Processing chunk with {} trades", chunk.len());
                // 处理块
            }
            Err(e) => {
                eprintln!("Error processing chunk: {}", e);
                break;
            }
        }
    }
}
```

## 请求金融工具定义

你可以使用 `TardisHttpClient` 在 Python 和 Rust 中请求金融工具定义。
此客户端与 [Tardis 金融工具元数据 API](https://docs.tardis.dev/api/instruments-metadata-api)
交互，请求并将金融工具元数据解析为 Nautilus 金融工具。

`TardisHttpClient` 构造函数接受可选参数 `api_key`、`base_url`、`timeout_secs`、
`normalize_symbols` 和 `proxy_url`。

客户端提供方法来获取特定的 `instrument`，或特定交易所上所有可用的 `instruments`。
使用 Tardis 的小写连字符（lower-kebab）交易所 ID，例如 `binance-futures`。

:::note
需要一个具有金融工具元数据 API 访问权限的 `TARDIS_API_KEY`。
:::

### 在 Python 中请求金融工具

要在 Python 中请求金融工具定义，创建类似以下的脚本：

```python
import asyncio

from nautilus_trader.core import nautilus_pyo3


async def run():
    http_client = nautilus_pyo3.TardisHttpClient()

    instrument = await http_client.instrument("bitmex", "xbtusd")
    print(f"Received: {instrument}")

    instruments = await http_client.instruments("bitmex")
    print(f"Received: {len(instruments)} instruments")


if __name__ == "__main__":
    asyncio.run(run())
```

### 在 Rust 中请求金融工具

要在 Rust 中请求金融工具定义，使用类似以下的代码。
完整示例请参阅 `crates/adapters/tardis/bin/example_http.rs`。

```rust
use nautilus_tardis::{
    enums::TardisExchange,
    http::client::TardisHttpClient,
};

#[tokio::main]
async fn main() {
    nautilus_common::logging::ensure_logging_initialized();

    let client = TardisHttpClient::new(None, None, None, true, None).unwrap();

    // Tardis 金融工具定义
    let resp = client
        .instruments_info(TardisExchange::Bitmex, Some("XBTUSD"), None)
        .await;
    println!("Received: {resp:?}");

    // Nautilus 金融工具定义
    let resp = client
        .instruments(
            TardisExchange::Bitmex,
            Some("XBTUSD"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
    println!("Received: {resp:?}");
}
```

## 金融工具提供者

`TardisInstrumentProvider` 通过 HTTP 金融工具元数据 API 从 Tardis 请求和解析金融工具定义。
由于有多个 [Tardis 支持的交易所](#交易场所)，在加载所有金融工具时，
你必须使用 `InstrumentProviderConfig` 筛选所需的交易场所：

```python
from nautilus_trader.config import InstrumentProviderConfig

# 参见支持的交易场所 https://nautilustrader.io/docs/nightly/integrations/tardis#venues
venues = {"BINANCE", "BYBIT"}
filters = {"venues": frozenset(venues)}
instrument_provider_config = InstrumentProviderConfig(load_all=True, filters=filters)
```

你也可以按常规方式加载特定的金融工具定义：

```python
from nautilus_trader.config import InstrumentProviderConfig

instrument_ids = [
    InstrumentId.from_str("BTCUSDT-PERP.BINANCE"),  # 使用 'binance-futures' 交易所
    InstrumentId.from_str("BTCUSDT.BINANCE"),  # 使用 'binance' 交易所
]
instrument_provider_config = InstrumentProviderConfig(load_ids=instrument_ids)
```

### 期权交易所过滤

金融工具提供者在未提供 `instrument_type` 过滤器或该过滤器不包含 `"option"` 时，会过滤掉
期权专用交易所，例如 `binance-options`、`binance-european-options`、`bybit-options`、
`okex-options` 和 `huobi-dm-options`。

要显式加载期权金融工具，请在 `instrument_type` 过滤器中包含 `"option"`：

```python
from nautilus_trader.config import InstrumentProviderConfig

venues = {"BINANCE", "BYBIT"}
filters = {
    "venues": frozenset(venues),
    "instrument_type": {"option"},  # 显式请求期权
}
instrument_provider_config = InstrumentProviderConfig(load_all=True, filters=filters)
```

此过滤机制在不需要期权交易所时避免不必要的 API 调用。

:::note
所有订阅都需要金融工具在缓存中可用。
为简单起见，建议为你打算订阅的交易场所加载所有金融工具。
:::

## 实时数据客户端

`TardisDataClient` 将 Tardis Machine 与运行中的 NautilusTrader 系统集成。
Python 实时数据客户端将标准订阅转换为 Tardis Machine 数据流，支持以下类型：

- `OrderBookDelta`（来自 Tardis 的 L2 粒度，包括变更或全深度快照）
- `QuoteTick`
- `TradeTick`
- `Bar`（具有 [Tardis 支持的 K 线聚合](#k-线)的交易 K 线）
- `FundingRateUpdate`（来自 derivative_ticker 消息）

当 `book_snapshot_output` 为 `depth10` 时，配置的 Tardis Machine 回放/流式选项还可以发出
`OrderBookDepth10`。Tardis Machine 回放路径和目录写入器支持来自 `option_summary` 的
`OptionGreeks`。设置 `extract_bbo_as_quotes` 可同时从这些 `option_summary` 消息中的
最优买价/卖价字段发出 `QuoteTick`。

### 数据 WebSocket

主 `TardisMachineClient` 数据 WebSocket 管理初始连接阶段期间接收的所有流订阅，
直到 `ws_connection_delay_secs` 指定的持续时间。对于此期间之后进行的任何额外订阅，
将创建一个新的 `TardisMachineClient`。这让主 WebSocket 能在单个流中处理许多启动时的订阅。

当使用 `ws_connection_delay_secs` 设置初始订阅延迟时，从这些流中取消订阅不会从 Tardis Machine
流中移除该订阅，因为 Tardis 不支持选择性取消订阅。该组件仍会从消息总线发布中取消订阅。

在任何初始延迟之后进行的所有订阅都会正常运行，在请求时完全从 Tardis Machine 流中取消订阅。

:::tip
如果你预计会频繁订阅和取消订阅数据，请将 `ws_connection_delay_secs` 设置为零。这将为每个
初始订阅创建一个新客户端，允许它们在取消订阅时单独关闭。
:::

## 成交 ID 派生

成交 tick 使用来自 Tardis 消息或 CSV 行中由交易场所提供的成交 ID 作为 `TradeId`。
当交易场所省略成交 ID（在某些交易所上为空字符串或 null）时，WebSocket 解析器和 CSV 解析器
都会回退到对符号、时间戳、价格、数量和方向进行确定性的 FNV-1a 哈希。
同一交易场所事件在多次回放中产生相同的成交 ID，从而保持下游去重的完整性。

## 限制和注意事项

目前已知以下限制和注意事项：

- `TardisDataClient` 不支持历史报价和成交请求。历史外部 `Bar` 请求使用 Tardis Machine 回放，
  并需要基于日期的回放窗口。对于目录工作流，建议使用 `run_tardis_machine_replay`。

## 贡献

:::info
如需了解更多功能或为 Tardis 适配器做出贡献，请参阅我们的[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
