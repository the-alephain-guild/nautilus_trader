# 跟踪止损限价单 (Trailing-Stop-Limit)

`FIX OrdType <40>=4` (Stop Limit) + 跟踪锚定

*跟踪止损限价单 (Trailing-Stop-Limit)* 是一种条件单，它让止损触发价格以固定的偏移量跟随既定的市场价格。一旦被触发，系统会立即按既定价格挂出一个 *限价单 (Limit)*（该价格也会随着市场波动持续更新，直到被触发为止）。

## 使用场景

当你既想要跟踪止损的动态跟随特性，又想为成交价设置上限时，可以使用 *跟踪止损限价单 (Trailing-Stop-Limit)*。它的优势在于将跟踪式保护与价格控制结合在一起。代价则是 *止损限价单 (Stop-Limit)* 在跟踪场景下的等价缺陷：在快速反转行情中，释放出的 *限价单 (Limit)* 可能无法成交，从而使持仓敞口保持开放。

## 示例

在下面的示例中，我们在 Currenex FX ECN 上创建一个 *跟踪止损限价单 (Trailing-Stop-Limit)*，使用 USD 买入 1,250,000 AUD，限价为 0.71000 USD，在 0.72000 USD 处激活，随后以相对当前卖价 0.00100 USD 的止损偏移量进行跟踪，在另行通知前持续有效：

```rust tab="Rust"
use nautilus_model::{
    enums::{OrderSide, TimeInForce, TrailingOffsetType, TriggerType},
    identifiers::InstrumentId,
    types::{Price, Quantity},
};
use rust_decimal_macros::dec;
use ustr::Ustr;

let order = self.core.order_factory().trailing_stop_limit(
    InstrumentId::from("AUD/USD.CURRENEX"),
    OrderSide::Buy,
    Quantity::from(1_250_000),
    Price::from("0.71000"),          // limit price
    dec!(0.00050),                   // limit_offset
    dec!(0.00100),                   // trailing_offset
    Some(TrailingOffsetType::Price), // optional (default PRICE)
    Some(Price::from("0.72000")),    // activation_price
    None,                            // trigger_price (falls back to activation_price)
    Some(TriggerType::BidAsk),       // optional (default DEFAULT)
    Some(TimeInForce::Gtc),          // optional (default GTC)
    None,                            // expire_time
    Some(false),                     // post_only (default false)
    Some(true),                      // reduce_only (default false)
    None,                            // quote_quantity (default false)
    None,                            // display_qty
    None,                            // emulation_trigger
    None,                            // trigger_instrument_id
    None,                            // exec_algorithm_id
    None,                            // exec_algorithm_params
    Some(vec![Ustr::from("TRAILING_STOP")]), // tags
    None,                            // client_order_id
);
```

```python tab="Python"
import pandas as pd
from decimal import Decimal
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.enums import TriggerType
from nautilus_trader.model.enums import TrailingOffsetType
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Price
from nautilus_trader.model import Quantity
from nautilus_trader.model.orders import TrailingStopLimitOrder

order: TrailingStopLimitOrder = self.order_factory.trailing_stop_limit(
    instrument_id=InstrumentId.from_str("AUD/USD.CURRENEX"),
    order_side=OrderSide.BUY,
    quantity=Quantity.from_int(1_250_000),
    price=Price.from_str("0.71000"),
    activation_price=Price.from_str("0.72000"),
    trigger_type=TriggerType.BID_ASK,  # <-- optional (default DEFAULT)
    limit_offset=Decimal("0.00050"),
    trailing_offset=Decimal("0.00100"),
    trailing_offset_type=TrailingOffsetType.PRICE,
    time_in_force=TimeInForce.GTC,  # <-- optional (default GTC)
    expire_time=None,  # <-- optional (default None)
    reduce_only=True,  # <-- optional (default False)
    tags=["TRAILING_STOP"],  # <-- optional (default None)
)
```

更多细节请参阅 [`TrailingStopLimitOrder` API 参考文档](/docs/python-api-latest/model/orders.html#nautilus_trader.model.orders.trailing_stop_limit.TrailingStopLimitOrder)。

## 相关指南

- [订单 (Orders)](index.md#触发偏移类型) - 触发与跟踪偏移类型。
- [模拟订单 (Emulated orders)](emulated.md) - 在不原生支持的交易场所上模拟跟踪止损。
- [执行 (Execution)](../execution.md) - 订单如何到达交易场所以及成交如何处理。
