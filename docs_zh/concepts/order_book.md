# 订单簿 (Order Book)

NautilusTrader 提供了一个用 Rust 实现的高性能订单簿，能够基于 L1 到 L3 数据维护完整的订单簿状态。`OrderBook` 是追踪公开市场深度的核心组件，而 `OwnOrderBook` 则单独追踪你自己的订单，从而可以生成过滤后的视图，展示真实可用的流动性。

:::note
本指南记录的是 Rust 模型 API，订阅与处理器示例用的是 Python 策略与 actor API。Python 侧以 `nautilus_trader.model.OrderBook` 与 `nautilus_trader.model.OwnOrderBook` 暴露这两个类型；其 Python 接口见 model API 参考。
:::

## 订单簿类型

无论是回测还是实盘交易，都会为每个金融工具维护一个 `OrderBook` 实例：

- `L3_MBO`：**按订单的市场行情 (Market by order)** 数据。追踪每个价位上的每一笔订单，以订单 ID 作为键。
- `L2_MBP`：**按价格的市场行情 (Market by price)** 数据。按价位聚合订单（每个价位一条记录）。
- `L1_MBP`：**盘口 (Top-of-book)** 数据，也称为最优买卖价 (BBO)。仅捕获最优价格。

:::note
诸如 `QuoteTick`、`TradeTick` 和 `Bar` 这样的盘口数据也可以维护 `L1_MBP` 订单簿。
:::

## 订阅订单簿数据

策略和 actor 通过以下方法订阅订单簿更新。订阅和处理器属于 Python 策略/actor 层：

```python
# L3/L2 增量变化（deltas）
self.subscribe_order_book_deltas(instrument_id)

# 聚合深度快照（最多 10 档）
self.subscribe_order_book_depth(instrument_id)

# 按定时间隔推送的完整订单簿快照
self.subscribe_order_book_at_interval(instrument_id, interval_ms=1000)
```

每种订阅类型会将数据传递给对应的处理器：

```python
def on_order_book_deltas(self, deltas: OrderBookDeltas) -> None: ...


def on_order_book_depth(self, depth: OrderBookDepth10) -> None: ...


def on_order_book(self, order_book: OrderBook) -> None: ...
```

## 访问订单簿

`OrderBook` 暴露了盘口访问器：

```rust
let best_bid: Option<Price> = book.best_bid_price();
let best_ask: Option<Price> = book.best_ask_price();
let spread: Option<f64> = book.spread();
let midpoint: Option<f64> = book.midpoint();
```

## 分析方法

`OrderBook` 支持市场深度分析和成交模拟：

```rust
// 给定数量的平均成交价格
let avg_px = book.get_avg_px_for_quantity(quantity, OrderSide::Buy);

// 达到目标敞口（名义金额）所需的平均价格和数量
let (price, qty, exposure) =
    book.get_avg_px_qty_for_exposure(target_exposure, OrderSide::Buy);

// 在某价格或更优价格上可用的累计数量
let qty = book.get_quantity_for_price(price, OrderSide::Buy);

// 仅在特定价位上的数量
let qty = book.get_quantity_at_level(price, OrderSide::Buy, 2);

// 针对订单簿模拟成交
let fills: Vec<(Price, Quantity)> = book.simulate_fills(&order);

// 无视订单数量、列出所有交叉的价位
let levels = book.get_all_crossed_levels(OrderSide::Buy, price, 2);
```

## 完整性检查

`book_check_integrity` 函数验证订单簿状态与其类型保持一致：

- **L1_MBP**：每一侧不超过一档。
- **L2_MBP**：每个价位不超过一笔订单。
- **L3_MBO**：无结构性约束（任意价位可有任意数量的订单）。
- **所有类型**：最优买价不得超过最优卖价（即交叉订单簿）。锁定市场（买价 == 卖价）被视为有效。

这些检查在应用增量变化（delta）的过程中于内部运行。传入增量变化的金融工具 ID 也会与订单簿的金融工具 ID 进行校验，若不匹配则返回 `BookIntegrityError::InstrumentMismatch`。

## 美化打印

`OrderBook` 和 `OwnOrderBook` 都提供了 `pprint` 方法，将订单簿渲染为人类可读的表格：

```rust
book.pprint(5, None);
book.pprint(5, Some(Decimal::new(1, 2))); // group_size = 0.01
```

`group_size` 参数会将价位归并到更粗粒度的分组中，适用于 tick 尺寸很细的金融工具。输出是一个格式化的表格，买单在左侧，价格居中，卖单在右侧。

## 自有订单簿

`OwnOrderBook` 将你自己的在途订单与公开订单簿分开追踪。做市以及其他报价类策略使用它，在扣除自己的订单后估算每个价位上可用的流动性。

当启用 `manage_own_order_books` 时，执行引擎会维护自有订单簿。随着订单事件改变状态，缓存会更新现有的自有订单簿。符合条件的订单需要带有价格，并且不使用 `IOC` 或 `FOK` 的生效时间 (time in force)。即便订单本身不满足被追踪的条件，终态事件仍可能清理已存在的自有订单簿条目。

### 订单生命周期

