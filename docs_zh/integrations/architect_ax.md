# AX Exchange

[AX Exchange](https://architect.exchange) 是全球首家针对传统标的资产类别永续期货的中心化、受监管交易所。它由 Architect Bermuda Ltd. 运营，并获得 [百慕大金融管理局 (Bermuda Monetary Authority, BMA)](https://www.bma.bm/) 的牌照许可。AX 将加密货币风格的永续合约带入传统金融市场，覆盖外汇、金属、能源、股指以及利率等领域。

本集成支持与 AX Exchange 进行实时市场数据接入和订单执行。

## 示例 (Examples)

你可以在 [此处](https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/architect_ax/) 找到实时示例脚本。

## 概览 (Overview)

本指南假设交易者要同时配置实时市场数据订阅和交易执行。AX Exchange 适配器包含多个组件，可根据使用场景一起使用或单独使用。

- `AxHttpClient`：底层 HTTP API 连接。
- `AxMdWebSocketClient`：市场数据 WebSocket 连接。
- `AxOrdersWebSocketClient`：订单 WebSocket 连接。
- `AxInstrumentProvider`：金融工具解析与加载功能。
- `AxDataClient`：市场数据订阅管理器。
- `AxExecutionClient`：账户管理与交易执行网关。
- `AxLiveDataClientFactory`：AX 数据客户端工厂（由交易节点构建器使用）。
- `AxLiveExecClientFactory`：AX 执行客户端工厂（由交易节点构建器使用）。

:::note
大多数用户会为实时交易节点定义一份配置（如下所示），通常不需要直接操作这些底层组件。
:::

## AX Exchange 文档 (AX Exchange documentation)

AX Exchange 为用户提供了文档，可在 [Architect 文档站点](https://docs.architect.exchange/) 找到。建议你在阅读本 NautilusTrader 集成指南的同时，也参考 AX Exchange 的官方文档。

## 产品 (Products)

AX Exchange 专注于传统资产类别上的永续期货合约。永续合约永不到期，从而消除了标准期货所带来的展期成本。

| 资产类别 | 示例                           | 备注          |
| ---- | ---------------------------- | ----------- |
| 外汇   | GBPUSD-PERP, EURUSD-PERP     | 主要和次要外汇货币对。 |
| 股指   | 股票指数永续合约                     |             |
| 金属   | XAU-PERP (黄金), XAG-PERP (白银) | 贵金属永续合约。    |
| 能源   | 原油、天然气                       | 能源商品永续合约。   |
| 利率   | SOFR、国债收益率                   | 利率永续合约。     |

### 永续合约 (Perpetual contracts)

永续合约（永续掉期，perpetual swap）是一种跟踪标的资产价格且永不到期的衍生品。与标准期货不同，它没有结算日，从而消除了展期成本并简化了仓位管理。资金费率 (funding rate) 机制通过多头和空头持有者之间的周期性支付，使合约价格与标的指数价格保持一致。关于资金费率机制和合约规格的详情，请参阅 [Architect 文档](https://docs.architect.exchange/)。

AX 永续合约的特征：

- **以 USD 现金结算**：无实物交割。所有盈亏均以 USD 结算。
- **资金费率**：周期性支付使合约价格与标的保持一致。
- **乘数为 1**：每张合约代表对标的的一单位敞口。
- **仅支持整数张合约**：不支持小数数量。
- **保证金**：开仓需要初始保证金，维持持仓需要维持保证金。

在 NautilusTrader 中，所有 AX 金融工具都表示为 `PerpetualContract`，这是一种与资产类别无关的永续掉期类型。资产类别（外汇、商品、股票等）会根据标的自动推断。该适配器使用 `MARGIN` 账户类型和 `NETTING` 订单管理。

## 符号体系 (Symbology)

AX Exchange 使用直观的命名约定。所有金融工具都是永续期货，通过在标的资产符号后追加 `-PERP` 后缀来标识。

**格式**：`{SYMBOL}-PERP`

| 标的      | AX 符号         | Nautilus InstrumentId |
| ------- | ------------- | --------------------- |
| GBP/USD | `GBPUSD-PERP` | `GBPUSD-PERP.AX`      |
| EUR/USD | `EURUSD-PERP` | `EURUSD-PERP.AX`      |
| 黄金      | `XAU-PERP`    | `XAU-PERP.AX`         |
| 白银      | `XAG-PERP`    | `XAG-PERP.AX`         |

场所标识符为 `AX`。要构造一个 Nautilus `InstrumentId`：

```python
from nautilus_trader.model.identifiers import InstrumentId

instrument_id = InstrumentId.from_str("GBPUSD-PERP.AX")
```

## 环境 (Environments)

AX Exchange 提供两个交易环境。在客户端配置中使用 `environment` 参数配置合适的环境。

| 环境             | 配置                                     | 描述           |
| -------------- | -------------------------------------- | ------------ |
| **Sandbox**    | `environment=AxEnvironment.SANDBOX`    | 使用模拟资金的测试环境。 |
| **Production** | `environment=AxEnvironment.PRODUCTION` | 使用真实资金的实盘交易。 |

### Sandbox

用于开发和测试的默认环境，使用模拟资金。当 `environment=AxEnvironment.SANDBOX` 时，所有 sandbox 端点都会被自动解析。

#### 1. 创建 sandbox 账户

按照 [Architect 文档](https://docs.architect.exchange/) 创建一个 sandbox 账户。注册时需要邀请码。

#### 2. 创建 API 密钥并为账户充值

使用 AX sandbox UI 生成 API 密钥，并向你的账户存入模拟资金。请妥善保管 `api_key` 和 `api_secret`。

#### 3. 设置环境变量

```bash
export AX_API_KEY="your-sandbox-api-key"
export AX_API_SECRET="your-sandbox-api-secret"
```

#### 4. 配置交易节点

```python
config = TradingNodeConfig(
    ...,  # Omitted
    data_clients={
        AX: AxDataClientConfig(
            environment=AxEnvironment.SANDBOX,
            instrument_provider=InstrumentProviderConfig(load_all=True),
        ),
    },
    exec_clients={
        AX: AxExecClientConfig(
            environment=AxEnvironment.SANDBOX,
            instrument_provider=InstrumentProviderConfig(load_all=True),
        ),
    },
)
```

### Production

用于使用真实资金的实盘交易。需要一个已通过验证的 AX Exchange 账户。

```python
config = AxExecClientConfig(
    environment=AxEnvironment.PRODUCTION,
)
```

:::warning
在下单前，请确保你使用的是正确的环境。默认使用 Sandbox 以防止意外进行实盘交易。
:::

## 市场数据 (Market data)

该适配器通过 WebSocket 订阅提供实时市场数据，并通过 HTTP 端点进行历史数据回填。

### 数据类型 (Data types)

| AX 数据    | Nautilus 数据类型       | 备注                               |
| -------- | ------------------- | -------------------------------- |
| 订单簿 (L1) | `QuoteTick`         | 来自 L1 订单簿订阅的最优买/卖盘口。             |
| 订单簿 (L2) | `OrderBookDelta`    | 聚合的价格档位。                         |
| 订单簿 (L3) | `OrderBookDelta`    | 单笔订单数量。                          |
| 成交       | `TradeTick`         | 来自 L1 订阅的实时成交事件。                 |
| 标记价格     | `MarkPriceUpdate`   | 从 L1 ticker 订阅中提取。               |
| K 线/蜡烛图  | `Bar`               | OHLCV 数据（仅总成交量，无买/卖分解）。          |
| 资金费率     | `FundingRateUpdate` | 通过 HTTP 轮询（非实时 WebSocket）；间隔可配置。 |
| 金融工具状态   | `InstrumentStatus`  | 来自 L1 ticker 订阅的状态变化（开市、暂停、收市）。  |

:::note
AX Exchange 不支持历史报价 tick 请求。仅可通过 WebSocket L1 订单簿订阅获取实时报价数据。
:::

### K 线周期 (Bar intervals)

| 周期    | 描述    |
| ----- | ----- |
| `1s`  | 1 秒   |
| `5s`  | 5 秒   |
| `1m`  | 1 分钟  |
| `5m`  | 5 分钟  |
| `15m` | 15 分钟 |
| `1h`  | 1 小时  |
| `1d`  | 1 天   |

## 订单能力 (Orders capability)

AX Exchange 支持带止损触发的市价单和限价单类型。

### 订单类型 (Order types)

| 订单类型                   | 是否支持 | 备注                   |
| ---------------------- | ---- | -------------------- |
| `MARKET`               | ✓    | 以最优可用价格立即执行。         |
| `LIMIT`                | ✓    | 以指定价格或更优价格执行。        |
| `STOP_LIMIT`           | ✓    | 当止损价被触及时触发一个限价单。     |
| `LIMIT_IF_TOUCHED`     | -    | *AX Exchange 当前未实现*。 |
| `STOP_MARKET`          | -    | *不支持*。               |
| `MARKET_IF_TOUCHED`    | -    | *不支持*。               |
| `TRAILING_STOP_MARKET` | -    | *不支持*。               |

### 执行指令 (Execution instructions)

| 指令            | 是否支持 | 备注                      |
| ------------- | ---- | ----------------------- |
| `post_only`   | ✓    | 仅做 maker；若订单会吃掉流动性则被拒绝。 |
| `reduce_only` | -    | *不支持*。                  |

### 有效期 (Time in force)

| 有效期            | 是否支持 | 备注                             |
| -------------- | ---- | ------------------------------ |
| `GTC`          | ✓    | Good Till Canceled（撤销前有效）。     |
| `GTD`          | -    | *AX Exchange 不支持*。             |
| `DAY`          | ✓    | 在交易日结束前有效。                     |
| `IOC`          | ✓    | Immediate or Cancel（立即成交否则取消）。 |
| `FOK`          | ✓    | Fill or Kill（全部成交否则取消）。        |
| `AT_THE_OPEN`  | ✓    | 在开市时执行，否则失效。                   |
| `AT_THE_CLOSE` | ✓    | 在收市时执行，否则失效。                   |

### 高级订单特性 (Advanced order features)

| 特性     | 是否支持 | 备注                                         |
| ------ | ---- | ------------------------------------------ |
| 订单修改   | ✓    | 通过 `POST /replace_order` 原子替换。返回一个新的订单 ID。 |
| 撤单     | ✓    | 单笔订单撤销。                                    |
| 撤销全部订单 | ✓    | 撤销某个金融工具的所有未结订单。                           |
| 批量撤单   | -    | *AX Exchange 不支持*。改用逐笔撤单。                  |
| 订单列表   | ✓    | 顺序提交（订单逐笔提交，非原子）。                          |

### 仓位管理 (Position management)

| 特性   | 是否支持 | 备注                  |
| ---- | ---- | ------------------- |
| 查询仓位 | ✓    | 实时仓位更新。             |
| 仓位模式 | -    | 仅支持净额 (netting) 模式。 |
| 跨保证金 | ✓    | 跨所有金融工具的跨保证金。       |

### 订单查询 (Order querying)

| 特性     | 是否支持 | 备注                          |
| ------ | ---- | --------------------------- |
| 查询未结订单 | ✓    | 列出所有活动订单。                   |
| 查询单笔订单 | ✓    | 按场所订单 ID 或客户端订单 ID（任意订单状态）。 |
| 订单状态报告 | ✓    | 从未结订单进行对账；见下方说明。            |
| 成交报告   | ✓    | 执行与成交历史。                    |

:::note
用于对账的订单状态报告由未结订单端点生成。已成交或已撤销的订单不会包含在对账快照中。通过 `query_order` 进行的单笔订单查询使用专用的 `/order-status` 端点，该端点对任意订单状态都有效。
:::

## 认证 (Authentication)

AX Exchange 使用 bearer token 认证：

1. 通过 `/authenticate`，使用 API key 和 secret 获取会话令牌 (session token)。
2. 会话令牌作为 bearer token 用于后续的 REST 和 WebSocket 请求。
3. 会话令牌在可配置的时长后过期（默认：86400 秒）。

## 配置 (Configuration)

### 环境与端点 (Environments and endpoints)

| 环境         | HTTP API（市场数据）                                   | HTTP API（订单）                                        | 市场数据 WS                                          | 订单 WS                                                |
| ---------- | ------------------------------------------------ | --------------------------------------------------- | ------------------------------------------------ | ---------------------------------------------------- |
| Sandbox    | `https://gateway.sandbox.architect.exchange/api` | `https://gateway.sandbox.architect.exchange/orders` | `wss://gateway.sandbox.architect.exchange/md/ws` | `wss://gateway.sandbox.architect.exchange/orders/ws` |
| Production | `https://gateway.architect.exchange/api`         | `https://gateway.architect.exchange/orders`         | `wss://gateway.architect.exchange/md/ws`         | `wss://gateway.architect.exchange/orders/ws`         |

:::info
订单管理的 HTTP 端点（下单、撤单、订单状态）使用与市场数据端点不同的基础 URL。这由适配器配置自动处理。
:::

### 数据客户端配置选项 (Data client configuration options)

| 选项                                 | 默认值       | 描述                                      |
| ---------------------------------- | --------- | --------------------------------------- |
| `api_key`                          | `None`    | API key；省略时从 `AX_API_KEY` 环境变量加载。       |
| `api_secret`                       | `None`    | API secret；省略时从 `AX_API_SECRET` 环境变量加载。 |
| `environment`                      | `SANDBOX` | 交易环境（`SANDBOX` 或 `PRODUCTION`）。         |
| `base_url_http`                    | `None`    | 覆盖 REST 基础 URL。                         |
| `base_url_ws_public`               | `None`    | 覆盖市场数据 WebSocket URL。                   |
| `base_url_ws_private`              | `None`    | 覆盖订单 WebSocket URL。                     |
| `proxy_url`                        | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。           |
| `http_timeout_secs`                | `60`      | REST 请求的超时时间（秒）。                        |
| `max_retries`                      | `3`       | REST 请求的最大重试次数。                         |
| `retry_delay_initial_ms`           | `1000`    | 重试之间的初始延迟（毫秒）。                          |
| `retry_delay_max_ms`               | `10000`   | 重试之间的最大延迟（毫秒，指数退避）。                     |
| `heartbeat_interval_secs`          | `20`      | WebSocket 连接的心跳间隔（秒）。                   |
| `recv_window_ms`                   | `5000`    | 签名请求的接收窗口（毫秒）。                          |
| `update_instruments_interval_mins` | `60`      | 金融工具目录刷新的间隔（分钟）。                        |
| `funding_rate_poll_interval_mins`  | `15`      | 资金费率轮询请求的间隔（分钟）。                        |
| `transport_backend`                | `Sockudo` | WebSocket 传输后端。                         |

### 执行客户端配置选项 (Execution client configuration options)

| 选项                        | 默认值       | 描述                                      |
| ------------------------- | --------- | --------------------------------------- |
| `api_key`                 | `None`    | API key；省略时从 `AX_API_KEY` 环境变量加载。       |
| `api_secret`              | `None`    | API secret；省略时从 `AX_API_SECRET` 环境变量加载。 |
| `environment`             | `SANDBOX` | 交易环境（`SANDBOX` 或 `PRODUCTION`）。         |
| `base_url_http`           | `None`    | 覆盖 REST 基础 URL。                         |
| `base_url_orders`         | `None`    | 覆盖订单 REST 基础 URL。                       |
| `base_url_ws_private`     | `None`    | 覆盖订单 WebSocket URL。                     |
| `proxy_url`               | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。           |
| `http_timeout_secs`       | `60`      | REST 请求的超时时间（秒）。                        |
| `max_retries`             | `3`       | REST 请求的最大重试次数。                         |
| `retry_delay_initial_ms`  | `1000`    | 重试之间的初始延迟（毫秒）。                          |
| `retry_delay_max_ms`      | `10000`   | 重试之间的最大延迟（毫秒，指数退避）。                     |
| `heartbeat_interval_secs` | `30`      | WebSocket 连接的心跳间隔（秒）。                   |
| `recv_window_ms`          | `5000`    | 签名请求的接收窗口（毫秒）。                          |
| `cancel_on_disconnect`    | `false`   | 当订单 WebSocket 断开时撤销所有未结订单。              |
| `transport_backend`       | `Sockudo` | WebSocket 传输后端。                         |

最常见的使用场景是配置一个实时 `TradingNode`，使其包含 AX Exchange 的数据和执行客户端。为此，请在你的客户端配置中添加一个 `AX` 部分：

```python
from nautilus_trader.adapters.architect_ax import AX
from nautilus_trader.adapters.architect_ax import AxDataClientConfig
from nautilus_trader.adapters.architect_ax import AxEnvironment
from nautilus_trader.adapters.architect_ax import AxExecClientConfig
from nautilus_trader.config import InstrumentProviderConfig
from nautilus_trader.config import TradingNodeConfig

config = TradingNodeConfig(
    ...,  # Omitted
    data_clients={
        AX: AxDataClientConfig(
            environment=AxEnvironment.SANDBOX,
            instrument_provider=InstrumentProviderConfig(load_all=True),
        ),
    },
    exec_clients={
        AX: AxExecClientConfig(
            environment=AxEnvironment.SANDBOX,
            instrument_provider=InstrumentProviderConfig(load_all=True),
        ),
    },
)
```

然后，创建一个 `TradingNode` 并添加客户端工厂：

```python
from nautilus_trader.adapters.architect_ax import AX
from nautilus_trader.adapters.architect_ax import AxLiveDataClientFactory
from nautilus_trader.adapters.architect_ax import AxLiveExecClientFactory
from nautilus_trader.live.node import TradingNode

# Instantiate the live trading node with a configuration
node = TradingNode(config=config)

# Register the client factories with the node
node.add_data_client_factory(AX, AxLiveDataClientFactory)
node.add_exec_client_factory(AX, AxLiveExecClientFactory)

# Finally build the node
node.build()
```

### API 凭据 (API credentials)

向 AX Exchange 客户端提供凭据有两种方式。要么将对应的 `api_key` 和 `api_secret` 值传递给配置对象，要么设置以下环境变量：

- `AX_API_KEY`
- `AX_API_SECRET`

:::tip
我们建议使用环境变量来管理你的凭据。
:::

在启动交易节点时，你将立即收到关于凭据是否有效以及是否具有交易权限的确认。

## 实现说明 (Implementation notes)

- **仅支持整数张合约**：AX Exchange 使用整数张合约数量。不支持小数数量；适配器会在本地生成 `OrderDenied`。
- **速率限制**：适配器应用保守的速率限制，为每秒 10 个请求，并在收到速率限制响应时自动进行指数退避。
- **市价单**：AX 不支持原生市价单。适配器使用一个预览端点来确定吃单穿透价格 (take-through price)，并提交一个激进的 IOC 限价单。
- **订单修改**：AX 通过 `POST /replace_order` 支持原子订单替换。适配器将 `modify_order` 映射到此端点。交易所会撤销原始订单并创建一个带有更新字段的新订单，返回一个新的订单 ID。
- **断开时撤单**：在执行客户端配置中设置 `cancel_on_disconnect=True`，可让交易所在订单 WebSocket 断开时撤销所有未结订单。
- **成交手续费**：来自 WebSocket 的实时成交事件不包含费用数据。流式成交的手续费报告为零。在对账期间，REST `/fills` 端点会提供准确的费用信息。

## 贡献 (Contributing)

:::info
如需更多功能或为 AX Exchange 适配器做出贡献，请参阅我们的 [贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
