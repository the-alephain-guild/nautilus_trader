# 触价限价单 (Limit-If-Touched)

`FIX OrdType <40>` 无专用取值（通常用 `4` Stop Limit 并配合一个有利的触发价）

*触价限价单 (Limit-If-Touched)* 是一种条件单，一旦被触发就会立即在指定价格挂出一个 *限价单 (Limit)*。

## 使用场景

可以使用 *触价限价单 (Limit-If-Touched)* 在触发价被触及后才激活一个带价格保护的订单，例如在价格接近目标位时激活止盈 *限价单 (Limit)*，而不是过早地把它挂在盘口。它的优势在于把条件激活与受限的成交价格结合了起来。代价则和 *止损限价单 (Stop-Limit)* 一样：如果价格在触发后穿过限价，订单可能无法成交。

## 示例

在下面的示例中，我们创建了一个 *触价限价单 (Limit-If-Touched)*，在 Binance Futures 交易所以 30,100 USDT 的限价买入 5 张 BTCUSDT-PERP 永续期货合约（一旦市场触及 30,150 USDT 的触发价），有效期至 2022 年 6 月 6 日正午（UTC）：

```rust tab="Rust"
use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::{OrderSide, TimeInForce, TriggerType},
    identifiers::InstrumentId,
    types::{Price, Quantity},
};
use ustr::Ustr;

let order = self.core.order_factory().limit_if_touched(
    InstrumentId::from("BTCUSDT-PERP.BINANCE"),
    OrderSide::Buy,
    Quantity::from(5),
    Price::from("30100"),
    Price::from("30150"),
    Some(TriggerType::LastPrice), // optional (default DEFAULT)
    Some(TimeInForce::Gtd),       // optional (default GTC)
    Some(UnixNanos::from(1_654_516_800_000_000_000_u64)), // 2022-06-06T12:00:00 UTC
    Some(true),                   // post_only (default false)
    Some(false),                  // reduce_only (default false)
    None,                         // quote_quantity (default false)
    None,                         // display_qty
    None,                         // emulation_trigger
    None,                         // trigger_instrument_id
    None,                         // exec_algorithm_id
    None,                         // exec_algorithm_params
    Some(vec![Ustr::from("TAKE_PROFIT")]), // tags
    None,                         // client_order_id
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
from nautilus_trader.model.orders import LimitIfTouchedOrder

order: LimitIfTouchedOrder = self.order_factory.limit_if_touched(
    instrument_id=InstrumentId.from_str("BTCUSDT-PERP.BINANCE"),
    order_side=OrderSide.BUY,
    quantity=Quantity.from_int(5),
    price=Price.from_str("30_100"),
    trigger_price=Price.from_str("30_150"),
    trigger_type=TriggerType.LAST_PRICE,  # <-- optional (default DEFAULT)
    time_in_force=TimeInForce.GTD,  # <-- optional (default GTC)
    expire_time=pd.Timestamp("2022-06-06T12:00"),
    post_only=True,  # <-- optional (default False)
    reduce_only=False,  # <-- optional (default False)
    tags=["TAKE_PROFIT"],  # <-- optional (default None)
)
```

更多细节请参阅 [`LimitIfTouchedOrder` API 参考](/docs/python-api-latest/model/orders.html#nautilus_trader.model.orders.limit_if_touched.LimitIfTouchedOrder)。

## 相关指南

- [订单 (Orders)](index.md#触发类型-trigger-type) - 触发类型及其他执行指令。
- [模拟订单 (Emulated orders)](emulated.md) - 在不原生支持条件单的交易所上模拟条件单。
- [执行 (Execution)](../execution.md) - 订单如何到达交易所以及成交如何处理。
