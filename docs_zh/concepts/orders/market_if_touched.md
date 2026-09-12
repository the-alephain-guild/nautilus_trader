# 触价市价单 (Market-If-Touched)

`FIX OrdType <40>=J` (Market If Touched)

*触价市价单 (Market-If-Touched)* 是一种条件单，一旦触发便会立即下达一个 *Market* 单。这种订单类型常用于在某个止损价位上建立新仓位，或为现有仓位锁定利润，可以是针对 LONG 仓位的 SELL 单，也可以是针对 SHORT 仓位的 BUY 单。

## 适用场景

当目标价格被触及时，使用 *触价市价单 (Market-If-Touched)* 可以获得执行确定性，例如在价格回撤到某个水平时入场，或在目标价位止盈。它的行为类似于反方向的止损单（在当前市场价之下买入，或在其之上卖出），并在触发时转换为 *Market* 单。其取舍与任何市价执行相同：触发价并非成交价，而且在快速行情中成交价可能会出现滑点。

## 示例

在下面的示例中，我们在 Binance Futures 交易所创建一个 *触价市价单 (Market-If-Touched)*，以 10,000 USDT 的触发价 SELL 10 张 ETHUSDT-PERP 永续期货合约，并保持有效直至另行通知：

```rust tab="Rust"
use nautilus_model::{
    enums::{OrderSide, TimeInForce, TriggerType},
    identifiers::InstrumentId,
    types::{Price, Quantity},
};
use ustr::Ustr;

let order = self.core.order_factory().market_if_touched(
    InstrumentId::from("ETHUSDT-PERP.BINANCE"),
    OrderSide::Sell,
    Quantity::from(10),
    Price::from("10000.00"),
    Some(TriggerType::LastPrice),    // optional (default DEFAULT)
    Some(TimeInForce::Gtc),          // optional (default GTC)
    None,                            // expire_time
    Some(false),                     // reduce_only (default false)
    None,                            // quote_quantity (default false)
    None,                            // emulation_trigger
    None,                            // trigger_instrument_id
    None,                            // exec_algorithm_id
    None,                            // exec_algorithm_params
    Some(vec![Ustr::from("ENTRY")]), // tags
    None,                            // client_order_id
);
```

```python tab="Python"
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.enums import TriggerType
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Price
from nautilus_trader.model import Quantity
from nautilus_trader.model.orders import MarketIfTouchedOrder

order: MarketIfTouchedOrder = self.order_factory.market_if_touched(
    instrument_id=InstrumentId.from_str("ETHUSDT-PERP.BINANCE"),
    order_side=OrderSide.SELL,
    quantity=Quantity.from_int(10),
    trigger_price=Price.from_str("10_000.00"),
    trigger_type=TriggerType.LAST_PRICE,  # <-- optional (default DEFAULT)
    time_in_force=TimeInForce.GTC,  # <-- optional (default GTC)
    expire_time=None,  # <-- optional (default None)
    reduce_only=False,  # <-- optional (default False)
    tags=["ENTRY"],  # <-- optional (default None)
)
```

更多详情参见 [`MarketIfTouchedOrder` API 参考](/docs/python-api-latest/model/orders.html#nautilus_trader.model.orders.market_if_touched.MarketIfTouchedOrder)。

## 相关指南

- [订单 (Orders)](index.md#触发类型-trigger-type) - 触发类型及其他执行指令。
- [模拟订单 (Emulated orders)](emulated.md) - 在不原生支持条件单的交易所上模拟条件单。
- [执行 (Execution)](../execution.md) - 订单如何到达交易所以及成交如何处理。