`OwnOrderBook` 在订单的整个生命周期中追踪它们。订单在提交时、或在对账（reconciliation）中被实例化时被加入，随着状态变化的到来而更新，并在关闭时被移除。更新涵盖订单模型所支持的各种状态，包括已接受、待更新、待取消、部分成交、完全成交、已取消、已过期、已拒绝以及已拒收。

每个 `OwnBookOrder` 携带以下信息：

- `client_order_id`：客户端订单 ID，用于将自有订单簿与缓存状态进行对账。
- `venue_order_id`：场所订单 ID（当已分配时）。
- `side`、`price` 和 `size`：订单方向以及自有订单簿中剩余的价位信息。
- `order_type` 和 `time_in_force`：供过滤器和诊断使用的订单类型元数据。
- `status`：当前订单状态，例如 `SUBMITTED`、`ACCEPTED` 或 `PENDING_CANCEL`。
- `ts_last`：应用于该自有订单簿订单的最新订单事件的时间戳。
- `ts_accepted`：订单被场所接受时的时间戳。
- `ts_submitted`：订单被提交时的时间戳。
- `ts_init`：订单被初始化时的时间戳。

借助这些字段，过滤后的视图可以按状态和接受时间将自有订单纳入或排除（参见[状态与时间过滤](#状态与时间过滤)）。

### 审计

`audit_open_orders` 方法将自有订单簿与一组有效的客户端订单 ID 进行对账。任何不在所提供集合中的自有订单簿订单都会被移除，并作为审计错误记录到日志。`Cache::audit_own_order_books` 会基于挂单（open）和在途（in-flight）订单来构建这个集合，因此已提交的订单不会在正常的场所延迟窗口期间被移除。实盘系统可以通过自有订单簿审计间隔（own-books audit interval）周期性地运行此审计。

### 查询

```rust
// 检查某个特定订单是否被追踪
let in_book = own_book.is_order_in_book(&client_order_id);

// 按方向获取所有被追踪的订单 ID
let bid_ids = own_book.bid_client_order_ids();
let ask_ids = own_book.ask_client_order_ids();

// 按价位聚合的数量
let bid_qty = own_book.bid_quantity(None, None, None, None, None);
let ask_qty = own_book.ask_quantity(None, None, None, None, None);

// 美化打印
own_book.pprint(5, None);
```

### 过滤后的视图

从公开订单簿中减去你自己的订单，以查看净可用流动性：

```rust
// 过滤后的 价格 -> 数量 映射（已减去自有订单）
let net_bids = book.bids_filtered_as_map(Some(10), Some(&own_book), None, None, None);
let net_asks = book.asks_filtered_as_map(Some(10), Some(&own_book), None, None, None);

// 完整的过滤后 OrderBook，可使用全部分析方法
let filtered = book.filtered_view(Some(&own_book), Some(10), None, None, None);
let avg_px = filtered.get_avg_px_for_quantity(quantity, OrderSide::Buy);
```

`filtered_view` 方法返回一个新的 `OrderBook`，其中已减去你自己的下单量，从而可以在这个净订单簿上使用完整的分析方法（`spread`、`midpoint`、`get_avg_px_for_quantity` 等）。

### 状态与时间过滤

过滤后的视图支持对自有订单进行可选的状态和基于时间的过滤：

```rust
let status = Some(AHashSet::from([OrderStatus::Accepted]));

// 仅减去 ACCEPTED 状态的订单（忽略 SUBMITTED、PENDING_CANCEL 等）
let filtered = book.filtered_view(Some(&own_book), None, status, None, None);
```

`accepted_buffer_ns` 参数提供了一个宽限期：设置后，只有满足 `ts_accepted + buffer <= now` 的订单才会被纳入。这会排除掉那些刚刚被接受、可能尚未出现在公开订单簿数据源中的订单。该缓冲期作用于 `ts_accepted` 字段，与订单状态无关。可与状态过滤器结合使用，从而同时排除非已接受状态的订单。

```rust
// 仅减去至少在 500ms 前被接受的订单
let filtered = book.filtered_view(
    Some(&own_book),
    None,
    None,
    Some(500_000_000),
    Some(clock.timestamp_ns()),
);
```

## 二元市场

对于二元/预测市场（例如 Polymarket），金融工具有两个互补的方向（YES 和 NO），其价格之和为 1.0。在 NO 一侧以 0.40 买入，在经济意义上等价于在 YES 一侧以 0.60 卖出。

`OwnOrderBook::combined_with_opposite` 方法负责处理这种转换，将你在两侧的订单合并到一个单一视图中：

```rust
let yes_own = own_yes_book
    .cloned()
    .unwrap_or_else(|| OwnOrderBook::new(yes_instrument_id));

let no_own = own_no_book
    .cloned()
    .unwrap_or_else(|| OwnOrderBook::new(no_instrument_id));

// 以平价价格转换（1 - price）合并 NO 一侧的订单
let combined = yes_own.combined_with_opposite(&no_own).unwrap();

// 使用合并后的自有订单簿过滤公开的 YES 订单簿
let filtered = book.filtered_view(Some(&combined), None, None, None, None);
```

该转换的规则如下：

- NO 一侧价格为 P 的卖单，在合并后的订单簿中变为价格 1 - P 的买单。
- NO 一侧价格为 P 的买单，在合并后的订单簿中变为价格 1 - P 的卖单。

这样便能完整呈现你在市场两侧的自有流动性全貌。
