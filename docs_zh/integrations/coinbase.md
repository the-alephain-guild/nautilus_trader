# Coinbase

Coinbase 成立于 2012 年，是美国受监管的最大加密货币交易所之一，通过 Advanced Trade API
提供现货 (spot)、永续合约 (perpetual swaps) 和到期期货 (dated futures) 的交易。本适配器
(adapter) 支持在现货 (Cash) 账户和 CFM 衍生品 (Margin) 账户上进行实时市场数据接入和订单执行，
两者共享同一个执行客户端，账户类型由工厂 (factory) 选择（参见 [执行范围](#执行范围-execution-scope)）。

:::note
本适配器仅由 Rust 实现，由 v2 系统（以及 Rust 的 `LiveNode`）消费。它不提供旧版 Python
`TradingNode` 集成；只有配置和枚举类型通过 PyO3 导出，以便 v2 Python 入口点可以构造它们。
:::

## 概述

Coinbase 适配器由 Rust 实现，由 v2 系统消费。适配器不提供旧版 Python `TradingNode` 集成；
只有配置和枚举类型通过 PyO3 导出，以便 v2 入口点可以从 Python 构造它们。

组件：

- `CoinbaseHttpClient`：双层 REST 客户端（原始端点方法 + 领域包装层）。
- `CoinbaseWebSocketClient`：底层 WebSocket 连接，带 JWT 订阅认证。
- `CoinbaseInstrumentProvider`：金融工具 (instrument) 解析和加载。
- `CoinbaseDataClient`：市场数据源管理器。
- `CoinbaseDataClientFactory`：数据客户端工厂。
- `CoinbaseExecutionClient`：执行客户端（现货或 CFM 衍生品；REST 下单 + WS 数据流）。
- `CoinbaseExecutionClientFactory`：执行客户端工厂；现货还是 CFM 衍生品由配置中的 `account_type` 选择。

可从 `nautilus_trader.adapters.coinbase` 访问的接口：

- `CoinbaseDataClientConfig`、`CoinbaseExecClientConfig`
- `CoinbaseEnvironment`、`CoinbaseMarginType`
- `COINBASE` 交易场所 (venue) 常量

## Coinbase 文档

Coinbase 为 Advanced Trade API 提供了文档：

- [REST API 参考](https://docs.cdp.coinbase.com/advanced-trade/reference)
- [WebSocket 频道](https://docs.cdp.coinbase.com/advanced-trade/docs/ws-channels)
- [API 密钥认证](https://docs.cdp.coinbase.com/coinbase-app/authentication-authorization/api-key-authentication)
- [速率限制](https://docs.cdp.coinbase.com/advanced-trade/docs/rate-limits)

我们建议你在使用本 NautilusTrader 集成 (integration) 指南的同时，也参阅 Coinbase 的文档。

:::info
本适配器面向 Coinbase Advanced Trade API。独立的
[Coinbase International Exchange (INTX)](https://international.coinbase.com)
交易场所由专门的 `coinbase_intx` 适配器支持。
:::

## 产品 (Products)

产品 (product) 是一组相关金融工具类型的统称。

支持以下产品类型：

| 产品类型 | 是否支持 | 备注                           |
| ---- | ---- | ---------------------------- |
| 现货   | ✓    | 以 USD、USDC 和 USDT 计价的现货交易对。  |
| 永续合约 | ✓    | FCM 交易场所上以 USD 计价的永续合约。      |
| 期货合约 | ✓    | 到期交割期货（nano BTC、nano ETH 等）。 |

## 符号体系 (Symbology)

Coinbase 直接将交易场所的原生 `product_id` 字段用作 Nautilus 符号。金融工具 ID 为
`{product_id}.COINBASE`。

| 产品   | 格式                              | 示例                                 |
| ---- | ------------------------------- | ---------------------------------- |
| 现货   | `{base}-{quote}`                | `BTC-USD`、`ETH-USDC`、`SOL-USDT`。   |
| 永续合约 | `{contract_code}-{ddMMMyy}-CDE` | `BIP-20DEC30-CDE`（BTC PERP）。       |
| 到期期货 | `{contract_code}-{ddMMMyy}-CDE` | `BIT-24APR26-CDE`（BTC 2026 年 4 月）。 |

`-CDE` 后缀表示 Coinbase Derivatives Exchange（FCM 交易场所）。永续合约带有交易所分配的
远期到期日（例如 `20DEC30`），但由于存在持续的资金费率 (funding rate)，它们被归类为
`CryptoPerpetual`。到期期货被归类为 `CryptoFuture`。

适配器从 API 元数据结构化地解析产品类型（依据 `future_product_details.contract_expiry_type`；
当它为 `EXPIRING` 时，依据是否存在非空的 `future_product_details.funding_rate` 作为永续合约
独有的结构化信号）；后备启发式方法会检查 `display_name` 中是否包含 `PERP` 或 `Perpetual` 子串。

完整 Nautilus 金融工具 ID 示例：

- `BTC-USD.COINBASE`（现货 Bitcoin/USD）。
- `ETH-USDC.COINBASE`（现货 Ether/USDC）。
- `BIP-20DEC30-CDE.COINBASE`（BTC 永续合约）。
- `BIT-24APR26-CDE.COINBASE`（BTC 到期期货，2026 年 4 月）。

### 别名产品（USDC 与 USD）

Coinbase 将同一交易对的 USDC 计价版本和 USD 计价版本合并到单一的撮合引擎订单簿中，并通过
`GET /products` 的 `alias` 和 `alias_to` 字段暴露这种关系：

```text
BTC-USD :  alias=""        alias_to=["BTC-USDC"]   # 规范形式 (canonical)
BTC-USDC:  alias="BTC-USD" alias_to=[]             # BTC-USD 的别名
```

当调用方使用别名一侧进行订阅或提交时，交易场所会在传输层将请求重写为规范 ID。适配器透明地
处理这一点：它在启动时记录 `product_id -> alias` 映射，在订阅和下单时发送规范 ID，在 WebSocket
客户端上注册反向映射，并在解析前将入站消息重新键控回调用方提供的 ID。

因此，一个只持有 USDC 的策略可以端到端地交易 `BTC-USDC.COINBASE`，而无需引用规范的 `BTC-USD`。
结算货币由提交的 `product_id` 决定，所以在 `BTC-USDC.COINBASE` 上下的订单总是借记或贷记 USDC 钱包。

## 环境 (Environments)

Coinbase 提供两种交易环境。使用客户端配置中的 `environment` 字段配置相应的环境。

| 环境      | `environment` 值               | REST 基础 URL                        |
| ------- | ----------------------------- | ---------------------------------- |
| Live    | `CoinbaseEnvironment.LIVE`    | `https://api.coinbase.com`         |
| Sandbox | `CoinbaseEnvironment.SANDBOX` | `https://api-sandbox.coinbase.com` |

### Live（生产环境）

使用真实资金进行实时交易的默认环境。

```python
config = CoinbaseExecClientConfig(
    api_key="YOUR_API_KEY",
    api_secret="YOUR_API_SECRET",
    # environment=CoinbaseEnvironment.LIVE (默认)
)
```

环境变量：`COINBASE_API_KEY`、`COINBASE_API_SECRET`。

### Sandbox

一个用于集成管线测试的静态模拟测试环境，详见
[Sandbox 文档](https://docs.cdp.coinbase.com/coinbase-app/advanced-trade-apis/sandbox)。

```python
config = CoinbaseExecClientConfig(
    api_key="ANY_NON_EMPTY_STRING",  # 适配器构造函数要求
    api_secret="ANY_NON_EMPTY_STRING",
    environment=CoinbaseEnvironment.SANDBOX,
)
```

Sandbox 交易场所不强制认证，但 `CoinbaseExecutionClient::new` 仍然要求两个字段（或对应的
环境变量）存在才能完成构造。

:::warning
**Sandbox 不是一个平行的交易场所：**

- 所有响应都是静态且预先定义的；没有实时市场或动态定价。
- 仅提供 Accounts 和 Orders 端点；其他资源不可用。
- 不要求认证（也不强制）。
- 自定义的 `X-Sandbox` 请求头可以触发预定义的错误场景。

使用 sandbox 来接通你的客户端并验证请求/响应的结构；任何真实的行为测试都应使用生产环境
（使用真实资金并谨慎操作）。
:::

## 认证 (Authentication)

Coinbase Advanced Trade 使用 ES256 JWT 认证。每个 REST 请求和每个 WebSocket 订阅都会生成
一个用你的 EC 私钥签名的短期 JWT。适配器从环境变量或配置字段中解析凭证。

### 创建 API 密钥

Coinbase 有多种密钥类型。本适配器要求使用采用 **ECDSA** 签名算法（而非 Ed25519）的
**Coinbase App Secret API key**。

<Steps>
<Step>
前往 CDP 门户的 API 密钥页面：
[portal.cdp.coinbase.com/projects/api-keys](https://portal.cdp.coinbase.com/projects/api-keys)。
</Step>
<Step>
选择 **Secret API Keys** 标签页并点击 **Create API key**。
</Step>
<Step>
输入一个昵称（例如 `nautilus-trading`）。
</Step>
<Step>
展开 **API restrictions** 并将权限设置为 **View** 和 **Trade**。
</Step>
<Step>
展开 **Advanced Settings** 并将签名算法从 Ed25519 改为 **ECDSA**。这一步是必需的：
Ed25519 密钥无法用于 Advanced Trade API。
</Step>
<Step>
点击 **Create API key**。从弹窗中保存密钥名称和私钥。密钥名称形如
`organizations/{org_id}/apiKeys/{key_id}`。私钥是 PEM 编码的 EC 密钥（SEC1 格式）。
</Step>
</Steps>

:::warning
Coinbase 不再自动下载密钥文件。请在关闭弹窗前从创建弹窗中复制这些值，或点击下载按钮。之后
你将无法再取回私钥。
:::

:::info
不要使用来自 coinbase.com/settings/api 的旧版 API 密钥（UUID 格式，HMAC-SHA256 签名）。
它们使用不同的认证方案（`CB-ACCESS-*` 请求头），本适配器不支持。
:::

完整细节请参阅 Coinbase 的
[API 密钥认证指南](https://docs.cdp.coinbase.com/coinbase-app/authentication-authorization/api-key-authentication)。

### 环境变量

| 变量                    | 描述                                               |
| --------------------- | ------------------------------------------------ |
| `COINBASE_API_KEY`    | 密钥名称（`organizations/{org_id}/apiKeys/{key_id}`）。 |
| `COINBASE_API_SECRET` | PEM 编码的 EC 私钥（完整的多行字符串）。                         |

示例：

```bash
export COINBASE_API_KEY="organizations/abc-123/apiKeys/def-456"
export COINBASE_API_SECRET="$(cat ~/path/to/cdp_api_key.pem)"
```

:::tip
我们建议使用环境变量来管理你的凭证。
:::

### JWT 生命周期

Coinbase 的 JWT 在 120 秒后过期。根据
[WebSocket 概述](https://docs.cdp.coinbase.com/coinbase-app/advanced-trade-apis/websocket/websocket-overview)，
每个经过认证的 WebSocket 消息（即每次订阅）都必须生成一个不同的 JWT。适配器会为每个签名的
REST 请求和每个经过认证的订阅消息重新生成一个新的 JWT；无需手动轮换。

## 投资组合 (Portfolios)

一个 Coinbase 账户持有一个或多个 **投资组合 (portfolio)**。每个投资组合都有自己的钱包
（USD、USDC、BTC 等）、余额和订单范围。每个账户都有一个 `DEFAULT` 投资组合；用户可以创建
额外的 `CONSUMER` 投资组合，以隔离策略、风险或税务批次。

一个 CDP API 密钥在 **创建时即绑定到单个投资组合**。除非显式指定其他投资组合，否则每个经过
认证的请求（账户查询、订单提交、撤单）都针对该投资组合操作。

### 查找你的投资组合 UUID

运行适配器的认证探测二进制程序；它会打印你的 CDP 密钥可见的投资组合、所绑定投资组合中的
账户余额，以及一些参考性的 REST 调用：

```bash
cargo run --bin coinbase-http-private --package nautilus-coinbase
```

示例输出：

```
Found 1 portfolio(s)
  name=Default type=DEFAULT uuid=ca7244bc-21d1-5e4c-bfe5-80f208ac5723 deleted=false
Account has 3 balance(s)
  USDC total=100.00000000 USDC free=100.00000000 USDC locked=0.00000000 USDC
  AUD total=0.00 AUD free=0.00 AUD locked=0.00 AUD
  BTC total=0.00000000 BTC free=0.00000000 BTC locked=0.00000000 BTC
```

等效的 curl（你必须先用你的 CDP PEM 密钥签出自己的 ES256 JWT）：

```bash
curl -H "Authorization: Bearer $JWT" \
  https://api.coinbase.com/api/v3/brokerage/portfolios
```

### 何时需要 `retail_portfolio_id`

Coinbase 的 `POST /orders` 端点默认路由到密钥所绑定的投资组合，所以单投资组合账户不需要设置
此字段。当下列任一情况成立时，在
[`CoinbaseExecClientConfig`](#执行客户端配置选项) 上设置它：

- 账户持有多个投资组合，而你想针对一个非密钥默认投资组合进行交易。
- 交易场所以 `account is not available` 拒绝订单，且已排除下文的钱包诊断。

### 创建新投资组合

大多数用户不需要创建新投资组合；账户的默认投资组合开箱即用。仅在你希望进行以下操作时，才在
[coinbase.com/portfolios](https://www.coinbase.com/portfolios) 上创建一个：

- 将 API 驱动的交易与手动零售活动隔离开。
- 在不同策略之间隔离风险或盈亏。
- 绕过受限的默认投资组合（例如 Vault）。

创建投资组合后，在发送任何订单之前先为其注资（在 coinbase.com 上从默认投资组合的钱包转账），
否则交易场所会针对计价货币返回 `account is not available`。

### 排查 `account is not available`

交易场所会因若干不同原因返回此错误；通过运行上面的探测二进制程序并检查投资组合钱包列表来诊断。

| 现象                                  | 可能原因                                                                              | 修复方法                                                                                                           |
| ----------------------------------- | --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| 仅针对特定产品被拒（例如只持有 USDC 时下单 `BTC-USD`） | 投资组合缺少该产品计价货币的钱包。在 Coinbase 上 USD 和 USDC 是分开的，且交易场所按提交的 `product_id` 而非规范别名来路由订单。 | 针对你所持有计价货币的产品提交（例如对 USDC 钱包使用 `BTC-USDC`）。适配器在内部解析数据侧的别名；无需修改配置。通过 coinbase.com 为缺失的钱包注资也是一个选项，但在只持有一种货币时并非必要。 |
| 所有产品上的每个订单都被拒                       | 密钥绑定到一个非默认投资组合，而 `retail_portfolio_id` 未设置。                                       | 将 `CoinbaseExecClientConfig` 上的 `retail_portfolio_id` 设置为目标投资组合 UUID。                                          |
| 非美国账户在 `*-USD` 产品上被拒                | 司法管辖限制（例如澳大利亚账户不能交易 USD 计价的交易对）。                                                  | 使用本地可用的计价货币（USDC、AUD、EUR 等）代替 USD。                                                                             |
| 密钥轮换后立即被拒                           | 新密钥创建在了与上一个密钥不同的投资组合中。                                                            | 更新 `retail_portfolio_id` 以匹配新密钥的投资组合，或转移资金。                                                                    |

## 订单功能 (Orders capability)

下面的表格描述了 Coinbase **交易场所** 的订单接口。所附带的
[`CoinbaseExecutionClient`](#执行范围-execution-scope) 会根据配置的 `account_type` 处理现货
或 CFM 衍生品。Coinbase 的订单功能在现货和衍生品之间有所不同（永续合约和到期期货共享同一套
FCM 订单接口）。

### 执行范围 (Execution scope)

`CoinbaseExecutionClientFactory` 只生产单一的 `CoinbaseExecutionClient` 类型。产品族由
`CoinbaseExecClientConfig` 上的 `account_type` 字段选择：

| `account_type`        | 启动加载的金融工具                               | 账户状态来源                                                                                |
| --------------------- | --------------------------------------- | ------------------------------------------------------------------------------------- |
| `AccountType::Cash`   | 仅 `CoinbaseProductType::Spot`。          | `/accounts` REST 端点。                                                                  |
| `AccountType::Margin` | `CoinbaseProductType::Future`（永续 + 到期）。 | CFM `balance_summary` REST + `futures_balance_summary` WS，以及来自 `cfm/positions` 的持仓报告。 |

其他账户类型会在工厂创建时被拒绝。OMS 始终为 `Netting`，因为交易场所不暴露对冲模式。

为了防止跨账户的串扰：

1. 连接时的金融工具启动加载被限制在配置的产品族内；另一个产品族的产品永远不会进入进程内缓存。
2. `submit_order` 会拒绝任何金融工具不在该缓存内的订单。
3. `generate_order_status_report(s)` 和 `generate_fill_reports` 会通过同一缓存对其输出做
   后过滤，所以一个同时持有现货和衍生品活动的 Coinbase 账户不会通过单个客户端暴露另一个范围的报告。

每个范围运行一个执行客户端；如果你需要在同一个 trader 上同时进行现货和 CFM 活动，请用不同的
`account_type` 值（以及不同的 `account_id`）实例化两个客户端。

### 订单类型 (Order types)

该矩阵列出了通过 Nautilus 模型暴露的订单类型。右列显示适配器发出的对应 `order_configuration`
键。不在此表中的 Coinbase 订单类型（TWAP、Bracket、Scaled、SOR LIMIT IOC）记录在
[高级订单功能](#高级订单功能-advanced-order-features) 中，并在那里标注为适配器 *尚未支持*。

| 订单类型                   | 现货  | 永续  | 期货  | 传输层结构                                                     |
| ---------------------- | --- | --- | --- | --------------------------------------------------------- |
| `MARKET`               | ✓   | ✓   | ✓   | `market_market_ioc`（现货 + CFM）；`market_market_fok`（仅 CFM）  |
| `LIMIT`                | ✓   | ✓   | ✓   | `limit_limit_gtc` / `limit_limit_gtd` / `limit_limit_fok` |
| `STOP_LIMIT`           | -   | ✓   | ✓   | `stop_limit_stop_limit_gtc` / `stop_limit_stop_limit_gtd` |
| `STOP_MARKET`          | -   | -   | -   | *交易场所未暴露。*                                                |
| `MARKET_IF_TOUCHED`    | -   | -   | -   | *交易场所未暴露。*                                                |
| `LIMIT_IF_TOUCHED`     | -   | -   | -   | *交易场所未暴露。*                                                |
| `TRAILING_STOP_MARKET` | -   | -   | -   | *交易场所未暴露。*                                                |

### 执行指令 (Execution instructions)

| 指令            | 现货  | 永续  | 期货  | 备注                        |
| ------------- | --- | --- | --- | ------------------------- |
| `post_only`   | ✓   | ✓   | ✓   | 仅限 LIMIT GTC 和 LIMIT GTD。 |
| `reduce_only` | -   | ✓   | ✓   | 仅限衍生品。                    |

### 有效期 (Time in force)

适配器接受此矩阵中的值；未列出的组合会在提交时被拒绝，并返回
`"Unsupported TIF {tif} for {order_type}"`。

| 订单类型         | GTC | GTD | IOC | FOK | 备注                                                                                                                               |
| ------------ | --- | --- | --- | --- | -------------------------------------------------------------------------------------------------------------------------------- |
| `MARKET`     | ✓   | -   | ✓   | (✓) | GTC 被映射为 IOC；显式的 IOC 会被遵守。FOK 会构造交易场所的 `market_market_fok` 结构，但撮合引擎目前在现货上会以 `UNSUPPORTED_ORDER_CONFIGURATION` 拒绝它；仅在 CFM 衍生品上可用。 |
| `LIMIT`      | ✓   | ✓   | -   | ✓   | GTD 需要 `expire_time`。LIMIT IOC *尚未支持*（参见 [SOR LIMIT IOC](#高级订单功能-advanced-order-features)）。                                      |
| `STOP_LIMIT` | ✓   | ✓   | -   | -   | 需要 `trigger_price`。仅限衍生品。                                                                                                        |

### 高级订单功能 (Advanced order features)

| 功能            | 现货  | 永续  | 期货  | 备注                                                            |
| ------------- | --- | --- | --- | ------------------------------------------------------------- |
| 订单修改          | ✓   | ✓   | ✓   | 仅限 GTC 变体（LIMIT、STOP_LIMIT、Bracket）；其他类型使用撤单-重发。              |
| Bracket 订单    | -   | -   | -   | *尚未支持。* 交易场所暴露 `trigger_bracket_gtc` / `trigger_bracket_gtd`。 |
| OCO 订单        | -   | -   | -   | 交易场所 *未将其作为独立的订单类型暴露*。                                        |
| 冰山订单          | -   | -   | -   | *交易场所未暴露。*                                                    |
| TWAP 订单       | -   | -   | -   | *尚未支持。* 交易场所暴露 `twap_limit_gtd`。                              |
| Scaled 订单     | -   | -   | -   | *尚未支持。* 交易场所暴露 `scaled_limit_gtc`。                            |
| SOR LIMIT IOC | -   | -   | -   | *尚未支持。* 交易场所暴露 `sor_limit_ioc`，用于智能订单路由的 LIMIT IOC。           |

底层交易场所规范请参阅
[Create Order 参考](https://docs.cdp.coinbase.com/api-reference/advanced-trade-api/rest-api/orders/create-order)
和 [Edit Order 参考](https://docs.cdp.coinbase.com/api-reference/advanced-trade-api/rest-api/orders/edit-order)。

### 持仓控制（衍生品）

| 控制项   | 备注                         |
| ----- | -------------------------- |
| 杠杆    | 按订单设置；默认 `1.0`。            |
| 保证金类型 | 按订单设置：cross（默认）或 isolated。 |
| 持仓模式  | 仅单向；不暴露对冲模式。               |

### 批量操作

| 操作   | 备注                                                                                         |
| ---- | ------------------------------------------------------------------------------------------ |
| 批量提交 | 不支持。每个订单是一个 `Create Order` 请求。                                                             |
| 批量修改 | 不支持。每次编辑是一个 `Edit Order` 请求。                                                               |
| 批量取消 | `POST /api/v3/brokerage/orders/batch_cancel` 接受一个 `order_ids` 数组。没有文档化的最大数量；响应中按订单返回成功/失败。 |

### 订单查询

| 功能      | 现货  | 永续  | 期货  | 备注                    |
| ------- | --- | --- | --- | --------------------- |
| 查询未成交订单 | ✓   | ✓   | ✓   | 列出所有活跃订单。             |
| 查询订单历史  | ✓   | ✓   | ✓   | 带游标分页的历史订单数据。         |
| 订单状态更新  | ✓   | ✓   | ✓   | 通过 `user` 频道实时更新状态变化。 |
| 交易历史    | ✓   | ✓   | ✓   | 执行和成交报告。              |

### 现货交易限制

- 现货订单不支持 `reduce_only`（该指令适用于衍生品）。
- 不支持追踪止损订单。
- 现货上没有原生的止损限价单和 bracket 订单。
- 支持以计价货币计量的 MARKET 订单；LIMIT 订单以基础货币单位计量。

### 衍生品交易

Coinbase 衍生品通过 FCM（期货佣金商，Futures Commission Merchant）交易场所进行交易。执行
客户端通过与现货相同的 `POST /orders` 端点提交订单；每个订单的 `leverage` 和 `margin_type`
（`CROSS` 或 `ISOLATED`）默认值来自 `CoinbaseExecClientConfig.default_leverage` 和
`default_margin_type`。保证金余额从 REST `cfm/balance_summary` 端点（连接时快照、
`query_account` 以及 WebSocket 重连时）和经过认证的 `futures_balance_summary` WebSocket
频道两处更新。持仓报告来自 REST `cfm/positions` 端点。

Coinbase 的 Advanced Trade API 没有在创建订单的 schema 中记录 `reduce_only` 字段，即使交易
场所的失败原因枚举承认这个概念。客户端为了 API 一致性在其 `submit_order` 签名中保留了
`reduce_only`，并仅在其被设为 `true` 时才在传输层包含该标志；如果交易场所日后接受它，无需
修改客户端。

#### 资金费率

适配器以 `derivatives_poll_interval_secs`（默认 15 秒）轮询 REST `/products/{id}` 端点，
并在 `funding_rate` 存在时从 FCM 的 `future_product_details` 载荷发出一个
`FundingRateUpdate`。资金费率间隔从 `funding_interval` 字段解析（通常为 `"3600s"`，即每小时
结算），下一次资金费率时间戳从 `funding_time` 解析。Coinbase Advanced Trade 不在 WebSocket
`ticker` 频道上发布 `funding_rate`，所以 REST 轮询是唯一的实时来源。

历史资金费率请求（`DataTester` TC-D53）尚未实现；同一个 REST products 端点可以在未来的版本
中提供它们，从连续的资金费率时间戳推导出间隔。

#### 金融工具状态

`subscribe_instrument_status` 在首次订阅时加入 Coinbase WebSocket 的 `status` 频道（交易
场所为所有产品发布单一的状态源），将入站事件过滤到已订阅的金融工具，并发出 `InstrumentStatus`
事件：`online` 对应 `MarketStatusAction::Trading`，`offline` 对应 `Halt`，`delisted` 对应
`Close`。报告空 `status` 字符串的期货产品对数据引擎没有任何信息，会被跳过。当最后一个金融工具
取消订阅时，该频道订阅会被丢弃。

#### 持仓对账

对于 Cash（现货）账户，客户端不返回任何持仓报告，因为 Coinbase 现货没有持仓。对于 Margin
账户，持仓报告来自 REST `cfm/positions`（列表）和 `cfm/positions/{product_id}`（单个）端点，
并被后过滤到启动加载的金融工具缓存。未成交订单和历史成交在连接时以及由 `LiveExecEngineConfig`
设置的标准对账间隔上，通过 `generate_order_status_report(s)` 和 `generate_fill_reports` 从
REST 对账。

#### 成交去重

user 频道的 WebSocket 可能在重连时重放事件。执行客户端维护一个 10,000 条目的 FIFO 去重表，
以 `(venue_order_id, trade_id)` 为键，并丢弃任何合成的 trade ID 与近期已见 ID 匹配的成交。
累积状态映射以相同容量做了上限约束，以防止那些在本客户端生命周期内从未收到终态事件的订单。在
非常长的断连之后（超出内存去重窗口），重放的成交可能发出重复的 `OrderFilled` 事件；在这种情况下
策略应依赖 REST 对账来恢复规范状态。

## 执行客户端行为 (Execution client behavior)

本节记录 `CoinbaseExecutionClient` 如何将 Nautilus 订单命令和 Coinbase 交易场所事件转换为
Nautilus 执行事件。

### 订单提交

`submit_order` 直接从 Nautilus 订单字段构造 Coinbase 的 `order_configuration` 结构：

- `MARKET` -> `market_market_ioc`。只接受 `TimeInForce::Ioc` 和 `Gtc`（Nautilus 默认值）；
  市价单上任何显式的 `Fok`、`Day` 或 `Gtd` 都会在 HTTP 调用之前被拒绝，从而避免调用方无声地
  获得 IOC 语义。用 `Gtc` 构造的 `MARKET` 订单在交易场所会以 IOC 执行；需要严格回测/实盘一致性
  的策略应显式地用 `Ioc` 构造 `MarketOrder`。
- `LIMIT` GTC -> `limit_limit_gtc`，GTD -> `limit_limit_gtd`（需要 `expire_time`），
  FOK -> `limit_limit_fok`。
- `STOP_LIMIT` GTC -> `stop_limit_stop_limit_gtc`，GTD -> `stop_limit_stop_limit_gtd`。
  止损方向由订单方向推导（`Buy` -> `STOP_DIRECTION_STOP_UP`，
  `Sell` -> `STOP_DIRECTION_STOP_DOWN`）。
- `STOP_MARKET`、`MARKET_IF_TOUCHED`、`LIMIT_IF_TOUCHED` 和追踪止损变体未被交易场所暴露。
  它们会以 `OrderRejected` 的形式出现，携带来自所派生的提交任务的 `build_order_configuration`
  错误（订单会先以 `OrderSubmitted` 发出）。

在 HTTP 创建成功时，会发出一个 `OrderAccepted`，携带 `success_response.order_id` 中返回的
交易场所订单 ID。在 `success=false` 的响应上，会发出 `OrderRejected`，并附带格式化的交易场所
失败原因。交易场所结果未知的 HTTP 失败会让订单保持在途状态，等待 WebSocket 更新、未成交订单
轮询或对账处理。

### 订单修改

`modify_order` 用类型化的 `EditOrderRequest` 向 `/orders/edit` 发送请求。Coinbase 将编辑
限制在 GTC 变体（LIMIT、STOP_LIMIT、Bracket）；其他订单类型必须使用撤单-重发。

Coinbase 的 `/orders/edit` 即使只修改其中一项，也同时要求 `price` 和 `size`；省略的 `size`
会被读作 0 并以 `INVALID_EDITED_SIZE` 或 `CANNOT_EDIT_TO_BELOW_FILLED_SIZE` 拒绝。执行客户端
会从缓存的订单中自动填充缺失字段，所以策略可以调用 `modify_order(price=X)` 而无需重复当前数量。
`ModifyOrder` 命令中的值优先；否则使用缓存订单当前的 `price` 和 `quantity`。

交易场所的编辑失败会发出 `OrderModifyRejected`，附带类型化的 `EditOrderResponse` 原因（优先
使用 `edit_failure_reason`，回退到 `preview_failure_reason`）。交易场所结果未知的 HTTP 失败
会让订单保持 `PENDING_UPDATE`，直到一次更新、查询结果或对账将其解决。

### 撤单

- `cancel_order` 发送一个单 ID 的 `batch_cancel`。明确的单订单交易场所失败会以
  `OrderCancelRejected` 出现；整个请求的传输失败且交易场所结果未知时，会让订单保持
  `PENDING_CANCEL` 以待对账。
- `cancel_all_orders` 通过 REST 列出未成交订单，但不使用仅 `OPEN` 的过滤器（因为 Coinbase
  的 `OPEN` 过滤器会排除仍可撤销的 `PENDING` 和 `QUEUED` 订单），在本地过滤到
  `{Submitted, Accepted, Triggered, PendingUpdate, PartiallyFilled}` 和请求的方向，然后以
  100 个为一组分块调用 `batch_cancel`。单订单的交易场所失败会发出 `OrderCancelRejected`；
  整个请求失败且交易场所结果未知时，会让受影响的订单保持待对账状态。
- `batch_cancel_orders` 以相同方式分块，并将明确的单订单交易场所失败暴露为
  `OrderCancelRejected`。传输失败且交易场所结果未知时，会让受影响的订单保持待对账状态。

### user WebSocket 频道

`CoinbaseExecutionClient` 以一个全新的 JWT 订阅 `user` 频道，不带 `product_ids` 过滤器，
将每个事件解析为一个 `OrderStatusReport`，并将其送入执行事件流。Coinbase 报告的是每个订单的
累积状态，而非逐笔成交，所以执行客户端从累积增量合成出一个 `FillReport`。每笔成交的价格按
`(avg_now * qty_now - avg_prev * qty_prev) / delta_qty` 推导，使多笔成交的订单携带正确的
成交价格，而非累积加权平均价。在交易场所将 `leaves_quantity` 清零的终态更新（`CANCELLED`、
`EXPIRED`、`FAILED`）上，原始数量会被恢复。

user 频道不会回传 `price`、`stop_price`、`trigger_type` 或 maker/taker 分类。执行客户端在
提交时以 `client_order_id` 为键缓存这些值，并在发出前修补报告，使对账器不会观察到
`Some(price) -> None` 的偏差，且 `post_only` 成交被正确地标记为 `liquidity_side = Maker`。
订单状态 `PENDING`、`QUEUED` 和 `OPEN` 都映射到 `OrderStatus::Accepted`，以避免在 user
频道更新与 REST `OrderAccepted` 事件竞争时出现虚假的反向状态转换警告。

携带 `INVALID_LIMIT_PRICE_POST_ONLY`（或预览/新订单的等价物）的 `submit_order` 拒绝会以
`due_post_only = true` 发出，使策略可以对 post-only 穿价做出反应（通常是针对新的 TOB 重新报价）。

在重连时，会通过 REST 重新获取账户状态，从而恢复断连窗口期间的余额变化。每个订单的累积跟踪
会跨重连持久化，使合成的成交增量保持正确。

## 速率限制 (Rate limiting)

Coinbase 为 Advanced Trade API 发布了以下限制：

| 接口                 | 限制                                         | 来源                                   |
| ------------------ | ------------------------------------------ | ------------------------------------ |
| WebSocket 连接       | 每个 IP 地址每秒 8 个                             | Advanced Trade WebSocket Rate Limits |
| WebSocket 未认证消息    | 每个 IP 地址每秒 8 个                             | Advanced Trade WebSocket Rate Limits |
| WebSocket 订阅截止时间   | 第一条订阅消息必须在连接后 5 秒内到达，否则服务器断开连接             | Advanced Trade WebSocket Overview    |
| 经认证的 WebSocket JWT | 120 秒；每个经过认证的订阅消息都必须生成一个新的 JWT             | Advanced Trade WebSocket Overview    |
| REST 每密钥配额         | 每个 API 密钥每小时 10,000 个请求（Coinbase App 通用政策） | Coinbase App Rate Limiting           |

当超过 REST 限制时，Coinbase 返回 HTTP `429`，响应体如下：

```json
{
  "errors": [
    {
      "id": "rate_limit_exceeded",
      "message": "Too many requests"
    }
  ]
}
```

:::info
在撰写本文时，Advanced Trade 专属的 REST 配额（每秒上限、每投资组合限制）未在 Advanced Trade
文档中单独发布；上面的 Coinbase App 每小时配额是文档中最具体的值。参考：
[REST 速率限制](https://docs.cdp.coinbase.com/advanced-trade/docs/rest-api-rate-limits/)、
[WebSocket 速率限制](https://docs.cdp.coinbase.com/advanced-trade/docs/ws-rate-limits)、
[Coinbase App 速率限制](https://docs.cdp.coinbase.com/coinbase-app/api-architecture/rate-limiting)。
:::

## 重连与重新订阅

WebSocket 客户端在重连时使用指数退避，基数为 250ms，上限为 30s。重连后，订阅会按它们创建的
顺序自动恢复。Coinbase 要求在连接后 5 秒内发送订阅消息，否则服务器断开连接；适配器会在
WebSocket 握手完成后立即发送排队的订阅。

对于经过认证的频道（`user`，以及 Margin 客户端上的 `futures_balance_summary`），适配器会为
每条订阅消息生成一个新的 JWT；根据 Coinbase 文档，"你必须为发送的每条 WebSocket 消息生成一个
不同的 JWT，因为 JWT 将在 120 秒后过期。" 一旦订阅被接受，数据流将在 WebSocket 连接的生命
周期内持续，无需进一步认证。

当执行客户端的 WebSocket 重连时，内部客户端会被从头重建（而不是依赖现有连接的状态机），以保证
即使上一会话的 `Disconnect` 命令与关闭信号发生竞争，也能获得一组全新的 `cmd_tx`/`out_rx`/信号
三元组。每个订单的累积跟踪会跨重连持久化，使合成的成交增量保持正确。

## 配置 (Configuration)

### 数据客户端配置选项

| 选项                                 | 默认值       | 描述                                                          |
| ---------------------------------- | --------- | ----------------------------------------------------------- |
| `api_key`                          | `None`    | 回退到 `COINBASE_API_KEY` 环境变量。                                |
| `api_secret`                       | `None`    | 回退到 `COINBASE_API_SECRET` 环境变量。                             |
| `base_url_rest`                    | `None`    | REST 基础 URL 覆盖。                                             |
| `base_url_ws`                      | `None`    | 市场数据 WebSocket URL 覆盖。                                      |
| `proxy_url`                        | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。                               |
| `environment`                      | `Live`    | `Live` 或 `Sandbox`。                                         |
| `http_timeout_secs`                | `10`      | HTTP 请求超时（秒）。                                               |
| `ws_timeout_secs`                  | `30`      | WebSocket 超时（秒）。                                            |
| `update_instruments_interval_mins` | `60`      | 金融工具目录刷新之间的间隔。                                              |
| `derivatives_poll_interval_secs`   | `15`      | 发出 `IndexPriceUpdate` 和 `FundingRateUpdate` 的 REST 轮询之间的间隔。 |
| `transport_backend`                | `Sockudo` | WebSocket 传输后端。                                             |

### 执行客户端配置选项

| 选项                       | 默认值       | 描述                                                                                                           |
| ------------------------ | --------- | ------------------------------------------------------------------------------------------------------------ |
| `api_key`                | `None`    | 回退到 `COINBASE_API_KEY` 环境变量。                                                                                 |
| `api_secret`             | `None`    | 回退到 `COINBASE_API_SECRET` 环境变量。                                                                              |
| `base_url_rest`          | `None`    | REST 基础 URL 覆盖。                                                                                              |
| `base_url_ws`            | `None`    | 用户数据 WebSocket URL 覆盖。                                                                                       |
| `proxy_url`              | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。                                                                                |
| `environment`            | `Live`    | `Live` 或 `Sandbox`。                                                                                          |
| `http_timeout_secs`      | `10`      | HTTP 请求超时（秒）。                                                                                                |
| `max_retries`            | `3`       | HTTP 请求的最大重试次数。                                                                                              |
| `retry_delay_initial_ms` | `100`     | 初始重试延迟（毫秒）。                                                                                                  |
| `retry_delay_max_ms`     | `5000`    | 最大重试延迟（毫秒）。                                                                                                  |
| `account_type`           | `Cash`    | 现货用 `Cash`，CFM 衍生品用 `Margin`。参见 [执行范围](#执行范围-execution-scope)。                                               |
| `default_margin_type`    | `None`    | 应用于衍生品订单的默认 `CoinbaseMarginType`（`Cross` 或 `Isolated`）。在 Cash 上被忽略。                                          |
| `default_leverage`       | `None`    | 应用于衍生品订单的默认杠杆。在 Cash 上被忽略。                                                                                   |
| `retail_portfolio_id`    | `None`    | CDP 零售投资组合 UUID。当 API 密钥绑定到非默认投资组合时必填（否则交易场所会以 `account is not available` 拒绝订单）。参见 [投资组合](#投资组合-portfolios)。 |
| `transport_backend`      | `Sockudo` | WebSocket 传输后端。                                                                                              |

配置通过 PyO3 导出的类型从 Python 构造：

```python
from nautilus_trader.adapters.coinbase import CoinbaseDataClientConfig
from nautilus_trader.adapters.coinbase import CoinbaseExecutionClientConfig
from nautilus_trader.adapters.coinbase import CoinbaseEnvironment

data_config = CoinbaseDataClientConfig(
    api_key="YOUR_COINBASE_API_KEY",
    api_secret="YOUR_COINBASE_API_SECRET",
    environment=CoinbaseEnvironment.LIVE,
)

exec_config = CoinbaseExecClientConfig(
    api_key="YOUR_COINBASE_API_KEY",
    api_secret="YOUR_COINBASE_API_SECRET",
    environment=CoinbaseEnvironment.LIVE,
)
```

v2 系统直接从这些配置实例化 Rust 工厂；无需 Python 工厂接线。

## 已知限制 (Known limitations)

### 交易场所侧

- 订单修改限于 GTC 订单（LIMIT、STOP_LIMIT、Bracket）；其他类型必须使用撤单-重发。
- OCO 订单未作为独立的订单类型暴露。
- 追踪止损、MARKET_IF_TOUCHED、LIMIT_IF_TOUCHED 和冰山订单未被交易场所暴露。
- 不提供批量提交和批量修改；只有批量取消。
- Sandbox 是一个静态模拟环境（仅 Accounts 和 Orders 端点，预定义响应，没有真实市场数据）。
- user 频道的 WebSocket 报告的是每个订单的累积状态，而非逐笔成交。执行客户端从累积增量推导
  每笔成交的数量、价格和佣金；逐笔的 `trade_id` 从 `(venue_order_id, cumulative_quantity)`
  合成。

### 适配器侧

- **每个客户端只支持一个产品族。** 提交、修改、撤单和报告生成都被过滤到配置的产品族
  （`AccountType::Cash` 下为现货；`AccountType::Margin` 下为永续 + 到期期货）。金融工具不在
  启动加载缓存内的订单会被拒绝。参见 [执行范围](#执行范围-execution-scope)。
- **Cash 账户的持仓报告始终为空。** Coinbase 现货没有持仓。衍生品（CFM）持仓报告来自
  `cfm/positions`，仅出现在 Margin 客户端上。
- **user 频道更新会省略 `price`、`stop_price` 和 `trigger_type`。** 对于本客户端提交的订单，
  缺失的字段会从 `submit_order` 时填充的缓存中修补。对于外部订单（由另一进程或通过 Coinbase
  UI 提交），user 频道处理器会在首次见到时通过获取 `/orders/historical/{venue_order_id}` 来
  丰富报告并缓存结果。该 REST 调用会给外部订单的第一条 user 频道更新增加延迟；后续更新使用
  缓存的丰富结果。
- **撤销全部和批量撤单的 REST 列表失败只会被记录日志。** 如果列出未成交订单的 REST 调用失败，
  不会发出任何单订单的 `OrderCancelRejected`；订单会保持 `PendingCancel`，直到下一次对账将其
  恢复。这与 Bybit 适配器的模式一致。
- **新上架的产品需要重连才能交易。** 金融工具缓存在连接时填充；之后上架的产品不在缓存中，
  `submit_order` 会拒绝它们。
- **MARKET 订单默认为 IOC。** 用 Nautilus 默认值 `TimeInForce::Gtc` 构造的 `MarketOrder`
  在交易场所会被映射为 `market_market_ioc`。显式的 `TimeInForce::Ioc` 会被遵守；
  `TimeInForce::Fok` 会路由到 `market_market_fok`，但在现货上会被撮合引擎在运行时以
  `UNSUPPORTED_ORDER_CONFIGURATION` 拒绝（其传输层结构在 API 规范中有记载，但仅在 CFM 衍生品
  上被接受）。`Day` 和 `Gtd` 会在提交时被拒绝。

## 认证二进制程序

两个二进制程序有助于实盘验证和账户卫生维护：

- `coinbase-http-private` 列出投资组合，打印钱包余额，针对 `BTC-USD` 和 `BTC-USDC` 运行
  `/orders/preview`，并暴露每个产品的门控标志。在让一个新账户上线时推荐先运行它。
- `coinbase-cancel-all-open` 取消认证 CDP 密钥上的每一个未成交订单。在测试运行之间用于清除
  挂单很有用。

两者都从环境中读取 `COINBASE_API_KEY` 和 `COINBASE_API_SECRET`。

## 贡献 (Contributing)

:::info
如需更多功能或为 Coinbase 适配器做出贡献，请参阅我们的
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
