# Polymarket

Polymarket 成立于 2020 年，是一个去中心化预测市场（Prediction Market）平台，
允许交易者通过买卖结果代币（outcome token）来对事件结果进行投机。

NautilusTrader 通过 Polymarket 的中央限价订单簿（Central Limit Order Book, CLOB）API 提供数据和执行的交易场所集成（Integration）。

目前代码仓库通过公开包路径 `nautilus_trader.adapters.polymarket` 暴露了两套 Polymarket 实现：

- Python 适配器，使用
  [官方 Python CLOB V2 客户端库](https://github.com/Polymarket/py-clob-client-v2)。
- Rust 原生适配器层，这是 NautilusTrader 正在收敛的方向。

:::warning
两套实现高度重叠，但它们在每个方面的行为并不完全一致。
本指南会在差异有意义之处明确指出当前的区别。
:::

NautilusTrader 支持多种 Polymarket 签名类型用于订单签名，
这为不同的钱包配置提供了灵活性，同时由 NautilusTrader 负责处理签名和订单准备。

## 安装

安装带有 Polymarket 支持的 NautilusTrader：

```bash
uv pip install "nautilus_trader[polymarket]"
```

从源码构建并包含所有扩展（包括 Polymarket）：

```bash
uv sync --all-extras
```

## 示例

你可以在[这里](https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/polymarket/)找到实时示例脚本。

## 二元期权

[二元期权](https://en.wikipedia.org/wiki/Binary_option)是一种金融奇异期权合约，交易者对一个"是或否"命题的结果进行押注。
如果预测正确，交易者获得固定收益；否则不获得任何收益。
NautilusTrader 将 Polymarket 的结果代币表示为 `BinaryOption` 金融工具。

Polymarket 使用 **pUSD** 作为交易的抵押代币，更多信息[见下文](#pusd)。

## Polymarket 文档

Polymarket 为不同受众提供了资源：

- [Polymarket Learn](https://learn.polymarket.com/)：教育内容和指南，帮助用户了解平台及其使用方式。
- [Polymarket CLOB API](https://docs.polymarket.com/trading/orders/overview)：面向开发者的技术文档，用于与 Polymarket CLOB API 交互。

## 概述

本指南假设交易者正在同时设置实时市场数据源和交易执行。
Polymarket 集成适配器包含多个组件，可以根据使用场景一起使用或单独使用。

- `PolymarketWebSocketClient`：底层 WebSocket API 连接（基于 Rust 编写的 Nautilus `WebSocketClient` 构建）。
- `PolymarketInstrumentProvider`：`BinaryOption` 金融工具的解析和加载功能。
- `PolymarketDataClient`：市场数据源管理器。
- `PolymarketExecutionClient`：交易执行网关。
- `PolymarketLiveDataClientFactory`：Polymarket 数据客户端工厂（由交易节点构建器使用）。
- `PolymarketLiveExecClientFactory`：Polymarket 执行客户端工厂（由交易节点构建器使用）。

:::note
大多数用户将为实时交易节点定义配置（如下所述），不需要直接使用这些底层组件。
:::

### Python 和 Rust 实现

当前文档同时覆盖 Python 适配器和 Rust 原生适配器层。
下表展示了当前影响行为的主要差异。

| 方面           | Python 适配器                                                        | Rust 适配器                                     | 备注                                      |
| ------------ | ----------------------------------------------------------------- | -------------------------------------------- | --------------------------------------- |
| 公开包路径        | `nautilus_trader.adapters.polymarket`                             | `nautilus_trader.adapters.polymarket`        | Rust 是收敛目标。                             |
| 订单签名         | 使用 `py-clob-client-v2`                                            | 原生 Rust 签名                                   | Python 签名较慢。                            |
| Post‑only 订单 | 仅支持 `GTC` 和 `GTD`                                                 | 仅支持 `GTC` 和 `GTD`                            | 两者均拒绝带市价 TIF（`IOC` 或 `FOK`）的 post‑only。 |
| 批量提交         | 对可批处理的 `SubmitOrderList` 请求使用 `POST /orders`                      | 对可批处理的 `SubmitOrderList` 请求使用 `POST /orders` | 两者均仅批处理独立的限价单，每次请求上限 15 个。              |
| 批量取消         | 使用 `DELETE /orders`                                               | 使用 `DELETE /orders`                          | 两者均与 Polymarket 官方文档保持一致。               |
| 市场取消订阅       | 发送动态 WebSocket `unsubscribe` 消息                                   | 发送动态 WebSocket `unsubscribe` 消息              | 两者均支持订阅和取消订阅。                           |
| 自动加载重试       | `auto_load_max_retries`（12）、`auto_load_retry_delay_*`（5.0/15.0 秒） | 相同的旋钮，相同的默认值                                 | 两者均以有界指数退避加抖动重试 CLOB 水化 / 索引滞后导致的未命中。   |
| 数据客户端配置      | 凭证、订阅缓冲、报价处理、provider 配置                                          | 基础 URL、超时、过滤器、新市场发现                          | 除自动加载系列外，配置项差异显著。                       |
| 执行客户端配置      | 凭证、重试、原始 WS 日志、实验性的基于交易的订单恢复                                      | 凭证、重试、账户 ID、原生超时                             | Rust 并未暴露每一个仅 Python 才有的选项。             |

## pUSD

**pUSD** 是 Polymarket 上用于交易的抵押代币。它是 Polygon 上的一个标准 ERC-20 代币，由 USDC 背书。

代理合约地址为 Polygon 上的
[0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB](https://polygonscan.com/address/0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB)。
链上直接充值会通过 [CollateralOnramp](https://docs.polymarket.com/resources/contracts)
将 Polygon USDC.e（桥接的 USDC）包装为 pUSD。
Bridge API 也可以从其他链充入受支持的资产，并在转换后记入 pUSD。

## 钱包和账户

要通过 NautilusTrader 与 Polymarket 交互，你需要一个兼容 **Polygon** 的钱包（如 MetaMask）。

### 签名类型

Polymarket 支持多种签名类型用于订单签名和验证：

| 签名类型 | 钱包类型          | 描述                                  | 使用场景                                             |
| ---- | ------------- | ----------------------------------- | ------------------------------------------------ |
| `0`  | EOA（外部拥有账户）   | 来自具有直接私钥控制的钱包的标准 EIP712 签名。         | **默认。**直接钱包连接（MetaMask、硬件钱包等）。                   |
| `1`  | 邮件/Magic 钱包代理 | 用于基于邮件账户（Magic Link）的智能合约钱包。        | 与邮件/Magic 账户关联的 Polymarket 代理。需要 `funder` 地址。    |
| `2`  | 浏览器钱包代理       | 用于浏览器钱包的修改版 Gnosis Safe（1-of-1 多签）。 | 与浏览器钱包关联的 Polymarket 代理。支持 UI 验证。需要 `funder` 地址。 |
| `3`  | 充值钱包          | 用于新 API 用户的 ERC-1271 充值钱包流程。        | 需要充值钱包 `funder`；API 凭证仍绑定到签名者。                   |

:::note
另请参阅 Polymarket 文档中的 [Proxy wallet](https://docs.polymarket.com/developers/proxy-wallet)，了解更多关于签名类型和代理钱包基础设施的详情。
:::

NautilusTrader 默认使用签名类型 0（EOA），但可以通过 `signature_type` 配置参数配置为使用任何支持的签名类型。

使用环境变量时，每个交易者实例支持单个钱包地址，
或者可以通过多个 `PolymarketExecutionClient` 实例配置多个钱包。

:::note
确保你的钱包中有足够的 **pUSD** 资金，否则在提交订单时会遇到"余额或授权额度不足（not enough balance or allowance）"的 API 错误。
:::

### 设置 Polymarket 合约的授权额度

在开始交易之前，你需要确保你的钱包已为 Polymarket 的智能合约设置了授权额度。
你可以通过运行位于 `nautilus_trader/adapters/polymarket/scripts/set_allowances.py` 的脚本来完成此操作。

此脚本改编自 @poly-rodr 创建的一个 [gist](https://gist.github.com/poly-rodr/44313920481de58d5a3f6d1f8226bd5e)。

:::note
对于每个你打算在 Polymarket 上用于交易的 EOA 钱包，你只需运行此脚本**一次**。
:::

此脚本自动化了批准 Polymarket 合约所需授权额度的过程。
它为 pUSD 抵押代币和条件代币框架（Conditional Token Framework, CTF）合约设置批准，以允许 Polymarket CLOB 交易所与你的资金交互。

运行脚本前，请确保满足以下先决条件：

- 安装 web3 Python 包：`uv pip install "web3==7.12.1"`。
- 拥有一个充有一定 POL（用于 Gas 费用）的兼容 **Polygon** 的钱包。
- 在 shell 中设置以下环境变量：
  - `POLYGON_PRIVATE_KEY`：你的兼容 **Polygon** 钱包的私钥。
  - `POLYGON_PUBLIC_KEY`：你的兼容 **Polygon** 钱包的公钥。

完成上述准备后，脚本将：

- 批准 Polymarket 抵押代币合约的最大 pUSD 额度（使用 `MAX_INT` 值）。
- 设置 CTF 合约的批准，允许其与你的账户交互以进行交易。

:::note
你也可以在脚本中调整批准额度而不使用 `MAX_INT`，
额度以 **pUSD** 的*小数单位*指定，但这尚未经过测试。
:::

确保在运行脚本之前将私钥和公钥正确存储在环境变量中。
以下是在终端会话中设置变量的示例：

```bash
export POLYGON_PRIVATE_KEY="YOUR_PRIVATE_KEY"
export POLYGON_PUBLIC_KEY="YOUR_PUBLIC_KEY"
```

使用以下命令运行脚本：

```bash
python nautilus_trader/adapters/polymarket/scripts/set_allowances.py
```

### 脚本详解

脚本执行以下操作：

- 通过 RPC URL (<https://polygon-rpc.com/>) 连接到 Polygon 网络。
- 签名并发送交易以批准 Polymarket 合约的最大 pUSD 授权额度。
- 设置 CTF 合约的批准，以代你管理条件代币。
- 对 Polymarket CLOB Exchange 和 Neg Risk adapter 等特定地址重复批准过程。

这使得 Polymarket 在执行交易时能够与你的资金交互，并确保与 CLOB 交易所的顺畅集成。

## API 密钥

要在 Polymarket 上交易，你需要生成 API 凭证。请按以下步骤操作：

1. 确保设置了以下环境变量：
   - `POLYMARKET_PK`：用于签署交易的私钥。
   - `POLYMARKET_FUNDER`：**Polygon** 网络上用于在 Polymarket 上资助交易的钱包地址（公钥）。

2. 使用以下命令运行脚本：

   ```bash
   python nautilus_trader/adapters/polymarket/scripts/create_api_key.py
   ```

脚本将生成并打印 API 凭证，你应将其保存到以下环境变量中：

- `POLYMARKET_API_KEY`
- `POLYMARKET_API_SECRET`
- `POLYMARKET_PASSPHRASE`

这些凭证可用于 Polymarket 客户端配置：

- `PolymarketDataClientConfig`
- `PolymarketExecClientConfig`

## 配置

在设置 NautilusTrader 与 Polymarket 配合使用时，正确配置必要参数（尤其是私钥）至关重要。

**关键参数**：

- `private_key`：用于签署订单的钱包私钥。其解释取决于你的 `signature_type` 配置。如果未在配置中显式提供，将自动读取 `POLYMARKET_PK` 环境变量。
- `funder`：用于资助交易的 **pUSD** 资金钱包地址。如果未提供，将读取 `POLYMARKET_FUNDER` 环境变量。
- API 凭证：你需要提供以下 API 凭证以与 Polymarket CLOB 交互：
  - `api_key`：如果未提供，将读取 `POLYMARKET_API_KEY` 环境变量。
  - `api_secret`：如果未提供，将读取 `POLYMARKET_API_SECRET` 环境变量。
  - `passphrase`：如果未提供，将读取 `POLYMARKET_PASSPHRASE` 环境变量。
  API 凭证由私钥签名者创建，用于 L2 认证。对于 `POLY_1271`，充值钱包仍是 `funder`，但它不是 L2 认证地址。
- `auto_load_missing_instruments`（默认 `True`）：控制对尚未存在于缓存中的金融工具发出的订阅和请求命令是否触发通过 Gamma API 的临时加载。禁用时，订阅未缓存的金融工具会返回错误。参见[运行时金融工具加载](#运行时金融工具加载)。
- `auto_load_debounce_ms`（默认 `100`）：将并发自动加载请求合并为单次批量 Gamma 调用的时间窗口（毫秒）。

:::tip
我们建议使用环境变量来管理你的凭证。
:::

## 订单功能

Polymarket 作为预测市场运营，与传统交易所相比，其订单类型和指令集更为有限。

### 订单类型

| 订单类型                   | 二元期权 | 备注                                     |
| ---------------------- | ---- | -------------------------------------- |
| `MARKET`               | ✓    | **买单需要报价数量（quote quantity）**，卖单需要基础数量。 |
| `LIMIT`                | ✓    |                                        |
| `STOP_MARKET`          | -    | *Polymarket 不支持*。                      |
| `STOP_LIMIT`           | -    | *Polymarket 不支持*。                      |
| `MARKET_IF_TOUCHED`    | -    | *Polymarket 不支持*。                      |
| `LIMIT_IF_TOUCHED`     | -    | *Polymarket 不支持*。                      |
| `TRAILING_STOP_MARKET` | -    | *Polymarket 不支持*。                      |

### 数量语义

Polymarket 根据订单类型*和*方向对订单数量的解释不同：

- **限价**订单将 `quantity` 解释为条件代币数量（基础单位）。
- **市价卖**单也使用基础单位数量。
- **市价买**单将 `quantity` 解释为 **pUSD** 的报价名义金额。

因此，使用基础计价数量提交的市价买单将执行远超预期的规模。

提交市价买单时，请在订单上设置 `quote_quantity=True`。Python SDK
或 Rust 适配器会在向 CLOB 提交前，将报价金额（pUSD）转换为带符号的基础单位份额数量。
Polymarket 执行客户端会拒绝基础计价的市价买单，以防止意外成交。

```python
# 使用报价数量的市价买单（花费 $10 pUSD）
order = strategy.order_factory.market(
    instrument_id=instrument_id,
    order_side=OrderSide.BUY,
    quantity=instrument.make_qty(10.0),
    time_in_force=TimeInForce.IOC,  # 映射为 Polymarket 的 FAK
    quote_quantity=True,  # 解释为 pUSD 名义金额
)
strategy.submit_order(order)
```

### 执行指令

| 指令            | 二元期权 | 备注                      |
| ------------- | ---- | ----------------------- |
| `post_only`   | ✓    | 仅支持 `GTC` 或 `GTD` 的限价单。 |
| `reduce_only` | -    | *Polymarket 不支持*。       |

### 有效期选项

Polymarket 将 `POST /order` 字段称为 `orderType`。在 NautilusTrader 中，这映射到
`TimeInForce`。有效的组合取决于 Nautilus 订单类型：

| Nautilus TIF | Polymarket `orderType` | Nautilus 订单适用范围    | 备注                                   |
| ------------ | ---------------------- | ------------------ | ------------------------------------ |
| `GTC`        | `GTC`                  | 仅 `LIMIT`          | 撤销前有效（Good‑Til‑Cancelled）；挂在订单簿上。    |
| `GTD`        | `GTD`                  | 仅 `LIMIT`          | 指定日期前有效（Good‑Til‑Date）；挂单直到到期、成交或取消。 |
| `FOK`        | `FOK`                  | `LIMIT` 或 `MARKET` | 立即全部成交否则取消整个订单。                      |
| `IOC`        | `FAK`                  | `LIMIT` 或 `MARKET` | 立即成交可用部分并取消剩余部分。                     |

:::note
Polymarket 用 `FAK`（Fill-And-Kill）表示 NautilusTrader 所称的
`IOC`（立即成交或取消，Immediate or Cancel）语义。Polymarket 文档将 `FOK` 和 `FAK` 归类为市价
订单类型，而 `GTC` 和 `GTD` 是限价订单类型。对于 Nautilus 的 `MARKET`
订单，两套适配器都只接受 `IOC` 和 `FOK`；`GTC` 和 `GTD` 仅对挂单的
`LIMIT` 订单有效。
:::

### 高级订单功能

| 功能        | 二元期权 | 备注                |
| --------- | ---- | ----------------- |
| 订单修改      | -    | 仅有取消功能。           |
| 括号/OCO 订单 | -    | *Polymarket 不支持。* |
| 冰山订单      | -    | *Polymarket 不支持。* |

### 批量操作

| 操作   | 二元期权 | 备注                                                               |
| ---- | ---- | ---------------------------------------------------------------- |
| 批量提交 | ✓    | 两套适配器均对独立的限价单批次使用 `POST /orders`（每次请求最多 15 个订单）。参见[批量提交](#批量提交)。 |
| 批量修改 | -    | *Polymarket 不支持*。                                                |
| 批量取消 | ✓    | 两套适配器均使用 `DELETE /orders`。                                       |

#### 批量提交

`SubmitOrderList` 命令被路由到 Polymarket 的 `POST /orders` 端点。该端点
每次请求最多接受 15 个订单（`BATCH_ORDER_LIMIT`）；更大的列表会被拆分为
连续的 15 个一组的分块。

- 只有 `LIMIT` 订单会被批处理。列表中的 `MARKET` 订单会被路由到
  单订单路径，该路径签署一个可成交的订单，并根据 Nautilus 的 `time_in_force`
  以 `FAK` 或 `FOK` 提交。
- `reduce_only` 订单、`quote_quantity` 订单，以及带市价 TIF
  （`IOC` 或 `FOK`）的 `post_only` 会在提交前被拒绝。
- 单个符合条件的订单会回落到 `POST /order`，从而保留单订单重试
  语义；批量路径有意禁用重试，因为该场所不暴露
  幂等键（idempotency key）。
- `BatchCancelOrders` 会一次性分派到 `DELETE /orders`。

### 提交错误处理

Polymarket 的公开文档描述了成功的
[`POST /order`](https://docs.polymarket.com/api-reference/trade/post-a-new-order) 响应，
包含 `success`、`orderID`、`status` 和 `errorMsg`，并将
[API 错误](https://docs.polymarket.com/resources/error-codes)记录为结构化的错误响应。
它没有将无状态的 `py-clob-client` 异常或传输失败记录为
场所拒绝。

适配器仅在响应证明订单未被接受时才拒绝，例如
`success=false`、有记录的订单处理错误，或其他不可重试的客户端/API
错误。传输失败、超时、模糊的重试耗尽、无状态的 `PolyApiException`、
格式错误的响应，以及服务端失败，都会保持订单为已提交状态。

对于未知结果，两套适配器都会尽可能从已签名的
EIP-712 订单推导出预期的 Polymarket 订单哈希，并将其缓存为 `VenueOrderId`。后续的 WebSocket 或对账
报告便会附着到本地的 `ClientOrderId` 上，而不会变成外部订单。

报价数量市价买单在未知路径上仍会应用已签名的报价到基础数量更新。在提交结果未知时请求的取消会被推迟，
直到预期的场所订单 ID 已知，并且成交跟踪会注册在该 ID 之下。

### 持仓管理

| 功能    | 二元期权 | 备注                              |
| ----- | ---- | ------------------------------- |
| 查询持仓  | ✓    | 来自 Polymarket Data API 的当前用户持仓。 |
| 持仓模式  | -    | 仅二元结果持仓。                        |
| 杠杆控制  | -    | 无杠杆可用。                          |
| 保证金模式 | -    | 无保证金交易。                         |

### 订单查询

| 功能      | 二元期权 | 备注        |
| ------- | ---- | --------- |
| 查询未成交订单 | ✓    | 仅活跃订单。    |
| 查询订单历史  | ✓    | 有限的历史数据。  |
| 订单状态更新  | ✓    | 实时订单状态变更。 |
| 交易历史    | ✓    | 执行和成交报告。  |

### 条件订单

| 功能     | 二元期权 | 备注                       |
| ------ | ---- | ------------------------ |
| 订单列表   | -    | 存在独立的订单批次，但不存在链接的条件依赖语义。 |
| OCO 订单 | -    | *Polymarket 不支持*。        |
| 括号订单   | -    | *Polymarket 不支持*。        |
| 条件订单   | -    | *Polymarket 不支持*。        |

### 精度限制

Polymarket 根据最小变动价位和 `orderType` 强制执行不同的精度约束。

**二元期权金融工具**通常支持最多 6 位小数的金额
（最小变动价位为 0.0001），但**市价单（`FAK` 和 `FOK`）有更严格的
精度要求**：

- **市价订单类型（`FAK` 和 `FOK`）：**
  - 卖单：maker 金额限制为 **2 位小数**。
  - taker 金额：限制为 **4 位小数**。
  - 乘积 `数量 × 价格` 不得超过 **2 位小数**。

- **挂单的限价订单类型（`GTC` 和 `GTD`）：** 基于
  市场最小变动价位的更灵活精度。

### 最小变动价位精度层级

| 最小变动价位 | 价格小数位 | 数量小数位 | 金额小数位 |
| ------ | ----- | ----- | ----- |
| 0.1    | 1     | 2     | 3     |
| 0.01   | 2     | 2     | 4     |
| 0.001  | 3     | 2     | 5     |
| 0.0001 | 4     | 2     | 6     |

:::note

- 最小变动价位精度层级在 [`py-clob-client-v2` `ROUNDING_CONFIG`](https://github.com/Polymarket/py-clob-client-v2/blob/main/py_clob_client_v2/order_builder/builder.py) 中定义。
- 市价单精度限制（数量字段 2 位小数，以及对计算出的金额的基于最小变动价位的边界）
  来自同一个 `ROUNDING_CONFIG`，并由
  `OrderBuilder.get_market_order_amounts` 在签名前强制执行。
- 最小变动价位可能在市场条件下动态变化，尤其是当市场变得单边时。

:::

### 最小变动价位变更处理

当市场的最小变动价位发生变化（`tick_size_change` WebSocket 事件）时，旧的
订单簿档位在新网格上可能无效（例如 `0.505` 适合 `0.001`
最小变动价位但不适合 `0.01` 最小变动价位）。为了将旧网格价格挡在新纪元之外，
适配器将该变更视为一次订单簿纪元（book epoch）转换：

1. 发布带有新 `price_increment` 和 `price_precision` 的更新后的 `BinaryOption`。
2. 丢弃该金融工具的本地订单簿。
3. 将该金融工具标记为等待全新快照。
4. 在快照到达之前丢弃增量的 `price_change` 订单簿增量。
5. 从快照重新播种订单簿并恢复正常处理。

成交 tick 和金融工具更新会原封不动地流过。Rust 适配器
通过从每个 `price_change` 读取 `best_bid` 和
`best_ask`，在该间隙期间持续发出 `QuoteTick` 事件。Python 适配器从
本地订单簿推导报价，因此报价订阅者会看到与增量相同的短暂间隙
（通常在亚秒级，直到场所快照到达）。

## 交易

Polymarket 上的交易可能具有以下状态：

- `MATCHED`：交易已被匹配，由运营方发送到执行器服务。执行器服务将交易作为交易事务提交到交易所合约。
- `MINED`：交易已被观察到挖入链中，尚未建立最终确定性阈值。
- `CONFIRMED`：交易已达到强概率最终确定性且成功。
- `RETRYING`：交易事务失败（回滚或重组），运营方正在重试/重新提交。
- `FAILED`：交易失败且不会重试。

一旦交易初始匹配，后续交易状态更新将通过 WebSocket 接收。
NautilusTrader 在 `OrderFilled` 事件的 `info` 字段中记录初始交易详情，
额外的交易事件以 JSON 格式存储在缓存中的自定义键下，以保留此信息。

### 交易 ID 推导

Polymarket 在 `last_trade_price` 市场数据事件上不发布交易 ID。
适配器通过 `determine_trade_id`（Rust 中为 FNV-1a，Python 中为 blake2b），
从资产 ID、方向、价格、数量和时间戳推导出一个确定性的 `TradeId`。
对于 CLOB Data API 的交易历史，适配器从一个哈希
后缀、一个资产后缀和一个按（交易，资产）维度的序列号组成 `TradeId`（格式
为 `{transactionHash[-24:]}-{asset[-4:]}-{seq:06d}`）。单个 Polygon 交易
可以结算多笔共享同一 `transactionHash` 的成交，因此较旧的
取末尾 36 个字符的形式会将这些成交折叠为单个 id，导致下游
目录静默丢弃重复项。同一场所事件在多次重放中会产生相同的
交易 ID，从而保持下游去重不受影响。

## 费用

Polymarket 使用公式 `fee = C * feeRate * p * (1 - p)`，其中 C 是
交易的份额数，p 是份额价格。费用在 p = 0.50 时达到峰值，并向
两端对称递减。只有 taker 支付费用；maker 支付为零。

| 类别              | Taker `feeRate` | Maker `feeRate` | Maker 返佣 |
| --------------- | --------------- | --------------- | -------- |
| Crypto          | 0.072           | 0               | 20%      |
| Sports          | 0.03            | 0               | 25%      |
| Finance         | 0.04            | 0               | 25%      |
| Politics        | 0.04            | 0               | 25%      |
| Economics       | 0.05            | 0               | 25%      |
| Culture         | 0.05            | 0               | 25%      |
| Weather         | 0.05            | 0               | 25%      |
| Other / General | 0.05            | 0               | 25%      |
| Mentions        | 0.04            | 0               | 25%      |
| Tech            | 0.04            | 0               | 25%      |
| Geopolitics     | 0               | 0               | -        |

费用以 USDC 计算，四舍五入到 5 位小数，并由协议在撮合时
应用。收取的最小费用为 0.00001 USDC；更小的费用会舍入为零。

:::note
有关最新费率，请参阅 Polymarket 的 [Fees](https://docs.polymarket.com/trading/fees) 文档。
:::

### 回测费用模型

对于回测，适配器自带 `PolymarketFeeModel`（一个
`nautilus_trader.backtest.models.FeeModel` 子类），它应用上述 taker
费用公式，并为被动的 maker 成交记入根据
市场类别推断的返佣。Polymarket 对 Crypto 市场支付 20% 的 maker 返佣，
对其他启用费用的类别（Sports、Finance、Politics、Economics、
Culture、Weather、Tech、Mentions、Other）支付 25%，
从每个市场的返佣池中按日分配。Geopolitics 市场免费且无返佣，模型
对它们返回零。

```python
from nautilus_trader.adapters.polymarket.fee_model import PolymarketFeeModel

# 默认：启用 maker 返佣
fee_model = PolymarketFeeModel()

# 或针对仅 taker 的策略
fee_model = PolymarketFeeModel(maker_rebates_enabled=False)
```

该模型也可以通过 `BacktestVenueConfig.fee_model` 经由
`ImportableFeeModelConfig` 和 `PolymarketFeeModelConfig` 配置。Maker 返佣份额
推断首先使用金融工具的类别标签，然后在标签缺失时回退到
文档记录的按类别费率。

## 对账

Polymarket API 返回所有**活跃**（未成交）订单，或在按
Polymarket 订单 ID（`venue_order_id`）查询时返回特定订单。Polymarket 的执行对账流程如下：

- 为 Polymarket 报告的所有具有活跃（未成交）订单的金融工具生成订单报告。
- 从 Polymarket Data API 报告的当前用户持仓生成持仓报告。
- 将这些报告与 Nautilus 执行状态进行比较。
- 生成缺失的订单，使 Nautilus 执行状态与 Polymarket 报告的持仓保持一致。

**注意**：Polymarket 不直接提供已不再活跃的订单数据。
Python 适配器暴露了一个实验性的 `generate_order_history_from_trades` 选项，
以从交易历史中填补这一空白的一部分。Rust 适配器当前未暴露相同的选项。

:::warning
一个可选的执行客户端配置 `generate_order_history_from_trades` 目前正在开发中。
目前不建议在生产环境中使用。
:::

### 从交易恢复单个订单

`/data/order/{id}` 只返回活跃订单，因此 `Filled` 或 `Canceled` 订单
会返回空响应。为避免引擎将本地 `ACCEPTED`
订单解析为 `REJECTED`（这会丢弃已在场所发生的成交），
`generate_order_status_report` 会回退到按场所订单 ID 过滤的
`/data/trades`。缓存的订单通过 `client_order_id` 解析，在仅知道场所 ID 时
回退到缓存的 `venue_order_id` 索引。
恢复以缓存的订单为键；没有缓存订单时，恢复会推迟到
引擎处理，而不会仅凭交易历史合成一个外部订单：

- 缓存订单 + 覆盖缓存数量的恢复成交（在
  `DUST_SNAP_THRESHOLD` 范围内，用于 CLOB 分位最小变动价位截断）：返回 `Filled`。
  引擎会通过推断成交对超出缓存 `filled_qty` 的任何差额进行对账。
- 缓存订单 + 比缓存数量少超过
  尘埃量的恢复成交：返回带恢复的 `filled_qty` 的 `Canceled`。
  引擎的 CANCELED 分支会在缓存的 `filled_qty` 处转换订单，
  因此在这种罕见的部分取消情形中，仅通过 REST（而非 WS）到达的任何新恢复成交
  不会被应用。关闭订单比让它卡在未成交状态更可取；如果在此场景中精确的成交元数据很重要，
  可以手动检查场所交易历史。
- 缓存订单，无交易：返回带
  `cancel_reason="ORDER_NOT_FOUND_AT_VENUE"` 的 `Canceled`。
- 无缓存订单（无论是否有交易）：返回 `None`；引擎的
  not-found-at-venue 路径会解析本地条目。

建议为 Polymarket 设置 `open_check_interval_secs`，以便引擎
周期性地为那些终态 WS 更新被遗漏的订单驱动这条恢复路径。

## 成交数量归一化

由于协议级的舍入，Polymarket 报告的成交数量会与提交的
订单数量略有偏移：CLOB 将撮合成交舍入
到整数分位最小变动价位（欠成交），而 V2 SDK 在市价买入报价数量订单上
将 `takerAmount` 截断到 USDC 刻度（过成交，几微份额）。
两种偏移源在绝对份额上都是固定的，因此适配器
用单一阈值 `DUST_SNAP_THRESHOLD = 0.01`
份额对它们进行归一化。超出该范围的任何部分都会作为真实的部分成交或
过成交呈现给引擎。

| 方向  | 来源                        | 适配器行为                    |
| --- | ------------------------- | ------------------------ |
| 过成交 | V2 USDC 刻度截断（微份额）         | 将成交向下吸附到 `submitted_qty` |
| 欠成交 | CLOB 分位最小变动价位截断（≤ `0.01`） | 保留；在 MATCHED 时合成尘埃成交     |

`FillReport.commission` 始终反映场所报告的数量，而非
吸附后的数量。几个最低单位（ulp）的差异在 pUSD 中小于微分。

成交跟踪器以 `venue_order_id` 为键，并在订单
接受时注册，因此在另一个会话中下单的订单的成交报告会原封不动地
通过。`DUST_SNAP_THRESHOLD` 不能按策略配置；它位于
`nautilus_polymarket::common::consts` 中。

## WebSocket

`PolymarketWebSocketClient` 基于高性能的 Nautilus `WebSocketClient` 基类构建，该基类由 Rust 编写。

### 数据

数据适配器在连接窗口期间缓冲初始的 `market` 订阅，然后随着请求新金融工具
而动态订阅。
当订阅数量增长超过配置的每连接上限时，客户端会在内部管理多个 WebSocket 连接。

### 运行时金融工具加载

Polymarket 列出了数千个活跃市场，新市场全天不断出现，因此在启动时预加载
完整宇宙很少切实可行。数据适配器按需自动加载缺失的金融工具，
以便策略可以订阅不在缓存中的市场：

- 当策略对未缓存的金融工具发出 `subscribe_quote_ticks`、`subscribe_trade_ticks`、`subscribe_order_book_deltas`
  或 `request_instrument` 时，适配器会注册该请求并
  等待 `auto_load_debounce_ms`（默认 100 毫秒），以便并发请求合并。
- 然后它发出单次批量 Gamma API 调用。超过 Gamma `condition_ids`
  查询上限（约 100）的批次会被拆分到多次调用并合并。
- 一旦金融工具被加载，它们会被发布到数据引擎（填充缓存），
  被推迟的订阅会原子地打开它们的 WebSocket 订阅。在自动加载进行中
  取消订阅的策略不会看到打开虚假的订阅。

该特性默认启用。通过在
`PolymarketDataClientConfig` 上设置 `auto_load_missing_instruments=False` 来禁用它。若想改为在启动时预加载已知的一组市场，请在
`PolymarketInstrumentProviderConfig` 上提供
`load_ids`、`event_slugs`、`market_slugs` 或 `event_slug_builder`。

新铸造的市场会经过一个数分钟的 CLOB 水化窗口，期间 Gamma
报告 `active=true`，但 `GET /markets/{cid}` 返回 404 或返回带空
`token_id` 字符串的 200。两套适配器都将这些归类为瞬态，并以
有界指数退避加抖动重试自动加载。通过 `auto_load_max_retries`
（默认 12）、`auto_load_retry_delay_initial_secs`（默认 5.0）和
`auto_load_retry_delay_max_secs`（默认 15.0）调整节奏；这些默认值将重试窗口上限约为 3
分钟。设置 `auto_load_max_retries=0` 可禁用重试。5 分钟市场（例如 updown crypto）
可能在场所完成水化之前就过期，因此请为此预留余量或提高上限。重试预算耗尽后，
仍在 Gamma 上缺失的条件会被记录为终态未命中。
Python 适配器随后会在下一次 `update_instruments_interval_mins` 刷新时重新拾起它；
Rust 适配器会让订阅保持未解析状态，直到调用方重新订阅。

### 市场解析事件

Rust 数据客户端在 `condition_id` 级别跟踪 Polymarket 敞口，以便当场所解析市场时
YES 和 NO 两条腿一起平仓。持仓事件会将未平仓的 Polymarket 二元
期权金融工具加入一个内部观察列表。一旦被观察的条件过期，数据客户端
会等待 `resolve_poll_grace_secs`，然后每隔 `resolve_poll_interval_secs` 轮询 Gamma，直到
该条件被解析或 `resolve_poll_max_wait_secs` 经过。

解析使用严格的赢家推断：

- Gamma 必须返回一个已关闭的二元市场，恰好有两个代币 ID、两个结果，以及一个二元的
  `outcomePrices` 形态。
- 如果 Gamma 没有为该条件提供严格的结果，客户端会回退到 CLOB
  `GET /markets/{condition_id}` 并使用 `tokens[].winner`。
- 非二元的、模糊的、格式错误的或仍未解析的载荷会被跳过。它们会保留在
  观察列表上，直到轮询窗口超时或手动请求解析它们。

当客户端应用一次解析时，它会为每条被跟踪的腿发出一个 `InstrumentStatus` 关闭和一个
`InstrumentClose`。赢家腿以 `1` 关闭，输家腿以 `0` 关闭。
关闭类型为 `InstrumentCloseType.ContractExpired`。该事件关闭 Nautilus 敞口，
并不在链上赎回代币或领取资金。

同一应用路径处理 WebSocket 的 `market_resolved` 事件、自动轮询和手动
请求。在 `resolve_poll_max_wait_secs` 之后，自动轮询会暂停被观察的条件并
记录它以供手动恢复。手动请求之后仍可重试该条件。

#### 手动解析请求

使用 `request_data()` 配合数据类型 `PolymarketResolveRequest` 来强制进行一次解析检查。该
请求接受以下任意参数：

| 参数               | 类型                  | 描述                              |
| ---------------- | ------------------- | ------------------------------- |
| `condition_id`   | `str`               | 解析一个 Polymarket 条件。             |
| `condition_ids`  | `str` 或 `list[str]` | 解析一个或多个 Polymarket 条件。          |
| `instrument_ids` | `str` 或 `list[str]` | 解析 Polymarket 金融工具 ID；其他场所会被忽略。 |

如果请求省略了所有选择器，客户端会使用观察列表。启用自动轮询时，
该回退会选择已暂停或已超时的条目。禁用自动轮询时，它会选择所有
已过期的合格条目，以便操作员可以手动运行恢复流程。

响应载荷是具有以下字典形态的自定义数据：

| 键                            | 含义                             |
| ---------------------------- | ------------------------------ |
| `requested_condition_ids`    | 请求检查的去重后的条件 ID。                |
| `fetched_markets`            | 在批量查找中返回的 Gamma 市场。            |
| `resolved_markets`           | 具有严格 Gamma 结果或成功 CLOB 回退结果的条件。 |
| `skipped_non_binary_markets` | 因非二元或模糊解析形态而被跳过的 Gamma 市场。     |
| `clob_fallback_successes`    | 通过 CLOB 回退路径解析的条件。             |
| `emitted_condition_ids`      | 发出了至少一个 `InstrumentClose` 的条件。 |
| `failed_condition_ids`       | Gamma 和 CLOB 查找都失败的条件。         |
| `used_watchlist_fallback`    | 请求是否从观察列表中选择了条件。               |
| `timed_out_watchlist`        | 在回退选择期间看到的已超时观察列表条目。           |
| `error`                      | 第一个汇总错误（如果发生了的话）。              |

赎回是单独的账户或执行工作流。不要扩展数据客户端的解析
路径去领取资金；它只将市场结果关闭事件发布到 Nautilus。

### 运行时清除金融工具

Polymarket 按需自动加载金融工具，因此长时间运行的会话会随着
市场解析、新市场出现以及策略在事件间轮换而不断增长缓存。使用 `cache.purge_instrument`
来丢弃策略不再跟踪的市场。该调用会移除金融工具记录以及每一个
以它为键的缓存自有映射（订单簿、报价、交易、bar）。

```python
class PolymarketHousekeeping(Strategy):
    def on_position_closed(self, event: PositionClosed) -> None:
        # 一旦持仓平仓且你不再有兴趣，就丢弃该市场。
        instrument_id = event.instrument_id
        self.unsubscribe_quote_ticks(instrument_id)
        self.unsubscribe_order_book_deltas(instrument_id)
        self.cache.purge_instrument(instrument_id)
```

Polymarket 上常见的触发条件：

- 某个市场解析并且不再产生交易。
- 某个事件结束，策略从其市场轮换离开。
- 策略轮换一个固定大小的观察列表并丢弃最旧的条目。

清除会跳过任何仍有非终态订单（initialized、submitted、
accepted、emulated、released 或 inflight）或非平仓持仓的金融工具，因此可以安全地调用而无需
与执行客户端协调。活跃的 WebSocket 订阅属于数据引擎。
如果你不再想要更新，请在清除前取消订阅。

缓存还暴露了 `purge_order`、`purge_position`、`purge_closed_orders`、
`purge_closed_positions` 和 `purge_account_events`，用于修剪已关闭的执行状态。
对于长时间运行的 Polymarket 节点，可从 `LiveExecEngineConfig`
调度批量清除（15 分钟间隔、60 分钟缓冲是一个合理的默认值）。完整集合参见
[缓存：清除缓存数据](../concepts/cache.md#purging-cached-data)。

:::warning
由调用方决定何时不再需要某个金融工具。清除另一个
actor、策略或引擎仍依赖的金融工具会导致金融工具查找缺失并丢失市场数据
历史。
:::

### 执行

执行适配器为订单和交易事件保持一个 `user` 频道连接，并根据交易期间所见的金融工具
按需管理市场订阅。

Python 和 Rust 适配器都支持动态的 WebSocket 订阅和取消订阅操作。

### 订阅限制

Polymarket 强制执行**每个 WebSocket 连接最多 500 个金融工具**的限制（未记录的限制）。

当你尝试在单个 WebSocket 连接上订阅 501 个或更多金融工具时：

- 你**不会**收到每个金融工具的初始订单簿快照。
- 你只会收到后续的订单簿更新。

NautilusTrader 自动管理 WebSocket 连接以处理此限制：

- 适配器默认为**每连接 200 个金融工具订阅**（在 Python 适配器中通过
  `ws_max_subscriptions_per_connection` 配置；在 Rust 适配器中为 `ws_max_subscriptions`）。
- 当订阅数量超过此限制时，会自动创建额外的 WebSocket 连接。
- 这确保你收到所有已订阅金融工具的完整订单簿数据（包括初始快照）。

:::tip
如果你需要订阅大量金融工具（例如 5000+），适配器将自动将这些订阅分布在多个 WebSocket 连接上。
你可以通过 `ws_max_subscriptions_per_connection`
（Python）或 `ws_max_subscriptions`（Rust）将每连接上限调高到 500。
:::

## 速率限制

Polymarket 通过 Cloudflare 节流来强制执行速率限制。
当超过限制时，请求会在滑动窗口上被节流。持续超额
仍可能表现为 HTTP 429 响应或临时封禁。

### REST 限制

Polymarket 会随时间更改这些配额。截至 2026-05-06，官方限制如下：

| 端点                            | 突发（10s） | 持续（10 分钟） | 备注                   |
| ----------------------------- | ------- | --------- | -------------------- |
| 通用速率限制                        | 15,000  | -         | 全局有记录的速率限制。          |
| 健康检查（`/ok`）                   | 100     | -         | 健康端点。                |
| CLOB 通用                       | 9,000   | -         | 跨 CLOB 端点的总和。        |
| CLOB `POST /order`            | 3,500   | 36,000    | 单订单提交。               |
| CLOB `POST /orders`           | 1,000   | 15,000    | 批量提交（每次请求最多 15 个订单）。 |
| CLOB `DELETE /order`          | 3,000   | 30,000    | 单订单取消。               |
| CLOB `DELETE /orders`         | 1,000   | 15,000    | 批量取消。                |
| CLOB `GET /balance-allowance` | 200     | -         | 余额和授权额度查询。           |
| CLOB API 密钥端点                 | 100     | -         | 密钥管理。                |
| Gamma 通用                      | 4,000   | -         | 跨 Gamma 端点的总和。       |
| Gamma `/markets`              | 300     | -         | 市场元数据。               |
| Gamma `/events`               | 500     | -         | 事件元数据。               |
| Data 通用                       | 1,000   | -         | 跨 Data API 端点的总和。    |
| Data `/trades`                | 200     | -         | 交易历史。                |
| Data `/positions`             | 150     | -         | 当前持仓。                |

### WebSocket 限制

WebSocket 配额不是已发布的 REST 速率限制表的一部分。
适配器自带一个可配置的每连接订阅上限
（Python 适配器中为 `ws_max_subscriptions_per_connection`，
Rust 适配器中为 `ws_max_subscriptions`），默认为 200；Polymarket
此前曾记录过每连接 500 的上限。

:::warning
超过 Polymarket 速率限制会触发 Cloudflare 节流。请求会使用
滑动窗口排队，而不是立即被拒绝，但持续超额可能
导致 HTTP 429 响应或临时封禁。
:::

### 数据加载器速率限制

`PolymarketDataLoader` 在使用默认 HTTP 客户端时包含内置的速率限制。
请求默认会自动被节流到每分钟 100 个请求。
这是一个 NautilusTrader 默认值，而非 Polymarket 当前发布的限制。
当前的 Rust HTTP 客户端也自带保守的每分钟 100 个请求的配额。

在跨多个市场获取大日期范围时：

- 共享同一 `http_client` 实例的多个加载器会自动协调速率限制。
- 为获得更高吞吐量，请传入具有调整后配额的自定义 `http_client`。
- 加载器不会在 429 错误时实现自动重试，因此如有需要请实现退避。

:::info
有关最新的速率限制详情，请参阅 Polymarket 官方文档：
<https://docs.polymarket.com/api-reference/rate-limits>
:::

## 限制和注意事项

目前已知以下限制：

- 通过 `py-clob-client-v2` 的 Python 订单签名速度较慢，每个订单大约需要一秒。
- 不支持 reduce-only 订单。
- 批量提交（`POST /orders`）每次请求最多接受 15 个订单；适配器会将更大的 `SubmitOrderList` 命令拆分为连续的 15 个一组的分块。

## 配置

Python 适配器和 Rust 原生适配器暴露了不同的配置项。下面的表格
完整记录了两套适配器。

### 数据客户端选项（Python v2）

类：`nautilus_trader.adapters.polymarket.config` 中的 `PolymarketDataClientConfig`。

| 选项                                    | 默认值          | 描述                                                        |
| ------------------------------------- | ------------ | --------------------------------------------------------- |
| `venue`                               | `POLYMARKET` | 为数据客户端注册的交易场所标识符。                                         |
| `private_key`                         | `None`       | 钱包私钥；省略时从 `POLYMARKET_PK` 读取。                             |
| `signature_type`                      | `0`          | 签名方案（0 = EOA，1 = 邮件代理，2 = 浏览器钱包代理）。                       |
| `funder`                              | `None`       | pUSD 资金钱包；省略时从 `POLYMARKET_FUNDER` 读取。                    |
| `api_key`                             | `None`       | API 密钥；省略时从 `POLYMARKET_API_KEY` 读取。                      |
| `api_secret`                          | `None`       | API 密钥；省略时从 `POLYMARKET_API_SECRET` 读取。                   |
| `passphrase`                          | `None`       | API 口令；省略时从 `POLYMARKET_PASSPHRASE` 读取。                   |
| `base_url_http`                       | `None`       | REST 基础 URL 覆盖。                                           |
| `base_url_ws`                         | `None`       | WebSocket 基础 URL 覆盖。                                      |
| `proxy_url`                           | `None`       | HTTP 和 WebSocket 传输的可选代理 URL。                             |
| `ws_connection_initial_delay_secs`    | `5`          | 第一次 WebSocket 连接前的延迟（秒），用于缓冲订阅。                           |
| `ws_connection_delay_secs`            | `0.1`        | 后续 WebSocket 连接尝试之间的延迟（秒）。                                |
| `ws_max_subscriptions_per_connection` | `200`        | 每个 WebSocket 连接的最大金融工具订阅数（Polymarket 限制为 500）。            |
| `update_instruments_interval_mins`    | `60`         | 金融工具目录刷新间隔（分钟）。                                           |
| `compute_effective_deltas`            | `False`      | 计算有效订单簿增量以节省带宽。                                           |
| `drop_quotes_missing_side`            | `True`       | 丢弃缺少买/卖价的报价，而不是替换为边界值。                                    |
| `auto_load_missing_instruments`       | `True`       | 当订阅或请求命令引用未缓存的金融工具时，按需加载金融工具。                             |
| `auto_load_debounce_ms`               | `100`        | 合并并发运行时金融工具加载的去抖窗口（毫秒）。                                   |
| `auto_load_max_retries`               | `12`         | 瞬态自动加载失败（CLOB 水化期间的 404 或空 `token_id`）的最大重试次数。设为 `0` 可禁用。 |
| `auto_load_retry_delay_initial_secs`  | `5.0`        | 瞬态自动加载重试之间的初始延迟（秒）。                                       |
| `auto_load_retry_delay_max_secs`      | `15.0`       | 瞬态自动加载重试之间的最大延迟（秒）。                                       |
| `instrument_config`                   | `None`       | 用于金融工具加载的可选 `PolymarketInstrumentProviderConfig`。         |

### 执行客户端选项（Python v2）

类：`nautilus_trader.adapters.polymarket.config` 中的 `PolymarketExecClientConfig`。

| 选项                                    | 默认值          | 描述                                                        |
| ------------------------------------- | ------------ | --------------------------------------------------------- |
| `venue`                               | `POLYMARKET` | 为执行客户端注册的交易场所标识符。                                         |
| `private_key`                         | `None`       | 钱包私钥；省略时从 `POLYMARKET_PK` 读取。                             |
| `signature_type`                      | `0`          | 签名方案（0 = EOA，1 = 邮件代理，2 = 浏览器钱包代理）。                       |
| `funder`                              | `None`       | pUSD 资金钱包；省略时从 `POLYMARKET_FUNDER` 读取。                    |
| `api_key`                             | `None`       | API 密钥；省略时从 `POLYMARKET_API_KEY` 读取。                      |
| `api_secret`                          | `None`       | API 密钥；省略时从 `POLYMARKET_API_SECRET` 读取。                   |
| `passphrase`                          | `None`       | API 口令；省略时从 `POLYMARKET_PASSPHRASE` 读取。                   |
| `base_url_http`                       | `None`       | REST 基础 URL 覆盖。                                           |
| `base_url_ws`                         | `None`       | WebSocket 基础 URL 覆盖。                                      |
| `base_url_data_api`                   | `None`       | Data API 基础 URL 覆盖（默认 `https://data-api.polymarket.com`）。 |
| `proxy_url`                           | `None`       | HTTP 和 WebSocket 传输的可选代理 URL。                             |
| `ws_max_subscriptions_per_connection` | `200`        | 每个 WebSocket 连接的最大金融工具订阅数（Polymarket 限制为 500）。            |
| `max_retries`                         | `None`       | 提交/取消请求的最大重试次数。                                           |
| `retry_delay_initial_ms`              | `None`       | 重试之间的初始延迟（毫秒）。                                            |
| `retry_delay_max_ms`                  | `None`       | 重试之间的最大延迟（毫秒）。                                            |
| `ack_timeout_secs`                    | `5.0`        | 等待来自缓存的订单/交易确认的超时时间（秒）。                                   |
| `generate_order_history_from_trades`  | `False`      | 为 `True` 时从交易报告生成合成订单历史（实验性功能）。                           |
| `log_raw_ws_messages`                 | `False`      | 为 `True` 时以 INFO 级别记录原始 WebSocket 载荷。                     |
| `instrument_config`                   | `None`       | 用于金融工具加载的可选 `PolymarketInstrumentProviderConfig`。         |

### 数据客户端选项（Rust v2）

结构体：`crates/adapters/polymarket/src/config.rs` 中的 `PolymarketDataClientConfig`。

| 选项                                   | 默认值                                       | 描述                                           |
| ------------------------------------ | ----------------------------------------- | -------------------------------------------- |
| `base_url_http`                      | `None`（官方 CLOB 端点）                        | CLOB REST 基础 URL 覆盖。                         |
| `base_url_ws`                        | `None`（官方 CLOB 端点）                        | CLOB WebSocket 基础 URL 覆盖。                    |
| `base_url_gamma`                     | `None`（官方 Gamma 端点）                       | Gamma API 基础 URL 覆盖。                         |
| `base_url_data_api`                  | `None`（`https://data-api.polymarket.com`） | Data API 基础 URL 覆盖。                          |
| `http_timeout_secs`                  | `60`                                      | HTTP 请求超时（秒）。                                |
| `ws_timeout_secs`                    | `30`                                      | WebSocket 连接/空闲超时（秒）。                        |
| `ws_max_subscriptions`               | `200`                                     | 每个 WebSocket 连接的最大金融工具订阅数。                   |
| `update_instruments_interval_mins`   | `60`                                      | 金融工具目录刷新间隔（分钟）。                              |
| `subscribe_new_markets`              | `false`                                   | 为 `true` 时通过 WebSocket 订阅新市场发现事件。            |
| `auto_load_missing_instruments`      | `true`                                    | 当订阅或请求命令引用未缓存的金融工具时，按需加载金融工具。                |
| `auto_load_debounce_ms`              | `100`                                     | 合并并发运行时金融工具加载的去抖窗口（毫秒）。                      |
| `auto_load_max_retries`              | `12`                                      | 瞬态自动加载失败（处于 CLOB 水化窗口的市场）的最大重试次数。设为 `0` 可禁用。 |
| `auto_load_retry_delay_initial_secs` | `5.0`                                     | 瞬态自动加载重试之间的初始延迟（秒）。                          |
| `auto_load_retry_delay_max_secs`     | `15.0`                                    | 瞬态自动加载重试之间的最大延迟（秒）。                          |
| `resolve_poll_enabled`               | `true`                                    | 自动轮询已过期的被观察条件以进行市场解析。                        |
| `resolve_poll_interval_secs`         | `30`                                      | 自动解析轮询尝试之间的间隔（秒）。                            |
| `resolve_poll_grace_secs`            | `10`                                      | 过期后到第一次自动解析轮询之间的延迟（秒）。                       |
| `resolve_poll_max_wait_secs`         | `1800`                                    | 过期后自动轮询暂停被观察条件以供手动恢复之前的最大等待时间（秒）。            |
| `filters`                            | `[]`                                      | 在加载和发现期间应用的金融工具过滤器。                          |
| `new_market_filter`                  | `None`                                    | 在发出之前应用于新发现市场的可选过滤器。                         |
| `transport_backend`                  | `Sockudo`                                 | WebSocket 传输后端。                              |

Rust 数据客户端配置不接受账户凭证；认证由
执行客户端处理。订阅缓冲（`ws_connection_initial_delay_secs`）和报价
处理（`compute_effective_deltas`、`drop_quotes_missing_side`）目前仅 Python 才有。

### 执行客户端选项（Rust v2）

结构体：`crates/adapters/polymarket/src/config.rs` 中的 `PolymarketExecClientConfig`。

| 选项                       | 默认值                                       | 描述                                                   |
| ------------------------ | ----------------------------------------- | ---------------------------------------------------- |
| `trader_id`              | 默认 `TraderId`                             | 客户端注册时使用的交易者标识符。                                     |
| `account_id`             | `POLYMARKET-001`                          | 此执行客户端的账户标识符。                                        |
| `private_key`            | `None`（`POLYMARKET_PK` 环境变量）              | 用于 EIP-712 签名的钱包私钥。                                  |
| `api_key`                | `None`（`POLYMARKET_API_KEY` 环境变量）         | CLOB API 密钥（L2 认证）。                                  |
| `api_secret`             | `None`（`POLYMARKET_API_SECRET` 环境变量）      | CLOB API 密钥（L2 认证）。                                  |
| `passphrase`             | `None`（`POLYMARKET_PASSPHRASE` 环境变量）      | CLOB API 口令（L2 认证）。                                  |
| `funder`                 | `None`（`POLYMARKET_FUNDER` 环境变量）          | pUSD 资金钱包；对于 `Poly1271`，这是充值钱包。                      |
| `signature_type`         | `Eoa`                                     | 签名方案（`Eoa`、`PolyProxy`、`PolyGnosisSafe`、`Poly1271`）。 |
| `base_url_http`          | `None`（官方 CLOB 端点）                        | CLOB REST 基础 URL 覆盖。                                 |
| `base_url_ws`            | `None`（官方 CLOB 端点）                        | CLOB WebSocket 基础 URL 覆盖。                            |
| `base_url_data_api`      | `None`（`https://data-api.polymarket.com`） | Data API 基础 URL 覆盖。                                  |
| `http_timeout_secs`      | `60`                                      | HTTP 请求超时（秒）。                                        |
| `max_retries`            | `3`                                       | 单订单提交/取消请求的最大重试次数。                                   |
| `retry_delay_initial_ms` | `1000`                                    | 重试之间的初始延迟（毫秒）。                                       |
| `retry_delay_max_ms`     | `10000`                                   | 重试之间的最大延迟（毫秒）。                                       |
| `ack_timeout_secs`       | `5`                                       | 等待 WebSocket 订单/交易确认的超时时间（秒）。                        |
| `transport_backend`      | `Sockudo`                                 | WebSocket 传输后端。                                      |

Rust 执行客户端不暴露 `generate_order_history_from_trades`、
`log_raw_ws_messages`、`ws_max_subscriptions_per_connection` 或 `instrument_config`。通过 `POST /orders` 的批量
提交无论 `max_retries` 如何都会有意跳过重试；
单订单路径在瞬态失败时仍会重试。

### 金融工具 provider 配置选项

金融工具 provider 配置通过数据客户端配置上的 `instrument_config` 参数传入。

| 选项                   | 默认值     | 描述                                                                     |
| -------------------- | ------- | ---------------------------------------------------------------------- |
| `load_all`           | `False` | 启动时加载所有场所金融工具。提供 slug 范围时自动设为 `True`。                                  |
| `event_slugs`        | `None`  | 通过 Gamma events 解析的静态 event slug。                                      |
| `market_slugs`       | `None`  | 通过 Gamma markets 直接加载的静态 market slug。                                  |
| `event_slug_builder` | `None`  | 用于可预测的 Up/Down 事件 slug 窗口的 Rust 支持的 `PolymarketUpDownEventSlugConfig`。 |

#### 事件 slug 构建器

Rust Python v2 适配器将 Python 视为配置、工厂和用户策略的边界。
Provider、数据和执行操作都在 Rust 中运行。因此 `event_slug_builder` 接受一个
Rust 支持的 `PolymarketUpDownEventSlugConfig`；它不接受 Python 可调用路径。

将其用于可预测的 Polymarket Up/Down 事件 slug，而无需下载完整的场所
目录。构建器以模式
`{asset}-updown-{interval_mins}m-{unix_timestamp}` 为配置的对齐周期窗口发出 slug。

```python
from nautilus_trader.adapters.polymarket import PolymarketInstrumentProviderConfig
from nautilus_trader.adapters.polymarket import PolymarketUpDownEventSlugConfig

instrument_config = PolymarketInstrumentProviderConfig(
    event_slug_builder=PolymarketUpDownEventSlugConfig(
        assets=["btc"],
        interval_mins=5,
        periods=3,
        start_offset_periods=0,
    ),
)
```

对于自定义事件模式，请传入显式的 `event_slugs`、传入直接的 `market_slugs`，或添加一个 Rust
过滤器或构建器。Rust v2 适配器拒绝 Python 可调用的 `event_slug_builder` 值，以便适配器
操作在实时交易期间不会跨入 Python。

## 历史数据加载

`PolymarketDataLoader` 提供了获取和解析历史市场数据的方法，
用于研究和回测（Backtest）目的。该加载器与多个 Polymarket API 集成以提供所需数据。

:::note
所有数据获取方法都是**异步**的，必须使用 `await` 调用。加载器可以选择性地接受 `http_client` 参数用于依赖注入（对测试有用）。
:::

### 数据来源

加载器从三个主要来源获取数据：

1. **Polymarket Gamma API** - 市场元数据、金融工具详情和活跃市场列表。
2. **Polymarket CLOB API** - 用于构建金融工具的市场详情。
3. **Polymarket Data API** - 历史交易和当前用户持仓。

当前的加载器**未**暴露用于 CLOB 价格历史时间序列或订单簿
历史快照的辅助方法。

### 方法命名约定

加载器提供了两种访问 Polymarket API 的方式：

| 前缀        | 类型   | 使用场景                           |
| --------- | ---- | ------------------------------ |
| `query_*` | 静态方法 | 无需金融工具即可探索 API。不需要加载器实例。       |
| `fetch_*` | 实例方法 | 使用已配置的加载器获取数据。使用加载器的 HTTP 客户端。 |

**在以下情况使用 `query_*`：**当你想要在确定某个特定金融工具之前探索市场、发现事件或获取元数据时：

```python
# 无需加载器：直接查询 API
market = await PolymarketDataLoader.query_market_by_slug("some-market")
event = await PolymarketDataLoader.query_event_by_slug("some-event")
```

**在以下情况使用 `fetch_*`：**当你有一个加载器实例并想要使用其
已配置的 HTTP 客户端获取数据时（用于跨多次调用协调速率限制）：

```python
loader = await PolymarketDataLoader.from_market_slug("some-market")

# 所有 fetch 调用共享加载器的 HTTP 客户端
markets = await loader.fetch_markets(active=True, limit=100)
events = await loader.fetch_events(active=True)
details = await loader.fetch_market_details(condition_id)
```

### 查找市场

使用提供的实用脚本来发现活跃市场：

```bash
# 列出所有活跃市场
python nautilus_trader/adapters/polymarket/scripts/active_markets.py

# 专门列出 BTC 和 ETH UpDown 市场
python nautilus_trader/adapters/polymarket/scripts/list_updown_markets.py
```

### 基本用法

创建加载器的推荐方式是使用工厂类方法，它们会自动处理
所有 API 调用和金融工具创建：

```python
import asyncio

from nautilus_trader.adapters.polymarket import PolymarketDataLoader


async def main():
    # 从 market slug 创建加载器（推荐）
    loader = await PolymarketDataLoader.from_market_slug("gta-vi-released-before-june-2026")

    # 加载器已设置好 instrument 和 token_id，可立即使用
    print(loader.instrument)
    print(loader.token_id)


asyncio.run(main())
```

对于包含多个市场的事件（例如温度区间），请使用 `from_event_slug`：

```python
# 返回一个加载器列表，事件中的每个市场对应一个
loaders = await PolymarketDataLoader.from_event_slug("highest-temperature-in-nyc-on-january-26")
```

#### 已解析市场的前视保护

当为一个在回测构建时已经解析的市场构建加载器时，
场所载荷会包含答案（`closed`、`closedTime`、
`umaResolutionStatus`、每个代币的 `winner`）。一个从 `on_start` 读取
`cache.instrument(...).info` 的策略，因此可以在模拟运行之前看到
结果。

向任一工厂传入 `sanitize_info=True`，可在金融工具构建之前
从 `instrument.info` 中删除这些字段。被删除的切片会
作为 `resolution_metadata` 暂存在加载器上，用于事后分析
（结算 PnL、Brier 评分），而不会泄露到模拟中：

```python
loader = await PolymarketDataLoader.from_market_slug(
    "some-resolved-market",
    sanitize_info=True,
)

assert "closed" not in loader.instrument.info
assert loader.resolution_metadata["closed"] is True
```

### 发现市场和事件

使用 `fetch_markets()` 和 `fetch_events()` 以编程方式发现可用的市场：

```python
loader = await PolymarketDataLoader.from_market_slug("any-market")

# 列出活跃市场
markets = await loader.fetch_markets(active=True, closed=False, limit=100)
for market in markets:
    print(f"{market['slug']}: {market['question']}")

# 列出活跃事件
events = await loader.fetch_events(active=True, limit=50)
for event in events:
    print(f"{event['slug']}: {event['title']}")

# 获取特定事件内的所有市场
event_markets = await loader.get_event_markets("highest-temperature-in-nyc-on-january-26")
```

如需在不创建加载器的情况下快速探索，请使用静态的 `query_*` 方法
（参见上文[方法命名约定](#方法命名约定)）。

### 获取交易历史

`load_trades()` 便捷方法一步完成获取和解析历史交易：

```python
import pandas as pd

# 加载所有可用交易
trades = await loader.load_trades()

# 或按时间范围过滤（客户端过滤）
end = pd.Timestamp.now(tz="UTC")
start = end - pd.Timedelta(hours=24)

trades = await loader.load_trades(
    start=start,
    end=end,
)
```

或者，你可以使用底层方法分别获取和解析：

```python
condition_id = loader.condition_id

# 从 Polymarket Data API 获取原始交易
raw_trades = await loader.fetch_trades(condition_id=condition_id)

# 解析为 NautilusTrader TradeTick
trades = loader.parse_trades(raw_trades)
```

交易数据来自 [Polymarket Data API](https://data-api.polymarket.com/trades)，
它提供真实的执行数据，包括价格、数量、方向和链上交易哈希。

:::note
公开的 Data API 在高活跃度市场上对基于偏移量的分页设有上限。当
触及该上限时，加载器会发出一个 `RuntimeWarning`，并返回截至上限
所获取的交易，而不是中止加载。如果你需要某个交易频繁的市场的
完整覆盖，请使用另一个历史数据源。
:::

### 完整回测示例

完整的工作示例位于 `examples/backtest/polymarket_simple_quoter.py`：

```python
import asyncio
from decimal import Decimal

from nautilus_trader.adapters.polymarket import POLYMARKET_VENUE
from nautilus_trader.adapters.polymarket import PolymarketDataLoader
from nautilus_trader.backtest.config import BacktestEngineConfig
from nautilus_trader.backtest.engine import BacktestEngine
from nautilus_trader.examples.strategies.ema_cross_long_only import EMACrossLongOnly
from nautilus_trader.examples.strategies.ema_cross_long_only import EMACrossLongOnlyConfig
from nautilus_trader.model.currencies import pUSD
from nautilus_trader.model.data import BarType
from nautilus_trader.model.enums import AccountType
from nautilus_trader.model.enums import OmsType
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.objects import Money


async def run_backtest():
    # 初始化加载器并获取市场数据
    loader = await PolymarketDataLoader.from_market_slug("gta-vi-released-before-june-2026")
    instrument = loader.instrument

    # 从 Polymarket Data API 加载历史交易
    trades = await loader.load_trades()

    # 配置并运行回测
    config = BacktestEngineConfig(trader_id=TraderId("BACKTESTER-001"))
    engine = BacktestEngine(config=config)

    engine.add_venue(
        venue=POLYMARKET_VENUE,
        oms_type=OmsType.NETTING,
        account_type=AccountType.CASH,
        base_currency=pUSD,
        starting_balances=[Money(10_000, pUSD)],
    )

    engine.add_instrument(instrument)
    engine.add_data(trades)

    bar_type = BarType.from_str(f"{instrument.id}-100-TICK-LAST-INTERNAL")
    strategy_config = EMACrossLongOnlyConfig(
        instrument_id=instrument.id,
        bar_type=bar_type,
        trade_size=Decimal("20"),
    )

    strategy = EMACrossLongOnly(config=strategy_config)
    engine.add_strategy(strategy=strategy)
    engine.run()

    # 显示结果
    print(engine.trader.generate_account_report(POLYMARKET_VENUE))


# 运行回测
asyncio.run(run_backtest())
```

**运行完整示例**：

```bash
python examples/backtest/polymarket_simple_quoter.py
```

### 辅助函数

适配器提供了用于处理 Polymarket 标识符的实用函数：

```python
from nautilus_trader.adapters.polymarket import get_polymarket_instrument_id

# 从 Polymarket 标识符创建 NautilusTrader InstrumentId
instrument_id = get_polymarket_instrument_id(
    condition_id="0xcccb7e7613a087c132b69cbf3a02bece3fdcb824c1da54ae79acc8d4a562d902",
    token_id="8441400852834915183759801017793514978104486628517653995211751018945988243154",
)
```

## 贡献

:::info
如需了解更多功能或为 Polymarket 适配器做出贡献，请参阅我们的
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
