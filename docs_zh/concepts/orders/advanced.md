# 高级订单 (Advanced orders)

下面这份指南应当结合 broker 或 venue 关于这些订单类型、订单列表/分组以及执行指令（例如 Interactive Brokers）的专门文档一起阅读。

## 订单列表 (Order lists)

多个或有订单 (contingent orders) 的组合，或更大批量的订单，可以分组到一个共享相同 `order_list_id` 的列表中。该列表所包含的订单之间可能有、也可能没有或有关系，这取决于订单本身是如何构造的，以及它们被路由到的具体 venue。

列表中的所有订单必须共享同一个 venue。订单可以指向该 venue 上不同的 instrument（例如配对交易、跨期价差、多腿合约的各条腿）；目标 venue 是否接受混合 instrument 的批量订单则因 venue 而异。列表的 `instrument_id` 取自第一个订单，作为代表值；需要逐订单 instrument 的下游消费方会单独解析每个订单。

混合 instrument 列表的注意事项：

- 盘前的逐订单检查（价格/数量精度、GTD）使用每个订单自身的 instrument。
- 累计风险检查（可用余额、最小/最大名义价值、减仓敞口、逐订单的市场数据查询）使用列表的代表 instrument。对于混合列表来说，这是一个单 instrument 的边界，而非逐 instrument 的精确值。
- 像 `cache.order_lists(instrument_id=...)` 这样的缓存查询会按代表 `instrument_id` 进行过滤；包含其他 instrument 的列表不会匹配针对那些其他 instrument 的查询。
- 当提供了 `position_id` 时，执行引擎会拒绝混合 instrument 的列表（一个 position 属于单一 instrument，与 OMS 无关）。
- 各 adapter 的 `submit_order_list` 实现各不相同。有些会逐腿遍历订单，并针对 venue API 解析每个订单自身的 `instrument_id`；另一些则仍围绕列表的代表 `instrument_id` 构建批量请求，从而会误路由非首个订单。请将混合 instrument 列表视为 adapter 相关的；在依赖它之前，先验证目标 adapter 的行为。今天最安全的路径，仍是在用户空间处理多腿路由的回测代码和自定义策略代码。

## 或有类型 (Contingency types)

- **OTO（One-Triggers-Other，一触发另一）** —— 一个父订单，一旦成交便自动下达一个或多个子订单。
  - *完全触发模型 (Full-trigger model)*：子订单**仅在父订单完全成交后**才被释放。常见于大多数零售股票/期权 broker（例如 Schwab、Fidelity、TD Ameritrade）以及许多现货加密 venue（Binance、Coinbase）。
  - *部分触发模型 (Partial-trigger model)*：子订单**按每次部分成交按比例 (pro-rata)** 释放。被专业级平台所采用，例如 Interactive Brokers、大多数期货/外汇 OMS，以及 Kraken Pro。

- **OCO（One-Cancels-Other，一撤销另一）** —— 两个（或多个）关联的活动订单，执行其中一个会撤销其余订单。

- **OUO（One-Updates-Other，一更新另一）** —— 两个（或多个）关联的活动订单，执行其中一个会减少其余订单的未成交数量。

:::info
这些或有类型对应 ContingencyType FIX tag <1385> <https://www.onixs.biz/fix-dictionary/5.0.sp2/tagnum_1385.html>。
:::

### One-Triggers-Other (OTO)

一个 OTO 订单包含两个部分：

1. **父订单 (Parent order)** —— 立即提交给撮合引擎。
2. **子订单 (Child order(s))** —— *挂在场外 (off-book)* 持有，直到触发条件满足。

#### 触发模型 (Trigger models)

| 触发模型                       | 子订单何时被释放？                                 |
| -------------------------- | ----------------------------------------- |
| **完全触发 (Full trigger)**    | 当父订单的累计数量等于其原始数量时（即它被*完全*成交）。             |
| **部分触发 (Partial trigger)** | 在父订单每次部分成交时立即触发；子订单的数量与已成交量相匹配，并随着后续成交增加。 |

