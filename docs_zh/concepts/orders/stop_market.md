# 止损市价单 (Stop-Market)

`FIX OrdType <40>=3` (Stop)

*止损市价单 (Stop-Market)* 是一种条件单，一旦被触发，便会立即下达一个*市价单 (Market)*。这种订单类型常被用作止损单以限制亏损，可以是针对多头持仓的 SELL 订单，也可以是针对空头持仓的 BUY 订单。

## 使用场景

当你在价格突破某个水平后需要确定的成交保障时，就应使用*止损市价单 (Stop-Market)*，例如保护性止损或突破入场。由于它在触发时会转换为*市价单 (Market)*，因此持仓几乎总能被开立或平掉。其代价在于触发价并不是成交价：在快速波动或跳空的行情中，成交可能远超止损价位，所以它是用价格确定性换取成交确定性（与*止损限价单 (Stop-Limit)* 相反）。

## 示例

在以下示例中，我们在 Binance 现货/杠杆 (Spot/Margin) 交易所创建一个*止损市价单 (Stop-Market)*，以 100,000 USDT 的触发价 SELL 1 BTC，在另行通知前持续有效：

```rust tab="Rust"
use nautilus_model::{
    enums::{OrderSide, TimeInForce, TriggerType},
    identifiers::InstrumentId,
    types::{Price, Quantity},
};

let order = self.core.order_factory().stop_market(
    InstrumentId::from("BTCUSDT.BINANCE"),
    OrderSide::Sell,
    Quantity::from(1),
    Price::from("100000"),
    Some(TriggerType::LastPrice), // optional (default DEFAULT)
    Some(TimeInForce::Gtc),       // optional (default GTC)
    None,                         // expire_time
    Some(false),                  // reduce_only (default false)
    None,                         // quote_quantity (default false)
    None,                         // display_qty
    None,                         // emulation_trigger
    None,                         // trigger_instrument_id
    None,                         // exec_algorithm_id
    None,                         // exec_algorithm_params
    None,                         // tags
    None,                         // client_order_id
);
```

```python tab="Python"
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.enums import TriggerType
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Price
from nautilus_trader.model import Quantity
from nautilus_trader.model.orders import StopMarketOrder

order: StopMarketOrder = self.order_factory.stop_market(
    instrument_id=InstrumentId.from_str("BTCUSDT.BINANCE"),
    order_side=OrderSide.SELL,
    quantity=Quantity.from_int(1),
    trigger_price=Price.from_int(100_000),
    trigger_type=TriggerType.LAST_PRICE,  # <-- optional (default DEFAULT)
    time_in_force=TimeInForce.GTC,  # <-- optional (default GTC)
    expire_time=None,  # <-- optional (default None)
    reduce_only=False,  # <-- optional (default False)
    tags=None,  # <-- optional (default None)
)
```

更多细节请参阅 [`StopMarketOrder` API 参考](/docs/python-api-latest/model/orders.html#nautilus_trader.model.orders.stop_market.StopMarketOrder)。

## 相关指南

- [订单 (Orders)](index.md#触发类型-trigger-type) - 触发类型及其他执行指令。
- [模拟订单 (Emulated orders)](emulated.md) - 在不支持原生条件单的交易所上模拟条件单。
- [执行 (Execution)](../execution.md) - 订单如何送达交易所以及成交如何处理。
