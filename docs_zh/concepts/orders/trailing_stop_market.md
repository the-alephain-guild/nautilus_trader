# 追踪止损市价单 (Trailing-Stop-Market)

`FIX OrdType <40>=3` (Stop) + 追踪挂钩 (trailing peg)

*追踪止损市价单 (Trailing-Stop-Market)* 是一种条件单，它让止损触发价相对设定的市场价格保持一个固定偏移量进行追踪。一旦被触发，系统会立即下达一个 *市价单 (Market)*。

## 使用场景

使用 *追踪止损市价单 (Trailing-Stop-Market)* 可以在让仓位继续运行的同时锁定收益：触发价会以固定偏移量追随有利方向的行情移动，仅在行情反转时才会触发，无需手动调整。它的优势在于动态保护加上触发时的成交确定性。代价则是需要权衡偏移量的选择——偏移量太小会带来被反复扫损（whipsaw）的风险，太大则会让出过多利润；此外，在行情急剧反转时市价成交仍可能产生滑点。

## 示例

在下面的示例中，我们在 Binance Futures 交易所创建一个 *追踪止损市价单 (Trailing-Stop-Market)*，用于卖出（SELL）10 张 ETHUSD-PERP COIN_M 保证金永续合约，激活价格为 5,000 USD，随后以相对当前最新成交价 1%（以基点表示）的偏移量进行追踪：

```rust tab="Rust"
use nautilus_model::{
    enums::{OrderSide, TimeInForce, TrailingOffsetType, TriggerType},
    identifiers::InstrumentId,
    types::{Price, Quantity},
};
use rust_decimal::Decimal;
use ustr::Ustr;

let order = self.core.order_factory().trailing_stop_market(
    InstrumentId::from("ETHUSD-PERP.BINANCE"),
    OrderSide::Sell,
    Quantity::from(10),
    Decimal::from(100),                    // trailing_offset
    Some(TrailingOffsetType::BasisPoints), // optional (default PRICE)
    Some(Price::from("5000")),             // activation_price
    None,                                  // trigger_price (falls back to activation_price)
    Some(TriggerType::LastPrice),          // optional (default DEFAULT)
    Some(TimeInForce::Gtc),                // optional (default GTC)
    None,                                  // expire_time
    Some(true),                            // reduce_only (default false)
    None,                                  // quote_quantity (default false)
    None,                                  // display_qty
    None,                                  // emulation_trigger
    None,                                  // trigger_instrument_id
    None,                                  // exec_algorithm_id
    None,                                  // exec_algorithm_params
    Some(vec![Ustr::from("TRAILING_STOP-1")]), // tags
    None,                                  // client_order_id
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
from nautilus_trader.model.orders import TrailingStopMarketOrder

order: TrailingStopMarketOrder = self.order_factory.trailing_stop_market(
    instrument_id=InstrumentId.from_str("ETHUSD-PERP.BINANCE"),
    order_side=OrderSide.SELL,
    quantity=Quantity.from_int(10),
    activation_price=Price.from_str("5_000"),
    trigger_type=TriggerType.LAST_PRICE,  # <-- optional (default DEFAULT)
    trailing_offset=Decimal(100),
    trailing_offset_type=TrailingOffsetType.BASIS_POINTS,
    time_in_force=TimeInForce.GTC,  # <-- optional (default GTC)
    expire_time=None,  # <-- optional (default None)
    reduce_only=True,  # <-- optional (default False)
    tags=["TRAILING_STOP-1"],  # <-- optional (default None)
)
```

更多细节请参阅 [`TrailingStopMarketOrder` API 参考](/docs/python-api-latest/model/orders.html#nautilus_trader.model.orders.trailing_stop_market.TrailingStopMarketOrder)。

## 相关指南

- [订单 (Orders)](index.md#触发偏移类型) - 触发与追踪偏移类型。
- [模拟订单 (Emulated orders)](emulated.md) - 在不支持原生追踪止损的交易所上模拟追踪止损。
- [执行 (Execution)](../execution.md) - 订单如何到达交易所以及成交如何被处理。