:::info
NautilusTrader 的默认回测 venue 对 OTO 订单使用*部分触发模型*。
若要选用*完全触发模式*，请为该 venue 设置 `oto_trigger_mode="FULL"`（例如通过 `BacktestVenueConfig`）。
:::

**在生产中使用部分触发：**

如果你的策略需要完全触发语义，但 venue 或回测引擎使用的是部分触发：

1. 提交不带或有子订单的父订单。
2. 订阅父订单的 `OrderFilled` 事件。
3. 仅在确认父订单完全成交后，才提交子订单（止损、止盈）。
4. 使用 `order.is_closed` 和 `order.filled_qty == order.quantity` 来验证完全成交。

> **为何这一区别重要**
> *完全触发*会留下一个风险窗口：任何部分成交的 position 在剩余数量成交之前，都处于没有保护性退出的活动状态。
> *部分触发*缓解了这一风险，确保每一批已执行的手数立即拥有其关联的止损/限价，代价是产生更多的订单流量和更新。

一个 OTO 订单可以使用 venue 上任何受支持的资产类型（例如股票入场配期权对冲、期货入场配 OCO bracket、加密现货入场配 TP/SL）。

| Venue / Adapter ID                          | 资产类别        | 子订单的触发规则                 | 实务说明                                             |
| ------------------------------------------- | ----------- | ------------------------ | ------------------------------------------------ |
| Binance / Binance Futures (`BINANCE`)       | 现货、永续期货     | **部分或完全** —— 在首次成交时触发。   | OTOCO/TP-SL 子订单会立即出现；注意监控保证金使用。                  |
| Bybit Spot (`BYBIT`)                        | 现货          | **完全** —— 子订单在完成后下达。     | TP-SL 预设仅在限价订单完全成交后才激活。                          |
| Bybit Perps (`BYBIT`)                       | 永续期货        | **部分与完全** —— 可配置。        | “部分仓位 (Partial-position)”模式会随成交到来逐步调整 TP-SL 的规模。 |
| Kraken Futures (`KRAKEN`)                   | 期货与永续       | **部分与完全** —— 自动。         | 子订单数量与每次部分成交相匹配。                                 |
| OKX (`OKX`)                                 | 现货、期货、期权    | **完全** —— 附加的 stop 等待成交。 | 仓位级别的 TP-SL 可以单独添加。                              |
| Interactive Brokers (`INTERACTIVE_BROKERS`) | 股票、期权、外汇、期货 | **可配置** —— OCA 可按比例分配。   | `OcaType 2/3` 会减少剩余子订单的数量。                       |
| dYdX v4 (`DYDX`)                            | 永续期货 (DEX)  | 链上条件（规模精确）。              | TP-SL 由 oracle 价格触发；不适用部分成交。                     |
| Polymarket (`POLYMARKET`)                   | 预测市场 (DEX)  | 不适用。                     | 高级或有逻辑完全在策略层处理。                                  |
| Betfair (`BETFAIR`)                         | 体育博彩        | 不适用。                     | 高级或有逻辑完全在策略层处理。                                  |

### One-Cancels-Other (OCO)

一个 OCO 订单是一组关联订单，其中**任何**订单的执行（完全*或部分*）都会触发对其余订单的尽力 (best-efforts) 撤销。
两个订单同时处于活动状态；一旦其中一个开始成交，venue 就会尝试撤销其余订单的未成交部分。

### One-Updates-Other (OUO)

一个 OUO 订单是一组关联订单，其中一个订单的执行会导致其他订单的未成交数量立即*减少*。
两个订单并发处于活动状态，每次部分成交都会以尽力 (best-effort) 的方式按比例更新其对等订单的剩余数量。

## 或有订单校验 (Contingent order validation)

在使用或有订单（OTO、OCO、OUO）时，请注意以下校验规则和错误场景：

**订单列表要求：**

