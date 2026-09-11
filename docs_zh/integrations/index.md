# 集成 (Integrations)

NautilusTrader 使用模块化的*适配器 (adapters)* 连接交易场所 (venues) 和数据提供商 (data providers)，将原始 API 转换为统一接口和标准化领域模型。

目前支持以下集成：

| 名称                                                        | ID                    | 类型            | 状态                                                   | 文档                    |
| :-------------------------------------------------------- | :-------------------- | :------------ | :--------------------------------------------------- | :-------------------- |
| [AX Exchange](https://architect.exchange)                 | `AX`                  | 永续合约交易所       | ![status](https://img.shields.io/badge/stable-green) | [指南](architect_ax.md) |
| [Betfair](https://betfair.com)                            | `BETFAIR`             | 体育博彩交易所       | ![status](https://img.shields.io/badge/stable-green) | [指南](betfair.md)      |
| [Binance](https://binance.com)                            | `BINANCE`             | 加密货币交易所 (CEX) | ![status](https://img.shields.io/badge/stable-green) | [指南](binance.md)      |
| [Coinbase](https://coinbase.com)                          | `COINBASE`            | 加密货币交易所 (CEX) | ![status](https://img.shields.io/badge/stable-green) | [指南](coinbase.md)     |
| [BitMEX](https://www.bitmex.com)                          | `BITMEX`              | 加密货币交易所 (CEX) | ![status](https://img.shields.io/badge/stable-green) | [指南](bitmex.md)       |
| [Bybit](https://www.bybit.com)                            | `BYBIT`               | 加密货币交易所 (CEX) | ![status](https://img.shields.io/badge/stable-green) | [指南](bybit.md)        |
| [Databento](https://databento.com)                        | `DATABENTO`           | 数据提供商         | ![status](https://img.shields.io/badge/stable-green) | [指南](databento.md)    |
| [Deribit](https://www.deribit.com)                        | `DERIBIT`             | 加密货币交易所 (CEX) | ![status](https://img.shields.io/badge/stable-green) | [指南](deribit.md)      |
| [Derive](https://www.derive.xyz)                          | `DERIVE`              | 加密货币交易所 (DEX) | ![status](https://img.shields.io/badge/beta-yellow)  | [指南](derive.md)       |
| [dYdX](https://dydx.exchange/)                            | `DYDX`                | 加密货币交易所 (DEX) | ![status](https://img.shields.io/badge/stable-green) | [指南](dydx.md)         |
| [Hyperliquid](https://hyperliquid.xyz)                    | `HYPERLIQUID`         | 加密货币交易所 (DEX) | ![status](https://img.shields.io/badge/stable-green) | [指南](hyperliquid.md)  |
| [Lighter](https://lighter.xyz)                            | `LIGHTER`             | 加密货币交易所 (DEX) | ![status](https://img.shields.io/badge/beta-yellow)  | [指南](lighter.md)      |
| [Interactive Brokers](https://www.interactivebrokers.com) | `INTERACTIVE_BROKERS` | 经纪商 (多交易场所)   | ![status](https://img.shields.io/badge/stable-green) | [指南](ib.md)           |
| [Kraken](https://kraken.com)                              | `KRAKEN`              | 加密货币交易所 (CEX) | ![status](https://img.shields.io/badge/stable-green) | [指南](kraken.md)       |
| [OKX](https://okx.com)                                    | `OKX`                 | 加密货币交易所 (CEX) | ![status](https://img.shields.io/badge/stable-green) | [指南](okx.md)          |
| [Polymarket](https://polymarket.com)                      | `POLYMARKET`          | 预测市场 (DEX)    | ![status](https://img.shields.io/badge/stable-green) | [指南](polymarket.md)   |
| [Tardis](https://tardis.dev)                              | `TARDIS`              | 加密货币数据提供商     | ![status](https://img.shields.io/badge/stable-green) | [指南](tardis.md)       |

- **ID**：集成适配器客户端的默认客户端 ID。
- **类型**：集成的类型（通常为交易场所类型）。

## 状态说明

- `planned`：计划在未来开发。
- `building`：正在建设中，可能尚未处于可用状态。
- `beta`：已完成最小可用状态，处于 Beta 测试阶段。
- `stable`：功能集和 API 已稳定，该集成已经过开发者和用户的合理程度测试（可能仍存在部分缺陷）。

## 实现目标

NautilusTrader 的首要目标是提供一个可与多种集成配合使用的统一交易系统。为支持最广泛的交易策略，将优先实现*标准*功能：

- 请求历史行情数据。
- 流式实时行情数据。
- 执行状态对账。
- 提交标准订单类型和标准执行指令。
- 修改现有订单（如果交易所支持）。
- 取消订单。

每个集成的实现旨在满足以下标准：

- 底层客户端组件应尽可能贴近交易所 API。
- 交易所的全部功能范围（适用于 NautilusTrader 的部分）*最终*都应得到支持。
- 将添加交易所特定的数据类型，以支持用户合理预期的功能和返回类型。
- 交易所或 NautilusTrader 不支持的操作在调用时将记录为警告或错误日志。

## API 统一化

所有集成必须符合 NautilusTrader 的系统 API，这要求进行规范化和标准化处理：

- 交易品种代码应使用交易场所的原生格式，除非需要消歧义（例如 Binance 现货与 Binance 合约）。
- 时间戳必须使用 UNIX 纪元纳秒。如果使用毫秒，字段/属性名称应明确以 `_ms` 结尾。
