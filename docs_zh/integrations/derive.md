# Derive

Derive（前身为 Lyra）是一家去中心化衍生品交易场所，提供欧式期权和现金结算的永续合约（perpetual swaps），是规模最大的链上期权市场之一。交易针对 Derive Chain 上每个用户专属的智能合约钱包进行，因此抵押品始终保留在用户自己的托管之下，而订单则通过该场所的订单簿撮合。

Derive Chain 是一条结算到以太坊的乐观汇总（optimistic rollup）。订单在链下撮合、在链上结算，将订单簿执行与自托管结合在一起。订单通过 EIP-712 类型化数据签名进行授权，签名来自一个限定到某个子账户（subaccount）的会话密钥（session key），从而将签名密钥与钱包所有者分离，使用户可以在不转移资金的情况下轮换或撤销访问权限。

## 示例 (Examples)

Rust 示例测试器位于
[`crates/adapters/derive/examples/`](https://github.com/nautechsystems/nautilus_trader/tree/develop/crates/adapters/derive/examples/)。

## 概览 (Overview)

Derive 适配器以 Rust 实现，位于 `crates/adapters/derive`。它对外暴露：

- `DeriveHttpClient`：到 `api.lyra.finance`（mainnet）或 `api-demo.lyra.finance`（testnet）的低层 REST 连接。
- `DeriveWebSocketClient`：JSON-RPC WebSocket 传输，带订阅跟踪、重连，以及签名下单（即 WebSocket Trading API）。
- `DeriveInstrumentProvider`：按币种获取并缓存合约（instrument）。
- `DeriveDataClient`：实时市场数据客户端。
- `DeriveDataClientFactory`：用于 live node builder 的数据客户端工厂。
- `DeriveExecutionClient`：实时执行客户端，负责签名下单、撤单、查询和报告流程。
- `DeriveExecutionClientFactory`：用于 live node builder 的执行客户端工厂。

执行流程使用针对 Derive Chain 各操作模块合约的 EIP-712 类型化数据签名。

## Derive 文档 (Derive documentation)

Derive 在 [docs.derive.xyz](https://docs.derive.xyz) 发布 API 文档。请将其与本指南配合阅读以获取更多细节。

## 产品 (Products)

| 产品类型        | 是否支持 | 说明                                                |
| ----------- | ---- | ------------------------------------------------- |
| ERC-20 现货   | ✓    | 以 USDC 计价的交易对，如 `ETH-USDC`；解析为 `CurrencyPair`。    |
| 永续合约        | ✓    | 以 USDC 现金结算，按币种挂牌，如 `ETH-PERP`。                   |
| 期权（看涨 / 看跌） | ✓    | 欧式期权，采用 `{CURRENCY}-{EXPIRY}-{STRIKE}-{C\|P}` 格式。 |

## 符号体系 (Symbology)

Derive 合约使用场所原生符号加上场所后缀 `.DERIVE`：

- 现货：`ETH-USDC.DERIVE`（基础货币、计价货币）。
- 永续：`ETH-PERP.DERIVE`、`BTC-PERP.DERIVE`。
- 期权：`ETH-20260626-3000-C.DERIVE`（币种、到期日、行权价、类型）。

符号中以连字符分隔的第一段是底层币种。provider 对每个币种只调用一次 `public/get_instruments`，因此当启用 `auto_load_missing_instruments`（默认启用）时，订阅新币种会触发一次惰性 REST 拉取。

适配器根据场所的 `instrument_type`（`perp`、`option`、`erc20`）进行路由，而非根据符号后缀，因此现货交易对无需特殊的符号解析。现货复用与永续和期权相同的 Trade 模块签名路径；仓库内 `crates/adapters/derive/test_data/spot/` 下的 fixtures 捕获了现货的合约、订单簿、ticker 和成交字段形态，解析与执行路径都以它们为基准锚定。

:::warning
现货交易的实盘运行经验少于永续和期权。Testnet 能以 `0.1 ETH` 最小数量接受并撤销一个被动的 `ETH-USDC` 限价单，mainnet 的下单 / 撤单也已手动验证过。公开现货成交频道（`trades.erc20.ETH`、`trades.ETH-USDC`）可以成功订阅，但成交量可能较低，因此预期会收到稀疏的成交帧。
:::

## 环境 (Environments)

通过任一客户端配置上的 `DeriveEnvironment` 枚举来配置环境。

| 环境      | 配置                           | REST                            | WebSocket                        |
| ------- | ---------------------------- | ------------------------------- | -------------------------------- |
| Mainnet | `DeriveEnvironment::Mainnet` | `https://api.lyra.finance`      | `wss://api.lyra.finance/ws`      |
| Testnet | `DeriveEnvironment::Testnet` | `https://api-demo.lyra.finance` | `wss://api-demo.lyra.finance/ws` |

Testnet 是一条独立的链，拥有自己的会话密钥和余额；mainnet 与 testnet 的 API 密钥不能互换。公开市场数据（订单簿、ticker、成交）不需要凭据。

两个网络的 EIP-712 协议常量（`DOMAIN_SEPARATOR`、`ACTION_TYPEHASH`、各操作模块地址）随代码一同提供在 `crates/adapters/derive/src/common/consts.rs` 中，并对照 Derive 的 [协议常量参考](https://docs.derive.xyz/reference/protocol-constants) 进行跟踪。`DeriveExecClientConfig::domain_separator`、`action_typehash` 和 `trade_module_address` 接受按实例的覆盖值，其优先级高于随代码提供的默认值。

## Testnet 接入 (Testnet onboarding)

Derive 在 Web 应用中将演示环境标注为 “testnet”，而在 API 主机名中标注为 “demo”。本指南统一使用 “testnet”，以与仪表盘以及我们的 `DeriveEnvironment::Testnet` 枚举保持一致。要使执行客户端能够提交一笔签名订单，需完成以下步骤：

1. **登录 testnet 仪表盘。** 打开 [testnet.derive.xyz](https://testnet.derive.xyz) 并连接一个 EVM 钱包（MetaMask、WalletConnect、社交登录等）。这是用于授权下文智能合约钱包的所有者 EOA。
2. **注册 Derive Chain 智能合约钱包。** 首次登录会在 Derive testnet 链上部署一个用户专属的智能合约钱包。在 “Developers” -> “Derive Wallet” 下显示的地址就是客户端使用的 `wallet_address`（也是 `X-LYRAWALLET` 请求头）。它与你刚刚连接的 EOA 不同。
3. **创建子账户。** 在钱包下开设一个子账户（Standard Margin 是最简单的测试交易模式）。这个整数 id 就是客户端为每个 `private/order` 请求签名所针对的 `subaccount_id`。
4. **生成会话密钥。** 在 “Developers” -> “Session Keys” 下，创建一个限定到该子账户的会话密钥，并复制原始 secp256k1 私钥。这就是 `session_key` 的值；它绝不离开客户端，并会在 `Debug` 输出中被脱敏。会话密钥可以在同一面板中轮换或撤销。
5. **通过水龙头为子账户充值。** Testnet 仪表盘提供一个 USDC 水龙头，可滴注测试用抵押品。向子账户充值，使其链上余额显示非零抵押品；在子账户拥有足够保证金覆盖所请求的下单规模之前，API 会拒绝订单。
6. **设置环境变量。** 导出客户端在 testnet 模式下读取的三个值（或在 `DeriveExecClientConfig` 上传入，此时配置字段优先）：

   ```fish
   set -gx DERIVE_TESTNET_WALLET_ADDRESS      "0x..."   # Derive Chain smart-contract wallet
   set -gx DERIVE_TESTNET_SESSION_PRIVATE_KEY "0x..."   # secp256k1 session-key private key
   set -gx DERIVE_TESTNET_SUBACCOUNT_ID       "12345"   # integer subaccount id
   ```

### 最小资金 (Minimum funding)

场所没有固定的最低额度。撮合引擎接受任何能满足子账户对结果持仓的初始保证金要求的订单。请将下列数额视为进行最小可行测试的实用下限：

- **冒烟测试（下单并撤单，不成交）：** 任何正的 USDC 余额都足以覆盖签名下单的链路。
- **完成一笔 `ETH-PERP` 成交往返：** 按最坏情况下的滑点调整名义价值加上初始保证金缓冲来预算。以 $3500 的一张合约和场所约 10% 的 IM 计算，大约需要 $350 抵押品加 $400 缓冲。约 $1000 USDC 是首笔成交测试比较从容的可用余额。
- **期权：** 期权的 IM 高于永续。请为该期权拉取 `public/get_instrument`，将合约规模乘以标记价格，再加上期权专属的 IM（可在合约响应中查看），然后据此确定充值规模。

充值后请使用 `private/get_subaccount` 端点，对照你计划提交的订单确认 `initial_margin`/`maintenance_margin` 的余量；适配器的 `query_account` 命令会将这一快照作为 `AccountState` 事件发出，使策略层可以据此对交易进行门控。

## Mainnet 接入 (Mainnet onboarding)

Mainnet 接入与 testnet 类似，但针对生产仪表盘进行。使用真实资金。

1. **登录 mainnet 仪表盘。** 打开 [derive.xyz](https://derive.xyz) 并连接 EVM 所有者钱包（MetaMask、WalletConnect、社交登录等）。首次登录会部署你的 Derive Chain 智能合约钱包。
2. **复制钱包地址。** 在 “Developers” -> “Derive Wallet” 下，复制智能合约钱包地址。这就是客户端签名所针对的 `wallet_address`；它与你登录时使用的 EOA **不同**。请在 Derive Chain 浏览器上验证该地址确实带有合约代码（EOA 不带）。
3. **创建或选择子账户。** 在钱包下开设一个子账户（Standard Margin 是最简单的模式；只有在你理解了跨保证金语义之后，才切换到 Portfolio Margin）。这个整数 id 就是 `subaccount_id`。
4. **生成 mainnet 会话密钥。** 在 “Developers” -> “Session Keys” 下，创建一个限定到该子账户的会话密钥，并复制原始 secp256k1 私钥。会话密钥可以在同一面板中轮换或撤销；对探索性的测试器运行，建议使用短期密钥。
5. **为子账户充值。** 通过仪表盘的充值流程向子账户存入 USDC（或受支持的抵押品）。提交订单前，请通过 `private/get_subaccount`（或适配器的 `query_account`）确认 `collaterals_value` 和 `initial_margin` 的余量足以覆盖预期订单。
6. **设置环境变量。** 导出三个 mainnet 值（或在 `DeriveExecClientConfig` 上传入，此时配置字段优先）：

   ```fish
   set -gx DERIVE_WALLET_ADDRESS      "0x..."   # Derive Chain smart-contract wallet
   set -gx DERIVE_SESSION_PRIVATE_KEY "0x..."   # secp256k1 session-key private key
   set -gx DERIVE_SUBACCOUNT_ID       "12345"   # integer subaccount id
   ```

   `node_exec_tester` 示例固定为 `DeriveEnvironment::Testnet`；进行真实资金运行时，请将该字面量改为 `DeriveEnvironment::Mainnet`。`node_data_tester` 和 `node_delta_neutral` 示例默认使用 testnet，并读取 `DERIVE_ENVIRONMENT=mainnet` 来切换。生产部署通过 `DeriveDataClientConfig::environment` / `DeriveExecClientConfig::environment` 选择网络。

## 能力 (Capabilities)

### 市场数据 (Market data)

| 能力                  | 是否支持 | 说明                                                     |
| ------------------- | ---- | ------------------------------------------------------ |
| 请求单个合约（REST）        | ✓    | `public/get_instrument`；将一个合约加载到本地缓存。                  |
| 请求全部合约（REST）        | ✓    | `public/get_instruments`；为 `currencies` 中的每个币种拉取。      |
| 合约订阅                | -    | *不支持。* 改用所配置的 REST 刷新间隔。                               |
| 订单簿增量（L2_MBP）       | ✓    | 频道：`orderbook.{instrument}.{group}.{depth}`。           |
| 订单簿 depth10（L2_MBP） | ✓    | 同一订单簿频道，使用 `depth=10`。                                 |
| 按固定间隔的订单簿           | -    | *不支持。* 在本地从增量维护按间隔的订单簿。                                |
| 订单簿快照（REST）         | -    | *不支持。* 适配器未暴露。                                         |
| 历史订单簿增量（REST）       | -    | *不支持。* 适配器未暴露。                                         |
| 报价（`ticker_slim`）   | ✓    | 频道：`ticker_slim.{instrument}.{interval}`。              |
| 报价快照（REST）          | ✓    | 一次性的 `public/get_tickers`；发出单个 `QuoteTick`。            |
| 历史报价（REST）          | -    | *不支持。* 场所仅暴露 ticker 快照。                                |
| 成交                  | ✓    | 频道：`trades.{instrument_type}.{currency}`。              |
| 历史成交（REST）          | ✓    | `public/get_trade_history`；遵循 `start`、`end` 和 `limit`。 |
| K 线 / OHLC（REST）    | ✓    | `public/get_tradingview_chart_data`；分钟、小时、日和周线。        |
| K 线 / OHLC（WS）      | -    | *不支持。* 场所没有 K 线订阅频道。                                   |
| 标记价格流               | ✓    | 从 `ticker_slim` 派生；与报价订阅共享。                            |
| 指数价格流               | ✓    | 从 `ticker_slim` 派生；与报价订阅共享。                            |
| 资金费率流               | ✓    | 从永续 ticker 上的 `perp_details.funding_rate` 派生。          |
| 资金费率历史（REST）        | ✓    | 永续合约的 `public/get_funding_rate_history`。               |
| 合约状态                | -    | *不支持。* Ticker 载荷包含 `is_active`。                        |
| 合约收盘                | -    | *不支持。* 期权结算仅通过 REST。                                   |
| 期权希腊值               | ✓    | 从期权 ticker 上的 `option_pricing` 派生。                     |
| 期权链                 | ✓    | 由报价和希腊值聚合；`public/get_tickers` 引导出平值（ATM）。             |

`request_instrument` 针对所请求的 `InstrumentId` 调用 `public/get_instrument`，并在发出响应之前缓存返回的定义。缓存的合约携带了后续报价、成交、订单簿和 K 线解析所使用的精度和最小变动单位字段。

Derive 通过同一个 `orderbook.{instrument}.{group}.{depth}` 频道族暴露订单簿增量和 depth10 快照。`subscribe_book_deltas` 将快照增量作为 `OrderBookDeltas` 发布，而 `subscribe_book_depth10` 固定 `depth=10` 并发布 `OrderBookDepth10` 快照。

### 执行 (Execution)

下单、撤单、改单、查询和报告生成都使用 Derive 的 EIP-712 自托管签名流程。下单写操作（`private/order`、`private/cancel`、`private/cancel_all`、`private/replace`）通过 WebSocket Trading API 在同一个已认证的会话上进行，该会话同时通过私有频道（`{subaccount_id}.orders`、`{subaccount_id}.trades`、`{subaccount_id}.balances`）流式推送账户、订单、成交和余额状态。无论使用哪种传输，签名后的 EIP-712 报文体都是完全相同的。

:::note
HTTP 下单端点在 `DeriveHttpClient` 上仍然可用，供工具和测试使用，但实时执行客户端会将所有写操作经由 WebSocket Trading API 路由。报告生成、账户刷新和合约查询仍使用 REST。
:::

永续、期权和 ERC-20 现货对都使用 Derive Trade 模块。现货没有单独的签名路径，除下文描述的 reduce-only 保护外，对账（reconciliation）对待现货合约与对待其他合约类别相同。

适配器支持普通的 `private/order` 请求：`LIMIT` 和 `MARKET` 订单，可使用 `GTC`、`IOC` 或 `FOK` 的有效期（time-in-force）取值。它还为下列 Nautilus 原生的 stop 和 if-touched 订单类型支持 Derive 触发订单。不受支持的 Nautilus 订单类型会在签名之前被拒绝，因此它们不会在场所成交。

市价单在提交前需要一份已缓存的报价。在异步提交任务解析出合约之后，它会刷新当前的 ticker 快照，并据此刷新后的报价派生出已签名的、带滑点边界的 `limit_price`。

#### 条件订单 (Conditional orders)

Derive 触发订单使用仅限 WebSocket 的 `private/trigger_order` 端点，而非普通的 `private/order` 端点。场所会以 `order_status=untriggered` 状态存储它们，直到其触发器 worker 提交已签名的子订单。因此对账会同时读取 `private/get_open_orders` 和 `private/get_trigger_orders`。

Derive mainnet 要求触发订单的签名在场所时间起 30 至 90 天后过期。适配器以固定的 31 天有效期为触发订单签名；`signature_expiry_secs` 仍然控制普通的 `private/order` 和 `private/replace` 写操作，且必须大于场所规定的 300 秒最小值。

| Nautilus 订单类型     | 是否支持 | Derive `order_type` | Derive `trigger_type` | 说明            |
| ----------------- | ---- | ------------------- | --------------------- | ------------- |
| `StopMarket`      | ✓    | `market`            | `stoploss`            | 使用触发价格作为边界。   |
| `StopLimit`       | ✓    | `limit`             | `stoploss`            | 发送限价和触发价格。    |
| `MarketIfTouched` | ✓    | `market`            | `takeprofit`          | 使用触发价格作为边界。   |
| `LimitIfTouched`  | ✓    | `limit`             | `takeprofit`          | 发送限价和触发价格。    |
| `MarketToLimit`   | -    | -                   | -                     | *Derive 不支持*。 |
| 追踪止损              | -    | -                   | -                     | *Derive 不支持*。 |
| TWAP / algo / RFQ | -    | -                   | -                     | *本适配器未暴露*。    |

适配器将 Nautilus 的 `TriggerType::Default` 和 `TriggerType::MarkPrice` 映射为 Derive 的 `trigger_price_type=mark`。Derive 当前的错误码参考说明指数价格和最新成交价（last-trade）触发价格类型尚不受支持，因此 `IndexPrice`、`LastPrice`、`BidAsk` 以及其他触发价格类型会在签名之前于本地被拒绝。

Derive 错误 `11054` 说明触发订单不能替换其他订单，也不能被替换。因此适配器会以 `OrderModifyRejected` 事件拒绝针对触发订单的 Nautilus 改单请求；触发订单的更新请通过撤单并重新提交完成。

#### 执行指令 (Execution instructions)

| 指令            | 是否支持 | Derive 值      | 说明                     |
| ------------- | ---- | ------------- | ---------------------- |
| `post_only`   | ✓    | `post_only`   | 需要 `GTC`；若订单会吃掉流动性则拒绝。 |
| `reduce_only` | ✓    | `reduce_only` | 永续和期权支持。现货在本地被拒绝。      |

#### 有效期 (Time in force)

Derive 将 `gtc`、`post_only`、`fok` 和 `ioc` 记录为其 `time_in_force` 取值。适配器会在签名之前拒绝没有 Derive 对应项的 Nautilus 取值。Derive 将 post-only 作为一个 `time_in_force` 取值暴露，因此 `post_only` 不能与 `IOC` 或 `FOK` 组合使用。

| 有效期            | 是否支持 | Derive 值 | 说明            |
| -------------- | ---- | -------- | ------------- |
| `GTC`          | ✓    | `gtc`    | 撤单前一直有效。      |
| `IOC`          | ✓    | `ioc`    | 立即成交否则取消。     |
| `FOK`          | ✓    | `fok`    | 全部成交否则取消。     |
| `GTD`          | -    | -        | *Derive 不支持*。 |
| `DAY`          | -    | -        | *Derive 不支持*。 |
| `AT_THE_OPEN`  | -    | -        | *Derive 不支持*。 |
| `AT_THE_CLOSE` | -    | -        | *Derive 不支持*。 |

#### 现货 reduce-only 订单 (Spot reduce-only orders)

Derive 现货没有持仓（position）概念，因此 reduce-only 现货订单永远无法减少任何东西。场所总是以错误 `11025` 拒绝它；当适配器已知合约为现货时，会避免这一次往返。已缓存的现货合约会以 `OrderDenied` 被拒绝；惰性解析得到的现货合约则在提交时以 `OrderRejected` 被拒绝。

针对永续和期权的 reduce-only 订单仍会到达场所，结果取决于子账户的持仓状态。`derive-flatten` 程序只平掉衍生品持仓，绝不平现货，因为平掉现货余额会把基础资产倾倒进一个不同的计价货币。

#### 订单拒绝语义 (Order rejection semantics)

改变状态的写操作（`submit_order`、`modify_order`、`cancel_order`）通过 WebSocket Trading API 只发送一次，不会重放。适配器依据 WebSocket 请求的结果来区分终态（terminal）与歧义（ambiguous）处理。对于明确的场所失败，它会发出一个终态拒绝事件（`OrderRejected`、`OrderModifyRejected`、`OrderCancelRejected`）：

- 签名操作被拒，例如参数无效、保证金不足或订单未知。
- 场所业务码，例如 `11009 Zero liquidity`。
- post-only 穿越拒绝（`11008 Post only order cannot cross the market`），报告为带 `due_post_only=true` 的 `OrderRejected`。
- 限速响应（`-32000 Rate limit exceeded`），此时网关在撮合引擎看到请求之前就拒绝了它。

对于到达场所的 post-only 订单，Derive 会以 JSON-RPC `11008` 和消息 `Post only order cannot cross the market` 拒绝一个会穿越市场的订单。适配器将该终态拒绝标记为 `due_post_only=true`；如果某个 WebSocket/订单报告拒绝携带了相同的原因，被跟踪的订单路径会应用相同的分类。针对不受支持的 post-only IOC/FOK 组合的本地拒绝不会被标记为 `due_post_only`，因为它们并不代表一次场所穿越拒绝。

对于歧义的写操作结果，适配器不发出任何终态事件，而是让 WebSocket 对账或之后的状态报告来确定状态。歧义集合被刻意限定得很窄：

- `-32603`，一个通用的 JSON-RPC 内部错误。
- 无法解码的响应（该操作可能已经被处理）。
- 请求超时、重连时丢失的响应，以及传输错误。

这一区分保护了订单生命周期的两端。一次错误的终态拒绝会让引擎把一个仍然有效的订单当作已拒绝；而一次错误的歧义结果则可能让一个未成功下出的订单永远悬挂在 `Submitted` 状态，因为不会有任何 WebSocket 帧到达。

## 订阅参数 (Subscription parameters)

`subscribe_book_deltas` 和 `subscribe_book_depth10` 接受下列 `subscribe_params` 键：

| 键       | 类型     | 默认值    | 允许值                         |
| ------- | ------ | ------ | --------------------------- |
| `group` | string | `"1"`  | `"1"`、`"10"`、`"100"`        |
| `depth` | string | `"10"` | `"1"`、`"10"`、`"20"`、`"100"` |

`subscribe_quotes` 接受：

| 键          | 类型     | 默认值      | 允许值              |
| ---------- | ------ | -------- | ---------------- |
| `interval` | string | `"1000"` | `"100"`、`"1000"` |

未知取值会在订阅时被拒绝。

### 共享 ticker 订阅 (Shared ticker subscription)

报价、标记价格、指数价格、资金费率和期权希腊值全部派生自同一个 `ticker_slim.{instrument}.{interval}` WebSocket 订阅。适配器对底层的 WS 订阅调用进行引用计数：某个合约上第一个被订阅的数据流会打开该频道，最后一次取消订阅会关闭它。因此，第一次订阅时给出的 `interval` 会胜出；后续以不同 `interval` 订阅的数据流共享既有频道。

标记价格、指数价格、资金费率和期权希腊值读取的都是场所包含在完整 ticker 载荷中的字段（`mark_price`、`index_price`、`perp_details.funding_rate`、`option_pricing`）。已观察到的 Derive 在 `ticker_slim` 上的推送携带了这些字段，因此这些派生数据流可以正常工作。如果场所今后在该频道上推送紧凑的 `SlimEnvelope` 形态，那些派生数据流将对该帧静默地不产生任何数据；而报价数据流仍然有效，因为两种形态中都存在买卖价（bid/ask）。

资金费率只对永续有意义，期权希腊值只对期权有意义。为某个合约的类别订阅错误的数据流（例如为期权订阅资金费率）会被接受，WebSocket 订阅也会打开，但解析器不会为该数据流返回任何事件，因为场所载荷缺少相关字段（非永续缺少 `perp_details`，非期权缺少 `option_pricing`）。在订阅衍生品专属的数据流之前，请先核实合约类别。

## 配置 (Configuration)

### 数据客户端配置选项 (Data client configuration options)

类/结构体：`DeriveDataClientConfig`。

| 选项                                 | 默认值       | 说明                                        |
| ---------------------------------- | --------- | ----------------------------------------- |
| `base_url_rest`                    | `None`    | 覆盖 REST 基础 URL。                           |
| `base_url_ws`                      | `None`    | 覆盖 WebSocket 基础 URL。                      |
| `proxy_url`                        | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。             |
| `environment`                      | `Mainnet` | 网络选择器（在 Python 中为 `MAINNET` 或 `TESTNET`）。 |
| `http_timeout_secs`                | `10`      | REST 请求超时（秒）。                             |
| `ws_timeout_secs`                  | `30`      | WebSocket 连接和空闲超时（秒）。                     |
| `update_instruments_interval_mins` | `60`      | 合约刷新之间的间隔（分钟）。                            |
| `currencies`                       | `[]`      | 连接时批量加载的币种。为空表示按需惰性加载。                    |
| `include_expired`                  | `false`   | 在 `public/get_instruments` 中包含已过期的期权行。    |
| `auto_load_missing_instruments`    | `true`    | 在 subscribe 或 request 命令之前惰性加载未知合约。       |
| `transport_backend`                | `Sockudo` | 启用 `transport-sockudo` 时使用的 WebSocket 传输。 |

### 执行客户端配置选项 (Execution client configuration options)

类/结构体：`DeriveExecClientConfig`。

| 选项                          | 默认值       | 说明                                            |
| --------------------------- | --------- | --------------------------------------------- |
| `wallet_address`            | `None`    | Derive Chain 智能合约钱包地址。回退到下方的环境变量。             |
| `session_key`               | `None`    | secp256k1 会话密钥私钥。回退到下方的环境变量。                  |
| `subaccount_id`             | `None`    | Derive 子账户 id。回退到下方的环境变量。                     |
| `base_url_rest`             | `None`    | 覆盖 REST 基础 URL。                               |
| `base_url_ws`               | `None`    | 覆盖 WebSocket 基础 URL。                          |
| `proxy_url`                 | `None`    | HTTP 和 WebSocket 传输的可选代理 URL。                 |
| `environment`               | `Mainnet` | 网络选择器（在 Python 中为 `MAINNET` 或 `TESTNET`）。     |
| `http_timeout_secs`         | `10`      | REST 请求超时（秒）。                                 |
| `max_retries`               | `3`       | 对可恢复的读操作和明确的非写路径的重试次数。                        |
| `retry_delay_initial_ms`    | `100`     | 初始重试延迟（毫秒）。                                   |
| `retry_delay_max_ms`        | `5000`    | 最大重试延迟（毫秒）。                                   |
| `max_fee_per_contract`      | `None`    | 签入每笔订单的每合约 USDC 费用上限。                         |
| `domain_separator`          | `None`    | 可选的 EIP-712 domain separator 覆盖值。             |
| `action_typehash`           | `None`    | 可选的 EIP-712 action typehash 覆盖值。              |
| `trade_module_address`      | `None`    | 可选的 Trade 模块合约地址覆盖值。                          |
| `signature_expiry_secs`     | `600`     | 订单/replace 的 TTL；必须 >300s。触发订单使用固定的 31 天 TTL。 |
| `market_order_slippage_bps` | `50`      | 市价单限价的滑点边界。                                   |
| `transport_backend`         | `Sockudo` | 启用 `transport-sockudo` 时使用的 WebSocket 传输。     |

当构建禁用了 `transport-sockudo` 特性时，默认传输回退到 `Tungstenite`。

当未设置时，`wallet_address`、`session_key` 和 `subaccount_id` 会回退到环境变量：

| 字段               | Mainnet 变量                   | Testnet 变量                           |
| ---------------- | ---------------------------- | ------------------------------------ |
| `wallet_address` | `DERIVE_WALLET_ADDRESS`      | `DERIVE_TESTNET_WALLET_ADDRESS`      |
| `session_key`    | `DERIVE_SESSION_PRIVATE_KEY` | `DERIVE_TESTNET_SESSION_PRIVATE_KEY` |
| `subaccount_id`  | `DERIVE_SUBACCOUNT_ID`       | `DERIVE_TESTNET_SUBACCOUNT_ID`       |

会话密钥是注册到钱包上用于 API 签名的 secp256k1 私钥。`session_key` 字段在 `Debug` 输出和 Python `repr` 中会被脱敏。

### Python v2 live node

由 Rust 支撑的 Python v2 节点使用 `LiveNode.builder(...)` 并传入具体的工厂实例。执行工厂需要 `DeriveExecFactoryConfig`，它将 trader 和账户标识符与底层的 `DeriveExecClientConfig` 包装在一起。

```python
from decimal import Decimal

from nautilus_trader.adapters.derive import DeriveDataClientConfig
from nautilus_trader.adapters.derive import DeriveDataClientFactory
from nautilus_trader.adapters.derive import DeriveEnvironment
from nautilus_trader.adapters.derive import DeriveExecClientConfig
from nautilus_trader.adapters.derive import DeriveExecFactoryConfig
from nautilus_trader.adapters.derive import DeriveExecutionClientFactory
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveNode
from nautilus_trader.model import AccountId
from nautilus_trader.model import TraderId

trader_id = TraderId("TESTER-001")

data_config = DeriveDataClientConfig(
    environment=DeriveEnvironment.TESTNET,
    currencies=["ETH", "BTC"],
)

exec_config = DeriveExecClientConfig(
    environment=DeriveEnvironment.TESTNET,
    max_fee_per_contract=Decimal("1000"),
)

exec_factory_config = DeriveExecFactoryConfig(
    trader_id,
    AccountId("DERIVE-001"),
    exec_config,
)

node = (
    LiveNode.builder("DERIVE-001", trader_id, Environment.LIVE)
    .add_data_client(None, DeriveDataClientFactory(), data_config)
    .add_exec_client(None, DeriveExecutionClientFactory(), exec_factory_config)
    .build()
)
```

不要将 `DeriveExecClientConfig` 直接传给 `add_exec_client`；Derive 执行工厂需要包装后的 `DeriveExecFactoryConfig`，这样它才能用正确的 trader 和账户标识符创建 `ExecutionClientCore`。

### Rust 数据客户端 (Rust data client)

```rust
use nautilus_derive::{
    common::enums::DeriveEnvironment,
    config::DeriveDataClientConfig,
};

let config = DeriveDataClientConfig {
    environment: DeriveEnvironment::Testnet,
    currencies: vec!["ETH".to_string(), "BTC".to_string()],
    ..Default::default()
};
```

值得注意的字段：

- `currencies`：连接时要批量加载哪些币种。为空表示按订阅惰性加载。
- `include_expired`：在 `public/get_instruments` 中包含已过期的期权行。
- `auto_load_missing_instruments`：当某个合约未知时，在订阅时惰性加载。
- `update_instruments_interval_mins`：REST 刷新间隔（默认 60 分钟）。
- `http_timeout_secs`、`ws_timeout_secs`：传输超时。

### Rust 执行客户端 (Rust execution client)

```rust
use nautilus_derive::{
    common::enums::DeriveEnvironment,
    config::DeriveExecClientConfig,
};

let config = DeriveExecClientConfig {
    wallet_address: Some("0x...".to_string()),
    session_key: Some("0x...".to_string()),
    subaccount_id: Some(1),
    environment: DeriveEnvironment::Testnet,
    ..Default::default()
};
```

## 已知限制 (Known limitations)

- `request_instruments` 要求在 `DeriveDataClientConfig::currencies` 中至少配置一个币种；场所的 `public/get_instruments` 端点按币种限定范围，而适配器不会枚举整个币种集合。
- `data_client.rs` 的集成测试断言所记录 REST 调用的集合，而非顺序，因为 `fetch_instrument_definitions` 通过 `tokio::try_join!` 并行发起 `perp` 和 `option` 请求。
- 场所不推送合约状态、合约收盘或 K 线订阅；ticker 载荷携带保证金参数和 `is_active`，而 K 线仅通过 REST 提供。
- 订单簿快照 REST 端点以及历史订单簿增量 / 历史报价端点未被场所暴露。参见上方的能力表。
- 自 2025 年 12 月 1 日起，Derive 官方 REST 文档将 `public/get_ticker` 标记为已弃用，转而推荐 `public/get_tickers`。适配器使用 `public/get_tickers` 进行报价快照和期权链的远期价格引导。
