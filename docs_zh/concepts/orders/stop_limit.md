# 止损限价 (Stop-Limit)

`FIX OrdType <40>=4` (Stop Limit)

*止损限价 (Stop-Limit)* 订单是一种条件订单，一旦被触发，会立即按指定价格挂出一个 *限价 (Limit)* 订单。

## 适用场景

当你既需要止损触发，又希望对最差可接受成交价设置上限时，可使用 *止损限价 (Stop-Limit)* 订单，例如保护性离场，或那种你拒绝在某价格之外成交的突破入场。它的优点在于释放出来的 *限价 (Limit)* 订单具有价格保护。它的代价是相对于 *止损市价 (Stop-Market)* 所固有的核心风险：如果市场跳空越过触发价和限价，订单可能完全无法成交，从而使持仓失去保护。

## 示例

在下面的示例中，我们在 Currenex FX ECN 上创建一个 *止损限价 (Stop-Limit)* 订单，一旦市场触及触发价 1.30010 USD，便以限价 1.3000 USD 买入 50,000 GBP，订单有效期至 2022 年 6 月 6 日中午（UTC）：

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::{OrderSide, TimeInForce, TriggerType},
    identifiers::InstrumentId,
    types::{Price, Quantity},
};

let order = self.core.order_factory().stop_limit(
    InstrumentId::from("GBP/USD.CURRENEX"),
    OrderSide::Buy,
    Quantity::from(50_000),
    Price::from("1.30000"),
    Price::from("1.30010"),
    Some(TriggerType::BidAsk), // optional (default DEFAULT)
    Some(TimeInForce::Gtd),    // optional (default GTC)
    Some(UnixNanos::from(1_654_516_800_000_000_000_u64)), // 2022-06-06T12:00:00 UTC
    Some(true),                // post_only (default false)
    Some(false),               // reduce_only (default false)
    None,                      // quote_quantity (default false)
    None,                      // display_qty
    None,                      // emulation_trigger
    None,                      // trigger_instrument_id
    None,                      // exec_algorithm_id
    None,                      // exec_algorithm_params
    None,                      // tags
    None,                      // client_order_id
);
```

```python tab="Python"
import pandas as pd
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.enums import TriggerType
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Price
from nautilus_trader.model import Quantity
from nautilus_trader.model.orders import StopLimitOrder

order: StopLimitOrder = self.order_factory.stop_limit(
    instrument_id=InstrumentId.from_str("GBP/USD.CURRENEX"),
    order_side=OrderSide.BUY,
    quantity=Quantity.from_int(50_000),
    price=Price.from_str("1.30000"),
    trigger_price=Price.from_str("1.30010"),
    trigger_type=TriggerType.BID_ASK,  # <-- optional (default DEFAULT)
    time_in_force=TimeInForce.GTD,  # <-- optional (default GTC)
    expire_time=pd.Timestamp("2022-06-06T12:00"),
    post_only=True,  # <-- optional (default False)
    reduce_only=False,  # <-- optional (default False)
    tags=None,  # <-- optional (default None)
)
```

更多细节参见 [`StopLimitOrder` API 参考](/docs/python-api-latest/model/orders.html#nautilus_trader.model.orders.stop_limit.StopLimitOrder)。

## 相关指南

- [订单 (Orders)](index.md#触发类型-trigger-type) - 触发类型及其他执行指令。
- [模拟订单 (Emulated orders)](emulated.md) - 在不原生支持条件订单的交易场所上模拟此类订单。
- [执行 (Execution)](../execution.md) - 订单如何送达交易场所以及成交如何处理。