- 一个或有分组中的所有订单必须共享相同的 `order_list_id`。
- 父订单必须先于其子订单提交，或与其子订单同时提交。
- 子订单通过 `parent_order_id` 引用其父订单。

**修改规则：**

- 父订单在 pending 状态时通常可以被修改，但修改可能会级联到子订单。
- 在大多数 venue 上子订单可以被独立修改，但请检查 venue 特定的行为。
- 撤销父订单将撤销所有关联的子订单。

**常见错误场景：**

| 场景                | 系统行为                        |
| ----------------- | --------------------------- |
| 子订单引用了不存在的父订单     | 订单被拒绝并返回 `INVALID_ORDER` 错误 |
| 父订单在子订单触发之前被撤销    | 子订单被自动撤销                    |
| OCO 同级订单在撤销传播之前成交 | 部分成交被承认，剩余数量被撤销             |
| bracket 保证金不足     | 入场订单可能执行，子订单被单独拒绝           |

:::warning
请始终在你的策略中处理 `OrderDenied` 和 `OrderRejected` 事件，尤其是对于或有订单，因为部分失败可能会使 position 处于无保护状态。
:::

## bracket 订单 (Bracket orders)

bracket 订单是一种高级订单类型，允许交易者同时为一个 position 设置止盈和止损水平。它涉及下达一个父订单（入场订单）和两个子订单：一个止盈 `LIMIT` 订单和一个止损 `STOP_MARKET` 订单。当父订单执行时，系统会下达子订单。如果市场朝有利方向变动，止盈会平掉该 position；如果朝不利方向变动，止损则限制亏损。

bracket 订单可以使用 [OrderFactory](/docs/python-api-latest/common.html#nautilus_trader.common.factories.OrderFactory) 轻松创建，它支持各种订单类型、参数和指令。

在下面的示例中，我们为一个买入 10 张 ETHUSDT-PERP 合约的 *Market* 入场订单加上 bracket，止盈为 3,300 USDT 的 *Limit*，止损为在 2,800 USDT 触发的 *Stop-Market*。入场默认为 `MARKET`，止盈默认为 `LIMIT`，止损默认为 `STOP_MARKET`；止盈和止损这两条腿是 `reduce_only` 的，并通过 `OUO` 或有关系关联：

```rust tab="Rust"
use nautilus_model::{
    enums::OrderSide,
    identifiers::InstrumentId,
    types::{Price, Quantity},
};

// `bracket()` returns a `bon` builder; finalize with `.call()`.
// The result is a `Vec<OrderAny>` ordered as [entry, stop-loss, take-profit].
let orders = self
    .core
    .order_factory()
    .bracket()
    .instrument_id(InstrumentId::from("ETHUSDT-PERP.BINANCE"))
    .order_side(OrderSide::Buy)
    .quantity(Quantity::from(10))
    .tp_price(Price::from("3300.00"))         // take-profit LIMIT (default)
    .sl_trigger_price(Price::from("2800.00")) // stop-loss STOP_MARKET (default)
    .call();
```

```python tab="Python"
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Price
from nautilus_trader.model import Quantity
from nautilus_trader.model.orders import OrderList

bracket: OrderList = self.order_factory.bracket(
    instrument_id=InstrumentId.from_str("ETHUSDT-PERP.BINANCE"),
    order_side=OrderSide.BUY,
    quantity=Quantity.from_int(10),
    tp_price=Price.from_str("3300.00"),  # <-- take-profit LIMIT (default)
    sl_trigger_price=Price.from_str("2800.00"),  # <-- stop-loss STOP_MARKET (default)
)
```

:::warning
你应当注意 position 的保证金要求，因为为一个 position 加上 bracket 会消耗更多的订单保证金。
:::

## 相关指南 (Related guides)

- [订单 (Orders)](index.md) - 订单概念、执行指令以及 order factory。
- [模拟订单 (Emulated orders)](emulated.md) - 在不原生支持的 venue 上模拟订单类型。
- [执行 (Execution)](../execution.md) - 订单执行与成交处理。
