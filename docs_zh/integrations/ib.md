# Interactive Brokers

Interactive Brokers (IB) 是一个交易平台（trading platform），提供广泛金融工具（financial instruments）的市场准入，包括股票（stocks）、期权（options）、期货（futures）、外汇（currencies）、债券（bonds）、基金（funds）和加密货币（cryptocurrencies）。NautilusTrader 提供了一个适配器（adapter）来与 IB 集成（integration），通过其 Python 库 [ibapi](https://github.com/nautechsystems/ibapi) 使用 [Trader Workstation (TWS) API](https://ibkrcampus.com/ibkr-api-page/trader-workstation-api/)。

TWS API 作为 IB 独立交易应用的接口：TWS 和 IB Gateway。两者均可从 IB 网站下载。如果你尚未安装 TWS 或 IB Gateway，请参阅[初始设置](https://ibkrcampus.com/ibkr-api-page/trader-workstation-api/#tws-download)指南。在 NautilusTrader 中，你将通过 `InteractiveBrokersClient` 建立与其中一个应用的连接。

另外，你也可以使用 IB Gateway 的 [Docker 化版本](https://github.com/gnzsnz/ib-gateway-docker)，这在将交易策略部署到托管云平台时特别有用。这需要在你的机器上安装 [Docker](https://www.docker.com/)，以及 [docker](https://pypi.org/project/docker/) Python 包，NautilusTrader 已将其作为额外依赖包含在内。

:::note
独立的 TWS 和 IB Gateway 应用在启动时需要手动输入用户名、密码和交易模式（实盘或模拟）。Docker 化版本的 IB Gateway 会自动处理这些步骤。
:::

## 安装

安装带有 Interactive Brokers（和 Docker）支持的 NautilusTrader：

```bash
uv pip install "nautilus_trader[ib,docker]"
```

从源码构建并包含所有额外依赖（包括 IB 和 Docker）：

```bash
uv sync --all-extras
```

:::note
由于 IB 不提供 `ibapi` 的 wheels 包，NautilusTrader [重新打包](https://pypi.org/project/nautilus-ibapi/)了它并发布到 PyPI。
:::

## 示例

你可以在[此处](https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/interactive_brokers/)找到实盘示例脚本。

## 快速入门

在实现你的交易策略之前，请确保 TWS（Trader Workstation）或 IB Gateway 正在运行。你可以使用凭证登录其中一个独立应用，或通过 `DockerizedIBGateway` 以编程方式连接。

:::warning
在连接 NautilusTrader 之前，请将 TWS 或 IB Gateway 配置为以 UTC 返回市场数据时间戳。此设置必须由用户在 TWS/IB Gateway 中启用，因为 NautilusTrader 设计为使用 UTC 时间戳工作。
:::

### 连接方式

连接到 Interactive Brokers 有两种主要方式：

1. **连接到已有的 TWS 或 IB Gateway 实例**
2. **使用 Docker 化的 IB Gateway（推荐用于自动化部署）**

### 默认端口

Interactive Brokers 根据应用和交易模式使用不同的默认端口：

| 应用         | 模拟交易 | 实盘交易 |
| ---------- | ---- | ---- |
| TWS        | 7497 | 7496 |
| IB Gateway | 4002 | 4001 |

### 建立与已有网关或 TWS 的连接

连接到已有的网关或 TWS 时，在 `InteractiveBrokersDataClientConfig` 和 `InteractiveBrokersExecClientConfig` 中指定 `ibg_host` 和 `ibg_port` 参数：

```python
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersDataClientConfig
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersExecClientConfig

# TWS 模拟交易示例（默认端口 7497）
data_config = InteractiveBrokersDataClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=7497,
    ibg_client_id=1,
)

exec_config = InteractiveBrokersExecClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=7497,
    ibg_client_id=1,
    account_id="DU123456",  # 你的模拟交易账户 ID
)
```

### 建立与 Docker 化 IB Gateway 的连接

对于自动化部署，推荐使用 Docker 化网关。在两个客户端配置中，将 `DockerizedIBGatewayConfig` 实例提供给 `dockerized_gateway`。不需要 `ibg_host` 和 `ibg_port` 参数，因为它们会自动管理。

```python
from nautilus_trader.adapters.interactive_brokers.config import DockerizedIBGatewayConfig
from nautilus_trader.adapters.interactive_brokers.gateway import DockerizedIBGateway

gateway_config = DockerizedIBGatewayConfig(
    username="your_username",  # 或设置 TWS_USERNAME 环境变量
    password="your_password",  # 或设置 TWS_PASSWORD 环境变量
    trading_mode="paper",  # "paper" 或 "live"
    read_only_api=True,  # 设置为 False 以允许订单执行
    timeout=300,  # 启动超时时间（秒）
)

# 首次启动可能需要一些时间
gateway = DockerizedIBGateway(config=gateway_config)
gateway.start()

# 确认已登录
print(gateway.is_logged_in(gateway.container))

# 查看日志
print(gateway.container.logs())
```

### 环境变量

要向 Interactive Brokers Gateway 提供凭证，可以将 `username` 和 `password` 传递给 `DockerizedIBGatewayConfig`，或设置以下环境变量：

- `TWS_USERNAME`：你的 IB 账户用户名。
- `TWS_PASSWORD`：你的 IB 账户密码。
- `TWS_ACCOUNT`：你的 IB 账户 ID（用作 `account_id` 的备选项）。

### 连接管理

适配器包含连接管理功能：

- **自动重连**：通过 `IB_MAX_CONNECTION_ATTEMPTS` 环境变量配置重试次数。
- **连接超时**：通过 `connection_timeout` 参数调整超时时间（默认：300 秒）。
- **连接看门狗**：监控连接健康状态，在需要时自动触发重连。
- **优雅的错误处理**：通过错误分类处理各种连接场景。

## 概述

Interactive Brokers 适配器提供了与 IB TWS API 的集成。适配器包含几个主要组件：

### 核心组件

- **`InteractiveBrokersClient`**：中央客户端，使用 `ibapi` 执行 TWS API 请求。管理连接、处理错误并协调所有 API 交互。
- **`InteractiveBrokersDataClient`**：连接到网关以获取流式市场数据，包括报价、成交和 K 线。
- **`InteractiveBrokersExecutionClient`**：处理账户信息、订单管理和交易执行。
- **`InteractiveBrokersInstrumentProvider`**：检索和管理金融工具定义，包括对期权和期货链的支持。
- **`HistoricInteractiveBrokersClient`**：提供检索金融工具和历史数据的方法，适用于回测和研究。

### 辅助组件

- **`DockerizedIBGateway`**：管理 Docker 化的 IB Gateway 实例，用于自动化部署。
- **配置类**：为所有组件提供配置选项。
- **工厂类**：创建和配置客户端实例及必要的依赖项。

### 支持的资产类别

适配器支持通过 Interactive Brokers 可用的所有主要资产类别的交易：

- **权益类**：股票、ETF 和股票期权。
- **固定收益类**：债券和债券基金。
- **衍生品**：期货、期权和权证。
- **外汇**：即期外汇和远期外汇。
- **加密货币**：比特币、以太坊和其他数字资产。
- **大宗商品**：实物商品和商品期货。
- **指数**：指数产品和指数期权。

## Interactive Brokers 客户端

`InteractiveBrokersClient` 是 IB 适配器的核心组件，负责管理一系列功能。这些功能包括建立和维护连接、处理 API 错误、执行交易，以及收集各类数据（如市场数据、合约/金融工具数据和账户详情）。

`InteractiveBrokersClient` 被划分为几个专门的 mixin 类，每个类处理一项特定职责。

### 客户端架构

客户端使用基于 mixin 的架构，每个 mixin 处理 IB API 的一个特定方面：

#### 连接管理 (`InteractiveBrokersClientConnectionMixin`)

- 建立并维护与 TWS/Gateway 的 socket 连接。
- 处理连接超时和重连逻辑。
- 管理连接状态和健康监控。
- 支持通过 `IB_MAX_CONNECTION_ATTEMPTS` 环境变量配置重连次数。

#### 错误处理 (`InteractiveBrokersClientErrorMixin`)

- 处理所有 API 错误和警告。
- 按类型对错误进行分类（客户端错误、连接问题、请求错误）。
- 处理订阅和请求特定的错误场景。
- 提供错误日志和调试信息。

#### 账户管理 (`InteractiveBrokersClientAccountMixin`)

- 检索账户信息和余额。
- 管理持仓数据和投资组合更新。
- 处理多账户场景。
- 处理账户相关通知。

#### 合约/金融工具管理 (`InteractiveBrokersClientContractMixin`)

- 检索合约详情和规格。
- 处理金融工具搜索和查找。
- 管理合约验证和校验。
- 支持复杂金融工具类型（期权链、期货链）。

#### 市场数据管理 (`InteractiveBrokersClientMarketDataMixin`)

- 处理实时和历史市场数据订阅。
- 处理报价、成交和 K 线数据。
- 管理市场数据类型设置（实时、延迟、冻结）。
- 处理逐笔数据和市场深度。

#### 订单管理 (`InteractiveBrokersClientOrderMixin`)

- 处理订单下达、修改和撤销。
- 处理订单状态更新和执行报告。
- 管理订单验证和错误处理。
- 支持复杂订单类型和条件。

### 关键特性

- **异步操作**：所有操作完全使用 Python asyncio 异步执行。
- **错误处理**：错误分类和处理。
- **连接韧性**：可配置重试逻辑的自动重连。
- **消息处理**：高效的消息队列处理，适用于高吞吐量场景。
- **状态管理**：对连接、订阅和请求进行适当的状态跟踪。

:::tip
要排查 TWS API 传入消息的问题，可以从 `InteractiveBrokersClient._process_message` 方法开始，它是处理所有从 API 接收到的消息的主要入口。
:::

## 符号体系

`InteractiveBrokersInstrumentProvider` 支持三种构建 `InstrumentId` 实例的方法，可通过 `InteractiveBrokersInstrumentProviderConfig` 中的 `symbology_method` 枚举进行配置。

### 符号体系方法

#### 1. 简化符号体系 (`IB_SIMPLIFIED`) - 默认

当 `symbology_method` 设置为 `IB_SIMPLIFIED`（默认设置）时，系统使用直观的、人类可读的符号规则：

**各资产类别的格式规则：**

- **外汇**：`{symbol}/{currency}.{exchange}`
  - 示例：`EUR/USD.IDEALPRO`
- **股票**：`{localSymbol}.{primaryExchange}`
  - localSymbol 中的空格替换为连字符
  - 示例：`BF-B.NYSE`、`SPY.ARCA`
- **期货**：`{localSymbol}.{exchange}`
  - 个别合约使用一位数年份
  - 示例：`ESM4.CME`、`CLZ7.NYMEX`
- **连续期货**：`{symbol}.{exchange}`
  - 代表近月合约，自动展期
  - 示例：`ES.CME`、`CL.NYMEX`
- **期货期权（FOP）**：`{localSymbol}.{exchange}`
  - 格式：`{symbol}{month}{year} {right}{strike}`
  - 示例：`ESM4 C4200.CME`
- **期权**：`{localSymbol}.{exchange}`
  - 移除 localSymbol 中的所有空格
  - 示例：`AAPL230217P00155000.SMART`
- **指数**：`^{localSymbol}.{exchange}`
  - 示例：`^SPX.CBOE`、`^NDX.NASDAQ`
- **债券**：`{localSymbol}.{exchange}`
  - 示例：`912828XE8.SMART`
- **加密货币**：`{symbol}/{currency}.{exchange}`
  - 示例：`BTC/USD.PAXOS`、`ETH/USD.PAXOS`

#### 2. 原始符号体系 (`IB_RAW`)

将 `symbology_method` 设置为 `IB_RAW` 会强制使用更严格的解析规则，直接与 IB API 中定义的字段对齐。此方法在所有区域和金融工具类型中提供最大兼容性：

**格式规则：**

- **差价合约（CFD）**：`{localSymbol}={secType}.IBCFD`
- **大宗商品**：`{localSymbol}={secType}.IBCMDTY`
- **其他类型的默认格式**：`{localSymbol}={secType}.{exchange}`

**示例：**

- `IBUS30=CFD.IBCFD`
- `XAUUSD=CMDTY.IBCMDTY`
- `EUR.USD=CASH.IDEALPRO`
- `AAPL=STK.SMART`

此配置确保明确的金融工具识别，并支持来自任何地区的金融工具，特别是那些使用非标准符号体系、简化解析可能失败的情况。

### MIC 交易场所转换

适配器支持将 Interactive Brokers 交易所代码转换为市场识别码（MIC，Market Identifier Code），以实现标准化的交易场所识别：

#### `convert_exchange_to_mic_venue`

设置为 `True` 时，适配器自动将 IB 交易所代码转换为相应的 MIC 代码：

```python
instrument_provider_config = InteractiveBrokersInstrumentProviderConfig(
    convert_exchange_to_mic_venue=True,  # 启用 MIC 转换
    symbology_method=SymbologyMethod.IB_SIMPLIFIED,
)
```

**MIC 转换示例：**

- `CME` -> `XCME`（芝加哥商品交易所）
- `NASDAQ` -> `XNAS`（纳斯达克股票市场）
- `NYSE` -> `XNYS`（纽约证券交易所）
- `LSE` -> `XLON`（伦敦证券交易所）

#### `symbol_to_mic_venue`

按标的代码前缀覆盖 MIC 交易场所。在交易场所解析中**最先**应用，与 `convert_exchange_to_mic_venue` 无关。当合约的标的代码匹配某个已配置的前缀时，就使用对应的 MIC 交易场所；否则解析使用交易所（如果 `convert_exchange_to_mic_venue` 为 True，可选地进行 MIC 转换）。这对于交易所为 SMART 的 OPT 合约（例如 SPX -> XCBO）以及与 databento 风格的金融工具 ID 对齐很有用。

```python
instrument_provider_config = InteractiveBrokersInstrumentProviderConfig(
    symbol_to_mic_venue={
        "SPX": "XCBO",  # 交易所为 SMART 的 OPT -> XCBO
        "ES": "XCME",  # 所有 ES 期货/期权使用 CME MIC
        "SPY": "ARCX",  # SPY 特定使用 ARCA
    },
)
# convert_exchange_to_mic_venue 可以为 True 或 False；symbol_to_mic_venue 会被最先应用
```

#### 交易场所解析与 `_process_contract_details`

通过 `IBContract` 加载金融工具时，提供者会将 `venue=None` 传入 `_process_contract_details`，因此每个合约详情会获得各自的交易场所（通过 `symbol_to_mic_venue`、validExchanges 和 MIC 转换）。传入单个交易场所字符串的调用方仍会让所有详情使用同一个交易场所。当你有混合或 SMART 路由的结果、需要逐详情解析时，请传入 `venue=None`。

### 支持的金融工具格式

适配器支持基于 Interactive Brokers 合约规格的各种金融工具格式：

#### 期货月份代码

- **F** = 1月、**G** = 2月、**H** = 3月、**J** = 4月
- **K** = 5月、**M** = 6月、**N** = 7月、**Q** = 8月
- **U** = 9月、**V** = 10月、**X** = 11月、**Z** = 12月

#### 各资产类别支持的交易所

**期货交易所：**

- `CME`、`CBOT`、`NYMEX`、`COMEX`、`KCBT`、`MGE`、`NYBOT`、`SNFE`

**期权交易所：**

- `SMART`（IB 智能路由）

**外汇交易所：**

- `IDEALPRO`（IB 外汇平台）

**加密货币交易所：**

- `PAXOS`（IB 加密货币平台）

**差价合约/大宗商品交易所：**

- `IBCFD`、`IBCMDTY`（IB 内部路由）

### 选择正确的符号体系方法

- **使用 `IB_SIMPLIFIED`**（默认）适用于大多数场景 - 提供简洁、可读的金融工具 ID
- **使用 `IB_RAW`** 适用于处理复杂的国际金融工具或简化解析失败的情况
- **启用 `convert_exchange_to_mic_venue`** 适用于需要标准化 MIC 交易场所代码以满足合规性或数据一致性要求的情况

## 金融工具与合约

在 Interactive Brokers 中，NautilusTrader 的 `Instrument` 对应于 IB 的 [Contract](https://ibkrcampus.com/ibkr-api-page/trader-workstation-api/#contracts)。适配器处理两种合约表示形式：

### 合约类型

#### 基本合约 (`IBContract`)

- 包含基本的合约识别字段
- 用于合约搜索和基本操作
- 不能直接转换为 NautilusTrader 的 `Instrument`

#### 合约详情 (`IBContractDetails`)

- 包含合约信息，包括：
  - 支持的订单类型
  - 交易时间和日历
  - 保证金要求
  - 价格增量和乘数
  - 市场数据权限
- 可以转换为 NautilusTrader 的 `Instrument`
- 交易操作所必需

### 合约发现

要搜索合约信息，请使用 [IB 合约信息中心](https://pennies.interactivebrokers.com/cstools/contract_info/)。

### 加载金融工具

加载金融工具有两种主要方法：

Interactive Brokers 不支持使用 `load_all=True` 加载完整的 IB 金融工具全集。请为节点在启动时所需的金融工具配置 `load_ids` 或 `load_contracts`，或在订阅某个金融工具的市场数据之前显式请求它。

#### 1. 使用 `load_ids`（推荐）

使用 `symbology_method=SymbologyMethod.IB_SIMPLIFIED`（默认）配合 `load_ids`，获得简洁、直观的金融工具标识：

对于外汇金融工具，请使用斜杠分隔的标的代码，例如 `EUR/USD.IDEALPRO`。带点的本地标的代码形式属于原始符号体系，例如 `EUR.USD=CASH.IDEALPRO`。

```python
from nautilus_trader.adapters.interactive_brokers.config import (
    InteractiveBrokersInstrumentProviderConfig,
)
from nautilus_trader.adapters.interactive_brokers.config import SymbologyMethod

instrument_provider_config = InteractiveBrokersInstrumentProviderConfig(
    symbology_method=SymbologyMethod.IB_SIMPLIFIED,
    load_ids=frozenset(
        [
            "EUR/USD.IDEALPRO",  # 外汇
            "SPY.ARCA",  # 股票
            "ESM24.CME",  # 期货
            "BTC/USD.PAXOS",  # 加密货币
            "^SPX.CBOE",  # 指数
        ]
    ),
)
```

#### 2. 使用 `load_contracts`（用于复杂金融工具）

对于期权/期货链等复杂场景，使用 `load_contracts` 配合 `IBContract` 实例：

```python
from nautilus_trader.adapters.interactive_brokers.common import IBContract

# 加载特定到期日的期权链
options_chain_expiry = IBContract(
    secType="IND",
    symbol="SPX",
    exchange="CBOE",
    build_options_chain=True,
    lastTradeDateOrContractMonth="20240718",
)

# 加载日期范围内的期权链
options_chain_range = IBContract(
    secType="IND",
    symbol="SPX",
    exchange="CBOE",
    build_options_chain=True,
    min_expiry_days=0,
    max_expiry_days=30,
)

# 加载期货链
futures_chain = IBContract(
    secType="CONTFUT",
    exchange="CME",
    symbol="ES",
    build_futures_chain=True,
)

instrument_provider_config = InteractiveBrokersInstrumentProviderConfig(
    load_contracts=frozenset(
        [
            options_chain_expiry,
            options_chain_range,
            futures_chain,
        ]
    ),
)
```

### 各资产类别的 IBContract 示例

```python
from nautilus_trader.adapters.interactive_brokers.common import IBContract

# 股票
IBContract(secType="STK", exchange="SMART", primaryExchange="ARCA", symbol="SPY")
IBContract(secType="STK", exchange="SMART", primaryExchange="NASDAQ", symbol="AAPL")

# 债券
IBContract(secType="BOND", secIdType="ISIN", secId="US03076KAA60")
IBContract(secType="BOND", secIdType="CUSIP", secId="912828XE8")

# 单个期权
IBContract(
    secType="OPT",
    exchange="SMART",
    symbol="SPY",
    lastTradeDateOrContractMonth="20251219",
    strike=500,
    right="C",
)

# 期权链（加载所有行权价/到期日）
IBContract(
    secType="STK",
    exchange="SMART",
    primaryExchange="ARCA",
    symbol="SPY",
    build_options_chain=True,
    min_expiry_days=10,
    max_expiry_days=60,
)

# 差价合约
IBContract(secType="CFD", symbol="IBUS30")
IBContract(secType="CFD", symbol="DE40EUR", exchange="SMART")

# 单个期货
IBContract(secType="FUT", exchange="CME", symbol="ES", lastTradeDateOrContractMonth="20240315")

# 期货链（加载所有到期日）
IBContract(secType="CONTFUT", exchange="CME", symbol="ES", build_futures_chain=True)

# 期货期权（FOP）- 单个
IBContract(
    secType="FOP",
    exchange="CME",
    symbol="ES",
    lastTradeDateOrContractMonth="20240315",
    strike=4200,
    right="C",
)

# 期货期权链（加载所有行权价/到期日）
IBContract(
    secType="CONTFUT",
    exchange="CME",
    symbol="ES",
    build_options_chain=True,
    min_expiry_days=7,
    max_expiry_days=60,
)

# 外汇
IBContract(secType="CASH", exchange="IDEALPRO", symbol="EUR", currency="USD")
IBContract(secType="CASH", exchange="IDEALPRO", symbol="GBP", currency="JPY")

# 加密货币
IBContract(secType="CRYPTO", symbol="BTC", exchange="PAXOS", currency="USD")
IBContract(secType="CRYPTO", symbol="ETH", exchange="PAXOS", currency="USD")

# 指数
IBContract(secType="IND", symbol="SPX", exchange="CBOE")
IBContract(secType="IND", symbol="NDX", exchange="NASDAQ")

# 大宗商品
IBContract(secType="CMDTY", symbol="XAUUSD", exchange="SMART")
```

### 高级配置选项

```python
# 使用自定义交易所的期权链
IBContract(
    secType="STK",
    symbol="AAPL",
    exchange="SMART",
    primaryExchange="NASDAQ",
    build_options_chain=True,
    options_chain_exchange="CBOE",  # 期权使用 CBOE 而非 SMART
    min_expiry_days=7,
    max_expiry_days=45,
)

# 指定月份的期货链
IBContract(
    secType="CONTFUT",
    exchange="NYMEX",
    symbol="CL",  # 原油
    build_futures_chain=True,
    min_expiry_days=30,
    max_expiry_days=180,
)
```

### 连续期货

对于连续期货合约（使用 `secType='CONTFUT'`），适配器仅使用标的代码和交易场所创建金融工具 ID：

```python
# 连续期货示例
IBContract(secType="CONTFUT", exchange="CME", symbol="ES")  # -> ES.CME
IBContract(secType="CONTFUT", exchange="NYMEX", symbol="CL")  # -> CL.NYMEX

# 启用 MIC 交易场所转换
instrument_provider_config = InteractiveBrokersInstrumentProviderConfig(
    convert_exchange_to_mic_venue=True,
)
# 结果为：
# ES.XCME（而非 ES.CME）
# CL.XNYM（而非 CL.NYMEX）
```

**连续期货与单个期货的区别：**

- **连续**：`ES.CME` - 代表近月合约，自动展期
- **单个**：`ESM4.CME` - 特定的 2024 年 3 月合约

:::note
使用 `build_options_chain=True` 或 `build_futures_chain=True` 时，`secType` 和 `symbol` 应指定为标的合约。适配器将自动发现并加载指定到期范围内的所有相关衍生品合约。
:::

## 期权价差

Interactive Brokers 通过 BAG 合约支持期权价差（option spreads），将多个期权腿组合成一个可交易的金融工具。NautilusTrader 提供了创建、加载和交易期权价差的支持。

### 创建期权价差金融工具 ID

期权价差使用 `new_generic_spread_id()` 函数创建，该函数将各个期权腿与其各自的比率组合在一起：

```python
from nautilus_trader.model.identifiers import InstrumentId, new_generic_spread_id

# 创建单个期权金融工具 ID
call_leg = InstrumentId.from_str("SPY C400.SMART")
put_leg = InstrumentId.from_str("SPY P390.SMART")

# 创建 1:1 看涨价差（买入看涨，卖出看涨）
call_spread_id = new_generic_spread_id(
    [
        (call_leg, 1),  # 买入 1 张合约
        (put_leg, -1),  # 卖出 1 张合约
    ]
)

# 创建 1:2 比率价差
ratio_spread_id = new_generic_spread_id(
    [
        (call_leg, 1),  # 买入 1 张合约
        (put_leg, 2),  # 买入 2 张合约
    ]
)
```

### 动态价差加载

期权价差在交易或订阅市场数据之前必须先请求。使用 `request_instrument()` 方法动态加载价差金融工具：

```python
# 在策略的 on_start 方法中
def on_start(self):
    # 请求价差金融工具
    self.request_instrument(spread_id)


def on_instrument(self, instrument):
    # 处理已加载的价差金融工具
    self.log.info(f"Loaded spread: {instrument.id}")

    # 现在可以订阅市场数据
    self.subscribe_quote_ticks(instrument.id)

    # 以及下达订单
    order = self.order_factory.market(
        instrument_id=instrument.id,
        order_side=OrderSide.BUY,
        quantity=instrument.make_qty(1),
        time_in_force=TimeInForce.DAY,
    )
    self.submit_order(order)
```

### 价差交易要求

1. **先加载单个腿**：确保在创建价差之前各个期权腿已可用。
2. **请求价差金融工具**：使用 `request_instrument()` 在交易前加载价差。
3. **订阅市场数据**：在价差加载后请求报价 tick。
4. **下达订单**：价差可用后可使用任何订单类型。

## 历史数据与回测

`HistoricInteractiveBrokersClient` 提供了从 Interactive Brokers 检索历史数据的方法，用于回测和研究目的。

### 支持的数据类型

- **K 线数据**：具有时间、tick 和成交量聚合的 OHLCV K 线。
- **逐笔数据**：具有微秒精度的成交 tick 和报价 tick。
- **金融工具数据**：完整的合约规格和交易规则。

### 历史数据客户端

```python
from nautilus_trader.adapters.interactive_brokers.historical.client import (
    HistoricInteractiveBrokersClient,
)
from ibapi.common import MarketDataTypeEnum

# 初始化客户端
client = HistoricInteractiveBrokersClient(
    host="127.0.0.1",
    port=7497,
    client_id=1,
    market_data_type=MarketDataTypeEnum.DELAYED_FROZEN,  # 无订阅时使用延迟数据
    log_level="INFO",
)

# 连接到 TWS/Gateway
await client.connect()
```

### 检索金融工具

#### 基本金融工具检索

```python
from nautilus_trader.adapters.interactive_brokers.common import IBContract

# 定义合约
contracts = [
    IBContract(secType="STK", symbol="AAPL", exchange="SMART", primaryExchange="NASDAQ"),
    IBContract(secType="STK", symbol="MSFT", exchange="SMART", primaryExchange="NASDAQ"),
    IBContract(secType="CASH", symbol="EUR", currency="USD", exchange="IDEALPRO"),
]

# 请求金融工具定义
instruments = await client.request_instruments(contracts=contracts)
```

#### 期权链检索并存储到目录

你可以在策略中使用 `request_instruments` 下载整个期权链，并通过 `update_catalog=True` 将数据保存到目录这一额外好处：

```python
# 在策略的 on_start 方法中
def on_start(self):
    self.request_instruments(
        venue=IB_VENUE,
        update_catalog=True,
        params={
            "ib_contracts": (
                # SPY 期权
                {
                    "secType": "STK",
                    "symbol": "SPY",
                    "exchange": "SMART",
                    "primaryExchange": "ARCA",
                    "build_options_chain": True,
                    "min_expiry_days": 7,
                    "max_expiry_days": 30,
                },
                # QQQ 期权
                {
                    "secType": "STK",
                    "symbol": "QQQ",
                    "exchange": "SMART",
                    "primaryExchange": "NASDAQ",
                    "build_options_chain": True,
                    "min_expiry_days": 7,
                    "max_expiry_days": 30,
                },
                # ES 期货期权
                {
                    "secType": "CONTFUT",
                    "exchange": "CME",
                    "symbol": "ES",
                    "build_options_chain": True,
                    "min_expiry_days": 0,
                    "max_expiry_days": 60,
                },
                # SPX 指数期权
                {
                    "secType": "IND",
                    "symbol": "SPX",
                    "exchange": "CBOE",
                    "build_options_chain": True,
                    "min_expiry_days": 0,
                    "max_expiry_days": 5,
                },
                # ES 期货链和期货期权
                {
                    "secType": "CONTFUT",
                    "exchange": "CME",
                    "symbol": "ES",
                    "build_futures_chain": True,
                    "build_options_chain": True,
                    "min_expiry_days": 0,
                    "max_expiry_days": 2,
                },
                # ESTX50 指数期权（Eurex）
                {
                    "secType": "IND",
                    "exchange": "EUREX",
                    "symbol": "ESTX50",
                    "build_options_chain": True,
                    "min_expiry_days": 0,
                    "max_expiry_days": 2,
                },
            ),
        },
    )
```

### 检索历史 K 线

```python
import datetime

# 请求历史 K 线
bars = await client.request_bars(
    bar_specifications=[
        "1-MINUTE-LAST",  # 1 分钟 K 线，使用最新价
        "5-MINUTE-MID",  # 5 分钟 K 线，使用中间价
        "1-HOUR-LAST",  # 1 小时 K 线，使用最新价
        "1-DAY-LAST",  # 日 K 线，使用最新价
    ],
    start_date_time=datetime.datetime(2023, 11, 1, 9, 30),
    end_date_time=datetime.datetime(2023, 11, 6, 16, 30),
    tz_name="America/New_York",
    contracts=contracts,
    use_rth=True,  # 仅限常规交易时段
    timeout=120,  # 请求超时时间（秒）
)
```

### 检索历史 tick 数据

```python
# 请求历史 tick 数据（报价 tick 使用 tick_type="TRADES" 或 "BID_ASK"）
ticks = await client.request_ticks(
    tick_type="TRADES",
    start_date_time=datetime.datetime(2023, 11, 6, 9, 30),
    end_date_time=datetime.datetime(2023, 11, 6, 16, 30),
    tz_name="America/New_York",
    contracts=contracts,
    use_rth=True,
    timeout=120,
)
```

### K 线规格

适配器支持各种 K 线规格：

#### 基于时间的 K 线

- `"1-SECOND-LAST"`、`"5-SECOND-LAST"`、`"10-SECOND-LAST"`、`"15-SECOND-LAST"`、`"30-SECOND-LAST"`
- `"1-MINUTE-LAST"`、`"2-MINUTE-LAST"`、`"3-MINUTE-LAST"`、`"5-MINUTE-LAST"`、`"10-MINUTE-LAST"`、`"15-MINUTE-LAST"`、`"20-MINUTE-LAST"`、`"30-MINUTE-LAST"`
- `"1-HOUR-LAST"`、`"2-HOUR-LAST"`、`"3-HOUR-LAST"`、`"4-HOUR-LAST"`、`"8-HOUR-LAST"`
- `"1-DAY-LAST"`、`"1-WEEK-LAST"`、`"1-MONTH-LAST"`

#### 价格类型

- `LAST` - 最新成交价
- `MID` - 买卖价中间值
- `BID` - 买入价
- `ASK` - 卖出价

### 完整示例

```python
import asyncio
import datetime
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.adapters.interactive_brokers.historical.client import (
    HistoricInteractiveBrokersClient,
)
from nautilus_trader.persistence.catalog import ParquetDataCatalog


async def download_historical_data():
    # 初始化客户端
    client = HistoricInteractiveBrokersClient(
        host="127.0.0.1",
        port=7497,
        client_id=5,
    )

    # 连接
    await client.connect()
    await asyncio.sleep(2)  # 等待连接稳定

    # 定义合约
    contracts = [
        IBContract(secType="STK", symbol="AAPL", exchange="SMART", primaryExchange="NASDAQ"),
        IBContract(secType="CASH", symbol="EUR", currency="USD", exchange="IDEALPRO"),
    ]

    # 请求金融工具
    instruments = await client.request_instruments(contracts=contracts)

    # 请求历史 K 线
    bars = await client.request_bars(
        bar_specifications=["1-HOUR-LAST", "1-DAY-LAST"],
        start_date_time=datetime.datetime(2023, 11, 1, 9, 30),
        end_date_time=datetime.datetime(2023, 11, 6, 16, 30),
        tz_name="America/New_York",
        contracts=contracts,
        use_rth=True,
    )

    # 请求 tick 数据
    ticks = await client.request_ticks(
        tick_type="TRADES",
        start_date_time=datetime.datetime(2023, 11, 6, 14, 0),
        end_date_time=datetime.datetime(2023, 11, 6, 15, 0),
        tz_name="America/New_York",
        contracts=contracts,
    )

    # 保存到目录
    catalog = ParquetDataCatalog("./catalog")
    catalog.write_data(instruments)
    catalog.write_data(bars)
    catalog.write_data(ticks)

    print(f"Downloaded {len(instruments)} instruments")
    print(f"Downloaded {len(bars)} bars")
    print(f"Downloaded {len(ticks)} ticks")

    # 断开连接
    await client.disconnect()


# 运行示例
if __name__ == "__main__":
    asyncio.run(download_historical_data())
```

### 数据限制

请注意 Interactive Brokers 的历史数据限制：

- **频率限制**：IB 对历史数据请求强制执行频率限制
- **数据可用性**：历史数据可用性因金融工具和订阅级别而异
- **市场数据权限**：某些数据需要特定的市场数据订阅
- **时间范围**：最大回溯期因 K 线大小和金融工具类型而异

### 最佳实践

1. **使用延迟数据**：对于回测，`MarketDataTypeEnum.DELAYED_FROZEN` 通常就足够了
2. **批量请求**：尽可能在单个请求中对多个金融工具进行分组
3. **处理超时**：为大数据请求设置适当的超时值
4. **尊重频率限制**：在请求之间添加延迟以避免触发频率限制
5. **验证数据**：在回测之前始终检查数据质量和完整性

:::warning
Interactive Brokers 强制执行节奏限制；过多的历史数据或订单请求会触发节奏违规，IB 可能会禁用 API 会话数分钟。
:::

## 实盘交易

使用 Interactive Brokers 进行实盘交易需要设置一个 `TradingNode`，其中包含 `InteractiveBrokersDataClient` 和 `InteractiveBrokersExecutionClient`。这些客户端依赖 `InteractiveBrokersInstrumentProvider` 进行金融工具管理。

### 架构概述

实盘交易设置由三个主要组件组成：

1. **InstrumentProvider**：管理金融工具定义和合约详情
2. **DataClient**：处理实时市场数据订阅
3. **ExecutionClient**：管理订单、持仓和账户信息

### InstrumentProvider 配置

`InteractiveBrokersInstrumentProvider` 提供对 IB 金融工具数据的访问。它支持加载单个金融工具、期权链和期货链。

#### 基本配置

```python
from nautilus_trader.adapters.interactive_brokers.config import (
    InteractiveBrokersInstrumentProviderConfig,
)
from nautilus_trader.adapters.interactive_brokers.config import SymbologyMethod
from nautilus_trader.adapters.interactive_brokers.common import IBContract

instrument_provider_config = InteractiveBrokersInstrumentProviderConfig(
    symbology_method=SymbologyMethod.IB_SIMPLIFIED,
    build_futures_chain=False,  # 设置为 True 以获取期货链
    build_options_chain=False,  # 设置为 True 以获取期权链
    min_expiry_days=10,  # 衍生品最小到期天数
    max_expiry_days=60,  # 衍生品最大到期天数
    convert_exchange_to_mic_venue=False,  # 使用 MIC 代码进行交易场所映射
    cache_validity_days=1,  # 缓存金融工具数据 1 天
    load_ids=frozenset(
        [
            # 使用简化符号体系的单个金融工具
            "EUR/USD.IDEALPRO",  # 外汇
            "BTC/USD.PAXOS",  # 加密货币
            "SPY.ARCA",  # 股票 ETF
            "V.NYSE",  # 个股
            "ESM4.CME",  # 期货合约（一位数年份）
            "^SPX.CBOE",  # 指数
        ]
    ),
    load_contracts=frozenset(
        [
            # 使用 IBContract 的复杂金融工具
            IBContract(secType="STK", symbol="AAPL", exchange="SMART", primaryExchange="NASDAQ"),
            IBContract(secType="CASH", symbol="GBP", currency="USD", exchange="IDEALPRO"),
        ]
    ),
)
```

#### 衍生品高级配置

```python
# 期权和期货链的配置
advanced_config = InteractiveBrokersInstrumentProviderConfig(
    symbology_method=SymbologyMethod.IB_SIMPLIFIED,
    build_futures_chain=True,  # 启用期货链加载
    build_options_chain=True,  # 启用期权链加载
    min_expiry_days=7,  # 加载 7 天以上到期的合约
    max_expiry_days=90,  # 加载 90 天内到期的合约
    load_contracts=frozenset(
        [
            # 加载 SPY 期权链
            IBContract(
                secType="STK",
                symbol="SPY",
                exchange="SMART",
                primaryExchange="ARCA",
                build_options_chain=True,
            ),
            # 加载 ES 期货链
            IBContract(
                secType="CONTFUT",
                exchange="CME",
                symbol="ES",
                build_futures_chain=True,
            ),
        ]
    ),
)
```

#### 过滤证券类型

使用 `filter_sec_types` 忽略特定的 IB `secType` 值。任何 `secType` 匹配此 frozenset 中条目的合约都会被跳过并记录警告（例如 `WAR` 或 `IOPT` 等不受支持的类型）：

```python
instrument_provider_config = InteractiveBrokersInstrumentProviderConfig(
    load_ids=frozenset(["SPY.ARCA"]),
    filter_sec_types=frozenset({"WAR", "IOPT"}),  # 排除不受支持的资产类型
)
```

### 与外部数据提供商的集成

Interactive Brokers 适配器可以与其他数据提供商一起使用，以增强市场数据覆盖范围。使用多个数据源时：

- 在各提供商之间使用一致的符号体系方法
- 考虑使用 `convert_exchange_to_mic_venue=True` 实现标准化的交易场所识别
- 确保正确处理金融工具缓存管理以避免冲突

### 数据客户端配置

`InteractiveBrokersDataClient` 与 IB 交互，用于流式传输和检索实时市场数据。连接后，它会配置[市场数据类型](https://ibkrcampus.com/ibkr-api-page/trader-workstation-api/#delayed-market-data)，并根据 `InteractiveBrokersInstrumentProviderConfig` 设置加载金融工具。

#### 支持的数据类型

- **报价 Tick**：实时买卖价格和数量
- **成交 Tick**：实时成交价格和成交量
- **K 线数据**：实时 OHLCV K 线（1 秒到 1 天间隔）
- **市场深度**：Level 2 订单簿数据（在可用的情况下）

#### 市场数据类型

Interactive Brokers 支持几种市场数据类型：

- `REALTIME`：实时市场数据（需要市场数据订阅）
- `DELAYED`：15-20 分钟延迟数据（大多数市场免费）
- `DELAYED_FROZEN`：不更新的延迟数据（适用于测试）
- `FROZEN`：最后已知的实时数据（市场关闭时）

#### 基本数据客户端配置

```python
from nautilus_trader.adapters.interactive_brokers.config import IBMarketDataTypeEnum
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersDataClientConfig

data_client_config = InteractiveBrokersDataClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=7497,  # TWS 模拟交易端口
    ibg_client_id=1,
    use_regular_trading_hours=True,  # 仅限股票常规交易时段
    market_data_type=IBMarketDataTypeEnum.DELAYED_FROZEN,  # 使用延迟数据
    ignore_quote_tick_size_updates=False,  # 包含仅数量变化的更新
    instrument_provider=instrument_provider_config,
    connection_timeout=300,  # 5 分钟
    request_timeout_secs=60,  # 1 分钟
)
```

#### 高级数据客户端配置

```python
# 使用实时数据的生产环境配置
production_data_config = InteractiveBrokersDataClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=4001,  # IB Gateway 实盘交易端口
    ibg_client_id=1,
    use_regular_trading_hours=False,  # 包含盘前盘后时段
    market_data_type=IBMarketDataTypeEnum.REALTIME,  # 实时数据
    ignore_quote_tick_size_updates=True,  # 减少 tick 数据量
    handle_revised_bars=True,  # 处理 K 线修订
    instrument_provider=instrument_provider_config,
    dockerized_gateway=dockerized_gateway_config,  # 如果使用 Docker
    connection_timeout=300,
    request_timeout_secs=60,
)
```

### 数据客户端配置选项

| 选项                               | 默认值                                            | 描述                                                             |
| -------------------------------- | ---------------------------------------------- | -------------------------------------------------------------- |
| `instrument_provider`            | `InteractiveBrokersInstrumentProviderConfig()` | 金融工具提供者设置，控制启动时加载哪些合约。                                         |
| `ibg_host`                       | `127.0.0.1`                                    | TWS/IB Gateway 的主机名或 IP。                                       |
| `ibg_port`                       | `None`                                         | TWS/IB Gateway 的端口（`7497`/`7496` 用于 TWS，`4002`/`4001` 用于 IBG）。 |
| `ibg_client_id`                  | `1`                                            | 连接到 TWS/IB Gateway 时使用的唯一客户端标识符。                               |
| `use_regular_trading_hours`      | `True`                                         | 为 `True` 时，请求限于常规交易时段的 K 线。                                    |
| `market_data_type`               | `REALTIME`                                     | 市场数据类型（`REALTIME`、`DELAYED`、`DELAYED_FROZEN` 等）。               |
| `ignore_quote_tick_size_updates` | `False`                                        | 为 `True` 时，过滤仅数量变化的报价 tick。                                    |
| `handle_revised_bars`            | `False`                                        | 为 `True` 时，处理来自 IB 的 K 线修订（K 线在初次发布后可能会更新）。                    |
| `dockerized_gateway`             | `None`                                         | 可选的 `DockerizedIBGatewayConfig`，用于容器化设置。                       |
| `connection_timeout`             | `300`                                          | 等待初始 API 连接的秒数。                                                |
| `request_timeout_secs`           | `60`                                           | 历史数据请求超时前等待的秒数。                                                |

#### 说明

- **`use_regular_trading_hours`**：为 `True` 时，仅在常规交易时段请求数据。主要影响股票的 K 线数据。
- **`ignore_quote_tick_size_updates`**：为 `True` 时，过滤掉仅数量变化（非价格变化）的报价 tick，减少数据量。
- **`handle_revised_bars`**：为 `True` 时，处理来自 IB 的 K 线修订（K 线在初次发布后可能会更新）。
- **`connection_timeout`**：等待初始连接建立的最大时间。
- **`request_timeout_secs`**：等待历史数据请求的最大时间。

### 执行客户端配置选项

| 选项                                           | 默认值                                            | 描述                                                             |
| -------------------------------------------- | ---------------------------------------------- | -------------------------------------------------------------- |
| `instrument_provider`                        | `InteractiveBrokersInstrumentProviderConfig()` | 金融工具提供者设置，控制启动时加载哪些合约。                                         |
| `ibg_host`                                   | `127.0.0.1`                                    | TWS/IB Gateway 的主机名或 IP。                                       |
| `ibg_port`                                   | `None`                                         | TWS/IB Gateway 的端口（`7497`/`7496` 用于 TWS，`4002`/`4001` 用于 IBG）。 |
| `ibg_client_id`                              | `1`                                            | 连接到 TWS/IB Gateway 时使用的唯一客户端标识符。                               |
| `account_id`                                 | `None`                                         | Interactive Brokers 账户标识符（回退到 `TWS_ACCOUNT` 环境变量）。             |
| `dockerized_gateway`                         | `None`                                         | 可选的 `DockerizedIBGatewayConfig`，用于容器化设置。                       |
| `connection_timeout`                         | `300`                                          | 等待初始 API 连接的秒数。                                                |
| `request_timeout_secs`                       | `60`                                           | 等待请求响应（合约详情等）的秒数。                                              |
| `fetch_all_open_orders`                      | `False`                                        | 为 `True` 时，拉取所有 API 客户端 ID 的未完成订单（不仅是当前会话）。                    |
| `track_option_exercise_from_position_update` | `False`                                        | 为 `True` 时，订阅实时持仓更新以检测期权行权。                                    |

### 执行客户端配置

`InteractiveBrokersExecutionClient` 处理交易执行、订单管理、账户信息和持仓跟踪。它提供订单生命周期管理和实时账户更新。

#### 支持的功能

- **订单管理**：下达、修改和撤销订单
- **订单类型**：市价、限价、止损、止损限价、追踪止损等
- **账户信息**：实时余额和保证金更新
- **持仓跟踪**：实时持仓更新和盈亏
- **交易报告**：执行报告和成交通知
- **风险管理**：交易前风险检查和持仓限制

#### 支持的订单类型

适配器支持大多数 Interactive Brokers 订单类型：

- **市价订单**：`OrderType.MARKET`
- **限价订单**：`OrderType.LIMIT`
- **止损订单**：`OrderType.STOP_MARKET`
- **止损限价订单**：`OrderType.STOP_LIMIT`
- **触及市价订单**：`OrderType.MARKET_IF_TOUCHED`
- **触及限价订单**：`OrderType.LIMIT_IF_TOUCHED`
- **追踪止损市价订单**：`OrderType.TRAILING_STOP_MARKET`
- **追踪止损限价订单**：`OrderType.TRAILING_STOP_LIMIT`
- **收盘市价订单**：`OrderType.MARKET` 配合 `TimeInForce.AT_THE_CLOSE`
- **收盘限价订单**：`OrderType.LIMIT` 配合 `TimeInForce.AT_THE_CLOSE`

#### 有效期选项

- **当日有效**：`TimeInForce.DAY`
- **撤销前有效**：`TimeInForce.GTC`
- **立即成交否则撤销**：`TimeInForce.IOC`
- **全部成交否则撤销**：`TimeInForce.FOK`
- **指定日期前有效**：`TimeInForce.GTD`
- **开盘时有效**：`TimeInForce.AT_THE_OPEN`
- **收盘时有效**：`TimeInForce.AT_THE_CLOSE`

#### 批量操作

| 操作   | 支持  | 说明            |
| ---- | --- | ------------- |
| 批量提交 | ✓   | 在单次请求中提交多个订单。 |
| 批量修改 | ✓   | 在单次请求中修改多个订单。 |
| 批量撤销 | ✓   | 在单次请求中撤销多个订单。 |

#### 持仓管理

| 功能    | 支持  | 说明           |
| ----- | --- | ------------ |
| 查询持仓  | ✓   | 实时持仓更新。      |
| 持仓模式  | ✓   | 净持仓与分别多空持仓。  |
| 杠杆控制  | ✓   | 账户级别保证金要求。   |
| 保证金模式 | ✓   | 组合保证金与单独保证金。 |

#### 订单查询

| 功能      | 支持  | 说明        |
| ------- | --- | --------- |
| 查询未完成订单 | ✓   | 列出所有活跃订单。 |
| 查询订单历史  | ✓   | 历史订单数据。   |
| 订单状态更新  | ✓   | 实时订单状态变化。 |
| 交易历史    | ✓   | 执行和成交报告。  |

#### 条件订单

| 功能     | 支持  | 说明                        |
| ------ | --- | ------------------------- |
| 订单列表   | ✓   | 原子化多订单提交。                 |
| OCO 订单 | ✓   | 可自定义 OCA 类型（1、2、3）的二择一订单。 |
| 括号订单   | ✓   | 父子订单关系。                   |
| 条件订单   | ✓   | 高级订单条件和触发器。               |

#### 基本执行客户端配置

```python
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersExecClientConfig
from nautilus_trader.config import RoutingConfig

exec_client_config = InteractiveBrokersExecClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=7497,  # TWS 模拟交易端口
    ibg_client_id=1,
    account_id="DU123456",  # 你的 IB 账户 ID（模拟或实盘）
    instrument_provider=instrument_provider_config,
    connection_timeout=300,
    routing=RoutingConfig(default=True),  # 通过此客户端路由所有订单
)
```

#### 高级执行客户端配置

```python
# 使用 Docker 化网关的生产环境配置
production_exec_config = InteractiveBrokersExecClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=4001,  # IB Gateway 实盘交易端口
    ibg_client_id=1,
    account_id=None,  # 将使用 TWS_ACCOUNT 环境变量
    instrument_provider=instrument_provider_config,
    dockerized_gateway=dockerized_gateway_config,
    connection_timeout=300,
    routing=RoutingConfig(default=True),
)
```

#### 账户 ID 配置

`account_id` 参数至关重要，必须与登录 TWS/Gateway 的账户匹配：

```python
# 选项 1：在配置中直接指定
exec_config = InteractiveBrokersExecClientConfig(
    account_id="DU123456",  # 模拟交易账户
    # ... 其他参数
)

# 选项 2：使用环境变量
import os

os.environ["TWS_ACCOUNT"] = "DU123456"
exec_config = InteractiveBrokersExecClientConfig(
    account_id=None,  # 将使用 TWS_ACCOUNT 环境变量
    # ... 其他参数
)
```

#### 订单参数

执行适配器在订单提交、订单列表提交和订单修改命令上支持 `params["exchange"]`。使用它可以为当前订单的路由覆盖 IB 合约交易所，同时保留缓存的金融工具合约：

```python
self.submit_order(order, params={"exchange": "IEX"})
```

将 `exchange` 保持未设置，或将其设置为空字符串，以使用缓存的合约交易所。

#### 订单标签和高级功能

适配器通过订单标签支持 IB 特定的订单参数：

```python
from nautilus_trader.adapters.interactive_brokers.common import IBOrderTags

# 使用 IB 特定参数创建订单
order_tags = IBOrderTags(
    allOrNone=True,  # 全部成交或全部不成交
    ocaGroup="MyGroup1",  # 全部撤销组
    ocaType=1,  # 带阻塞的全部撤销
    activeStartTime="20240315 09:30:00 EST",  # GTC 激活时间
    activeStopTime="20240315 16:00:00 EST",  # GTC 停用时间
    goodAfterTime="20240315 09:35:00 EST",  # 指定时间后有效
)

# 将标签应用于订单
order = order_factory.limit(
    instrument_id=instrument.id,
    order_side=OrderSide.BUY,
    quantity=instrument.make_qty(100),
    price=instrument.make_price(100.0),
    tags=[order_tags.value],
)
```

#### OCA（全部撤销）订单

适配器通过 `IBOrderTags` 的显式配置为 OCA 订单提供支持：

### 基本 OCA 配置

所有 OCA 功能必须通过 `IBOrderTags` 显式配置：

```python
from nautilus_trader.adapters.interactive_brokers.common import IBOrderTags

# 创建 OCA 配置
oca_tags = IBOrderTags(
    ocaGroup="MY_OCA_GROUP",
    ocaType=1,  # 类型 1：带阻塞的全部撤销（推荐）
)

# 应用于括号订单
bracket_order = order_factory.bracket(
    instrument_id=instrument.id,
    order_side=OrderSide.BUY,
    quantity=instrument.make_qty(100),
    tp_price=instrument.make_price(110.0),
    sl_trigger_price=instrument.make_price(90.0),
    tp_tags=[oca_tags.value],  # 必须显式添加 OCA 标签
    sl_tags=[oca_tags.value],  # 必须显式添加 OCA 标签
)
```

### 高级 OCA 配置

你可以使用 `IBOrderTags` 指定不同的 OCA 类型和行为：

```python
from nautilus_trader.adapters.interactive_brokers.common import IBOrderTags

# 创建自定义 OCA 配置
custom_oca_tags = IBOrderTags(
    ocaGroup="MY_CUSTOM_GROUP",
    ocaType=2,  # 使用类型 2：带阻塞的按比例缩减
)

# 应用于单个订单
order = order_factory.limit(
    instrument_id=instrument.id,
    order_side=OrderSide.BUY,
    quantity=instrument.make_qty(100),
    price=instrument.make_price(100.0),
    tags=[custom_oca_tags.value],
)
```

### OCA 类型

Interactive Brokers 支持三种 OCA 类型：

| 类型    | 名称         | 行为              | 用例                     |
| ----- | ---------- | --------------- | ---------------------- |
| **1** | 带阻塞的全部撤销   | 带阻塞保护撤销所有剩余订单   | **默认** - 最安全的选项，防止超额成交 |
| **2** | 带阻塞的按比例缩减  | 带阻塞保护按比例缩减剩余订单  | 部分成交并提供超额成交保护          |
| **3** | 不带阻塞的按比例缩减 | 不带阻塞保护按比例缩减剩余订单 | 最快执行，超额成交风险较高          |

#### 同一 OCA 组中的多个订单

```python
# 创建具有相同 OCA 组的多个订单
oca_tags = IBOrderTags(
    ocaGroup="MULTI_ORDER_GROUP",
    ocaType=3,  # 使用类型 3：不带阻塞的按比例缩减
)

order1 = order_factory.limit(
    instrument_id=instrument.id,
    order_side=OrderSide.BUY,
    quantity=instrument.make_qty(50),
    price=instrument.make_price(99.0),
    tags=[oca_tags.value],
)

order2 = order_factory.limit(
    instrument_id=instrument.id,
    order_side=OrderSide.BUY,
    quantity=instrument.make_qty(50),
    price=instrument.make_price(101.0),
    tags=[oca_tags.value],
)
```

### OCA 配置要求

OCA 功能**仅**通过显式配置可用：

1. **必须使用 IBOrderTags** - OCA 设置必须在订单标签中显式指定
2. **无自动检测** - `ContingencyType.OCO` 和 `ContingencyType.OUO` 不会自动创建 OCA 组
3. **手动配置** - 所有 OCA 组和类型必须手动指定

### 条件订单

适配器通过 `IBOrderTags` 中的 `conditions` 参数支持 Interactive Brokers 条件订单。条件订单允许你指定在订单传输或撤销之前必须满足的标准。

#### 支持的条件类型

- **价格条件**：基于特定金融工具的价格变动触发
- **时间条件**：在特定日期和时间触发
- **成交量条件**：基于交易量阈值触发
- **执行条件**：当特定金融工具发生交易时触发
- **保证金条件**：基于账户保证金水平触发
- **百分比变化条件**：基于价格百分比变化触发

#### 基本条件订单示例

```python
from nautilus_trader.adapters.interactive_brokers.common import IBOrderTags

# 创建价格条件：当 SPY 超过 $250 时触发
price_condition = {
    "type": "price",
    "conId": 265598,  # SPY 合约 ID
    "exchange": "SMART",
    "isMore": True,  # 价格大于阈值时触发
    "price": 250.00,
    "triggerMethod": 0,  # 默认触发方法
    "conjunction": "and",
}

# 创建带条件的订单标签
order_tags = IBOrderTags(
    conditions=[price_condition],
    conditionsCancelOrder=False,  # 条件满足时传输订单
)

# 应用于订单
order = order_factory.limit(
    instrument_id=instrument.id,
    order_side=OrderSide.BUY,
    quantity=instrument.make_qty(100),
    price=instrument.make_price(251.00),
    tags=[order_tags.value],
)
```

#### 多条件与逻辑运算

```python
# 创建带 AND/OR 逻辑的多个条件
conditions = [
    {
        "type": "price",
        "conId": 265598,
        "exchange": "SMART",
        "isMore": True,
        "price": 250.00,
        "triggerMethod": 0,
        "conjunction": "and",  # 与下一个条件为 AND 关系
    },
    {
        "type": "time",
        "time": "20250315-09:30:00",
        "isMore": True,
        "conjunction": "or",  # 与下一个条件为 OR 关系
    },
    {
        "type": "volume",
        "conId": 265598,
        "exchange": "SMART",
        "isMore": True,
        "volume": 10000000,
        "conjunction": "and",
    },
]

order_tags = IBOrderTags(
    conditions=conditions,
    conditionsCancelOrder=False,
)
```

#### 条件参数

**价格条件：**

- `conId`：要监控的金融工具的合约 ID
- `exchange`：要监控的交易所（例如 "SMART"、"NASDAQ"）
- `isMore`：True 表示 >=，False 表示 <=
- `price`：价格阈值
- `triggerMethod`：0=默认、1=双向买卖价、2=最新价、3=双向最新价、4=买卖价、7=最新买卖价、8=中间价

**时间条件：**

- `time`：UTC 格式的时间字符串 "YYYYMMDD-HH:MM:SS"（例如 "20250315-09:30:00"）
- `isMore`：True 表示在该时间之后，False 表示在该时间之前

**成交量条件：**

- `conId`：要监控的金融工具的合约 ID
- `exchange`：要监控的交易所
- `isMore`：True 表示 >=，False 表示 <=
- `volume`：成交量阈值

**执行条件：**

- `symbol`：要监控交易的标的代码
- `secType`：证券类型（例如 "STK"、"OPT"、"FUT"）
- `exchange`：要监控的交易所

**保证金条件：**

- `percent`：保证金缓冲百分比阈值
- `isMore`：True 表示 >=，False 表示 <=

**百分比变化条件：**

- `conId`：要监控的金融工具的合约 ID
- `exchange`：要监控的交易所
- `isMore`：True 表示 >=，False 表示 <=
- `changePercent`：百分比变化阈值

#### 完整示例：所有条件类型

```python
# 展示所有 6 种支持的条件类型的示例
from nautilus_trader.adapters.interactive_brokers.common import IBOrderTags

# 1. 价格条件 - 当 ES 期货 > 6000 时触发
price_condition = {
    "type": "price",
    "conId": 495512563,  # ES 期货合约 ID
    "exchange": "CME",
    "isMore": True,
    "price": 6000.0,
    "triggerMethod": 0,
    "conjunction": "and",
}

# 2. 时间条件 - 在特定时间触发
time_condition = {
    "type": "time",
    "time": "20250315-09:30:00",  # UTC 格式
    "isMore": True,
    "conjunction": "and",
}

# 3. 成交量条件 - 当成交量 > 100,000 时触发
volume_condition = {
    "type": "volume",
    "conId": 495512563,
    "exchange": "CME",
    "isMore": True,
    "volume": 100000,
    "conjunction": "and",
}

# 4. 执行条件 - 当 SPY 发生交易时触发
execution_condition = {
    "type": "execution",
    "symbol": "SPY",
    "secType": "STK",
    "exchange": "SMART",
    "conjunction": "and",
}

# 5. 保证金条件 - 当保证金缓冲 > 75% 时触发
margin_condition = {
    "type": "margin",
    "percent": 75,
    "isMore": True,
    "conjunction": "and",
}

# 6. 百分比变化条件 - 当价格变化 > 5% 时触发
percent_change_condition = {
    "type": "percent_change",
    "conId": 495512563,
    "exchange": "CME",
    "changePercent": 5.0,
    "isMore": True,
    "conjunction": "and",
}

# 使用任意条件组合
order_tags = IBOrderTags(
    conditions=[price_condition, time_condition],  # 多个条件
    conditionsCancelOrder=False,  # 条件满足时传输
)
```

#### 订单行为

设置 `conditionsCancelOrder` 以控制条件满足时的行为：

- `False`：条件满足时传输订单
- `True`：条件满足时撤销订单

#### 实现说明

- **所有 6 种条件类型均已完全支持**，并已在 Interactive Brokers 实盘订单中测试
- **价格条件**可正常工作，尽管 ibapi 库中存在一个已知 bug，`PriceCondition.__str__` 被错误地装饰为 property
- **时间条件**使用带破折号分隔符的 UTC 格式（`YYYYMMDD-HH:MM:SS`）以确保可靠解析
- **连接逻辑**允许使用 "and"/"or" 运算符进行复杂的条件组合

### 完整交易节点配置

设置完整的交易环境需要配置 `TradingNodeConfig` 及其所有必要组件。以下是针对不同场景的示例。

#### 模拟交易配置

```python
import os
from nautilus_trader.adapters.interactive_brokers.common import IB
from nautilus_trader.adapters.interactive_brokers.common import IB_VENUE
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersDataClientConfig
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersExecClientConfig
from nautilus_trader.adapters.interactive_brokers.config import (
    InteractiveBrokersInstrumentProviderConfig,
)
from nautilus_trader.adapters.interactive_brokers.config import IBMarketDataTypeEnum
from nautilus_trader.adapters.interactive_brokers.config import SymbologyMethod
from nautilus_trader.adapters.interactive_brokers.factories import (
    InteractiveBrokersLiveDataClientFactory,
)
from nautilus_trader.adapters.interactive_brokers.factories import (
    InteractiveBrokersLiveExecClientFactory,
)
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.live.node import TradingNode

# 金融工具提供者配置
instrument_provider_config = InteractiveBrokersInstrumentProviderConfig(
    symbology_method=SymbologyMethod.IB_SIMPLIFIED,
    load_ids=frozenset(
        [
            "EUR/USD.IDEALPRO",
            "GBP/USD.IDEALPRO",
            "SPY.ARCA",
            "QQQ.NASDAQ",
            "AAPL.NASDAQ",
            "MSFT.NASDAQ",
        ]
    ),
)

# 数据客户端配置
data_client_config = InteractiveBrokersDataClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=7497,  # TWS 模拟交易
    ibg_client_id=1,
    use_regular_trading_hours=True,
    market_data_type=IBMarketDataTypeEnum.DELAYED_FROZEN,
    instrument_provider=instrument_provider_config,
)

# 执行客户端配置
exec_client_config = InteractiveBrokersExecClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=7497,  # TWS 模拟交易
    ibg_client_id=1,
    account_id="DU123456",  # 你的模拟交易账户
    instrument_provider=instrument_provider_config,
    routing=RoutingConfig(default=True),
)

# 交易节点配置
config_node = TradingNodeConfig(
    trader_id="PAPER-TRADER-001",
    logging=LoggingConfig(log_level="INFO"),
    data_clients={IB: data_client_config},
    exec_clients={IB: exec_client_config},
    data_engine=LiveDataEngineConfig(
        time_bars_timestamp_on_close=False,  # IB 标准：使用 K 线开盘时间
        validate_data_sequence=True,  # 丢弃乱序 K 线
    ),
    timeout_connection=90.0,
    timeout_reconciliation=5.0,
    timeout_portfolio=5.0,
    timeout_disconnection=5.0,
    timeout_post_stop=2.0,
)

# 创建并配置交易节点
node = TradingNode(config=config_node)
node.add_data_client_factory(IB, InteractiveBrokersLiveDataClientFactory)
node.add_exec_client_factory(IB, InteractiveBrokersLiveExecClientFactory)
node.build()

if __name__ == "__main__":
    try:
        node.run()
    finally:
        node.dispose()
```

## 使用 Docker 化网关进行实盘交易

```python
from nautilus_trader.adapters.interactive_brokers.config import DockerizedIBGatewayConfig

# Docker 化网关配置
dockerized_gateway_config = DockerizedIBGatewayConfig(
    username=os.environ.get("TWS_USERNAME"),
    password=os.environ.get("TWS_PASSWORD"),
    trading_mode="live",  # "paper" 或 "live"
    read_only_api=False,  # 允许订单执行
    timeout=300,
)

# 使用 Docker 化网关的数据客户端
data_client_config = InteractiveBrokersDataClientConfig(
    ibg_client_id=1,
    use_regular_trading_hours=False,  # 包含盘前盘后时段
    market_data_type=IBMarketDataTypeEnum.REALTIME,
    instrument_provider=instrument_provider_config,
    dockerized_gateway=dockerized_gateway_config,
)

# 使用 Docker 化网关的执行客户端
exec_client_config = InteractiveBrokersExecClientConfig(
    ibg_client_id=1,
    account_id=os.environ.get("TWS_ACCOUNT"),  # 实盘账户 ID
    instrument_provider=instrument_provider_config,
    dockerized_gateway=dockerized_gateway_config,
    routing=RoutingConfig(default=True),
)

# 实盘交易节点配置
config_node = TradingNodeConfig(
    trader_id="LIVE-TRADER-001",
    logging=LoggingConfig(log_level="INFO"),
    data_clients={IB: data_client_config},
    exec_clients={IB: exec_client_config},
    data_engine=LiveDataEngineConfig(
        time_bars_timestamp_on_close=False,
        validate_data_sequence=True,
    ),
)
```

### 多客户端配置

对于高级设置，你可以配置具有不同用途的多个客户端：

```python
# 使用不同客户端 ID 分离数据和执行客户端
data_client_config = InteractiveBrokersDataClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=7497,
    ibg_client_id=1,  # 数据客户端使用 ID 1
    market_data_type=IBMarketDataTypeEnum.REALTIME,
    instrument_provider=instrument_provider_config,
)

exec_client_config = InteractiveBrokersExecClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=7497,
    ibg_client_id=2,  # 执行客户端使用 ID 2
    account_id="DU123456",
    instrument_provider=instrument_provider_config,
    routing=RoutingConfig(default=True),
)
```

### 用于不同账户的多个 IB 执行客户端

NautilusTrader 支持同时使用多个 Interactive Brokers 执行客户端，每个客户端连接到不同的 IB 账户。当你需要使用多个账户进行交易时，这很有用，例如：

- 为不同策略使用独立账户
- 同时运行模拟交易和实盘交易账户
- 同一 IB 登录下的多个受管账户

要配置多个 IB 执行客户端，请在 `exec_clients` 字典中提供具有唯一键的多个条目。每个条目指定不同的 `account_id`：

```python
from nautilus_trader.adapters.interactive_brokers.config import (
    InteractiveBrokersDataClientConfig,
    InteractiveBrokersExecClientConfig,
    InteractiveBrokersInstrumentProviderConfig,
    SymbologyMethod,
    IBMarketDataTypeEnum,
)
from nautilus_trader.live.config import TradingNodeConfig, RoutingConfig, LoggingConfig
from nautilus_trader.model.identifiers import AccountId, Venue, ClientId

# 共享的金融工具提供者配置
instrument_provider_config = InteractiveBrokersInstrumentProviderConfig(
    symbology_method=SymbologyMethod.IB_SIMPLIFIED,
)

# 数据客户端（所有账户共享）
data_client_config = InteractiveBrokersDataClientConfig(
    ibg_host="127.0.0.1",
    ibg_port=7497,
    ibg_client_id=1,
    market_data_type=IBMarketDataTypeEnum.REALTIME,
    instrument_provider=instrument_provider_config,
)

# 多个 IB 执行客户端的配置
config_node = TradingNodeConfig(
    trader_id="MULTI-ACCOUNT-001",
    logging=LoggingConfig(log_level="INFO"),
    # 所有账户共享单个数据客户端
    data_clients={
        "IB": data_client_config,
    },
    # 多个执行客户端，每个账户一个
    exec_clients={
        # 第一个账户：模拟交易账户
        "IB-PAPER": InteractiveBrokersExecClientConfig(
            ibg_host="127.0.0.1",
            ibg_port=7497,
            ibg_client_id=2,  # 唯一的 IB API 客户端 ID
            account_id="DU123456",  # 模拟交易账户 ID
            instrument_provider=instrument_provider_config,
            routing=RoutingConfig(default=False),  # 非默认
        ),
        # 第二个账户：实盘交易账户
        "IB-LIVE": InteractiveBrokersExecClientConfig(
            ibg_host="127.0.0.1",
            ibg_port=7497,
            ibg_client_id=3,  # 唯一的 IB API 客户端 ID
            account_id="U987654",  # 实盘账户 ID
            instrument_provider=instrument_provider_config,
            routing=RoutingConfig(default=True),  # 设为默认
        ),
        # 第三个账户：另一个受管账户
        "IB-ACCOUNT3": InteractiveBrokersExecClientConfig(
            ibg_host="127.0.0.1",
            ibg_port=7497,
            ibg_client_id=4,  # 唯一的 IB API 客户端 ID
            account_id="U456789",  # 另一个账户 ID
            instrument_provider=instrument_provider_config,
            routing=RoutingConfig(default=False),
        ),
    },
)
```

**多个 IB 执行客户端的要点：**

1. **唯一键**：`exec_clients` 中的每个条目必须有唯一的键（例如 `"IB-PAPER"`、`"IB-LIVE"`）。此键将成为该客户端的 `account_issuer`。

2. **唯一客户端 ID**：每个执行客户端必须使用不同的 `ibg_client_id`（2、3、4 等）。IB Gateway/TWS 要求每个 API 连接使用唯一的客户端 ID。

3. **账户 ID**：每个执行客户端必须指定与登录 IB Gateway/TWS 的账户匹配的不同 `account_id`。

4. **账户标识符**：系统会创建如下的 `AccountId` 实例：
   - `AccountId("IB-PAPER-DU123456")`
   - `AccountId("IB-LIVE-U987654")`
   - `AccountId("IB-ACCOUNT3-U456789")`

5. **路由**：订单和查询会根据以下条件自动路由到正确的执行客户端：
   - 命令中显式的 `client_id`
   - `account_id` 签发者（对于 `QueryAccount` 命令或设置了 account_id 的订单）
   - 默认客户端（如果某个客户端标记为 `routing=RoutingConfig(default=True)`）

6. **投资组合查询**：查询投资组合属性时，你可以指定：
   - `account_id` 用于账户特定查询：`portfolio.realized_pnls(account_id=AccountId("IB-PAPER-DU123456"))`
   - `venue` 用于跨该交易场所所有账户的聚合查询：`portfolio.realized_pnls(venue=Venue("IB-PAPER"))`

**示例：在策略中使用多个 IB 执行客户端：**

```python
from nautilus_trader.model.identifiers import AccountId, ClientId
from nautilus_trader.trading.strategy import Strategy


class MultiAccountStrategy(Strategy):
    """使用多个 IB 账户的示例策略。"""

    def on_start(self):
        # 定义账户 ID 以便引用
        self.paper_account = AccountId("IB-PAPER-DU123456")
        self.live_account = AccountId("IB-LIVE-U987654")

        # 查询模拟账户余额
        paper_account_state = self.cache.account(self.paper_account)
        if paper_account_state:
            self.log.info(f"Paper account balance: {paper_account_state.balance_total()}")

        # 查询实盘账户余额
        live_account_state = self.cache.account(self.live_account)
        if live_account_state:
            self.log.info(f"Live account balance: {live_account_state.balance_total()}")

    def submit_order_to_paper(self, order):
        """向模拟交易账户提交订单。"""
        self.submit_order(order, client_id=ClientId("IB-PAPER"))

    def submit_order_to_live(self, order):
        """向实盘交易账户提交订单。"""
        self.submit_order(order, client_id=ClientId("IB-LIVE"))

    def check_paper_pnl(self, instrument_id):
        """检查模拟账户的已实现盈亏。"""
        pnl = self.portfolio.realized_pnl(
            instrument_id=instrument_id, account_id=self.paper_account
        )
        return pnl

    def check_live_pnl(self, instrument_id):
        """检查实盘账户的已实现盈亏。"""
        pnl = self.portfolio.realized_pnl(instrument_id=instrument_id, account_id=self.live_account)
        return pnl
```

**示例：使用多个 IB 客户端查询账户信息：**

```python
from nautilus_trader.model.identifiers import AccountId

# 查询特定账户
paper_account = cache.account(AccountId("IB-PAPER-DU123456"))
live_account = cache.account(AccountId("IB-LIVE-U987654"))

# 使用 account_id 查询账户（推荐方法）
paper_account_by_id = cache.account(AccountId("IB-PAPER-DU123456"))

# 备选方案：使用 account_id 参数查询账户（同样有效）
paper_account_via_account_id = cache.account_for_venue(account_id=AccountId("IB-PAPER-DU123456"))

# 按账户查询投资组合属性
paper_realized_pnl = portfolio.realized_pnl(
    instrument_id=instrument_id, account_id=AccountId("IB-PAPER-DU123456")
)

# 查询跨所有 IB 账户聚合的投资组合属性
# 注意：这会跨具有相同交易场所的所有账户进行聚合
all_ib_realized_pnl = portfolio.realized_pnls(venue=Venue("IB"))
```

### 运行交易节点

```python
def run_trading_node():
    """运行交易节点并进行适当的错误处理。"""
    node = None
    try:
        # 创建并构建节点
        node = TradingNode(config=config_node)
        node.add_data_client_factory(IB, InteractiveBrokersLiveDataClientFactory)
        node.add_exec_client_factory(IB, InteractiveBrokersLiveExecClientFactory)
        node.build()

        # 在此添加你的策略
        # node.trader.add_strategy(YourStrategy())

        # 运行节点
        node.run()

    except KeyboardInterrupt:
        print("Shutting down...")
    except Exception as e:
        print(f"Error: {e}")
    finally:
        if node:
            node.dispose()


if __name__ == "__main__":
    run_trading_node()
```

### 其他配置选项

#### 环境变量

设置这些环境变量以简化配置：

```bash
export TWS_USERNAME="your_ib_username"
export TWS_PASSWORD="your_ib_password"
export TWS_ACCOUNT="your_account_id"
export IB_MAX_CONNECTION_ATTEMPTS="5"  # 可选：限制重连尝试次数
```

#### 日志配置

```python
# 增强的日志配置
logging_config = LoggingConfig(
    log_level="INFO",
    log_level_file="DEBUG",
    log_file_format="json",  # JSON 格式用于结构化日志
    log_component_levels={
        "InteractiveBrokersClient": "DEBUG",
        "InteractiveBrokersDataClient": "INFO",
        "InteractiveBrokersExecutionClient": "INFO",
    },
)
```

你可以在此处找到更多示例：<https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/interactive_brokers>

## 故障排除

### 常见连接问题

#### 连接被拒绝

- **原因**：TWS/Gateway 未运行或端口错误
- **解决方案**：确认 TWS/Gateway 正在运行并检查端口配置
- **默认端口**：TWS（7497/7496），IB Gateway（4002/4001）

#### 认证错误

- **原因**：凭证不正确或账户未登录
- **解决方案**：验证用户名/密码并确保账户已登录 TWS/Gateway

#### 客户端 ID 冲突

- **原因**：多个客户端使用相同的客户端 ID
- **解决方案**：为每个连接使用唯一的客户端 ID

#### 市场数据权限

- **原因**：市场数据订阅不足
- **解决方案**：测试时使用 `IBMarketDataTypeEnum.DELAYED_FROZEN` 或订阅所需的数据源

### 错误代码

Interactive Brokers 使用特定的错误代码。常见的包括：

- **200**：未找到证券定义
- **201**：订单被拒绝 - 原因如下
- **202**：订单已撤销
- **300**：无法通过 ticker ID 找到 EId
- **354**：请求的市场数据未订阅
- **2104**：市场数据服务器连接正常
- **2106**：HMDS 数据服务器连接正常

### 性能优化

#### 减少数据量

```python
# 通过忽略仅数量变化的更新来减少报价 tick 数据量
data_config = InteractiveBrokersDataClientConfig(
    ignore_quote_tick_size_updates=True,
    # ... 其他配置
)
```

#### 连接管理

```python
# 设置合理的超时时间
config = InteractiveBrokersDataClientConfig(
    connection_timeout=300,  # 5 分钟
    request_timeout_secs=60,  # 1 分钟
    # ... 其他配置
)
```

#### 内存管理

- 为策略选择合适的 K 线大小
- 限制同时订阅的数量
- 考虑使用历史数据进行回测而非实时数据

### 最佳实践

#### 安全性

- 永远不要在源代码中硬编码凭证
- 使用环境变量存储敏感信息
- 使用模拟交易进行开发和测试
- 对于仅数据应用，设置 `read_only_api=True`

#### 开发工作流

1. **从模拟交易开始**：始终先使用模拟交易进行测试
2. **使用延迟数据**：开发时使用 `DELAYED_FROZEN` 市场数据
3. **实现适当的错误处理**：优雅地处理连接丢失和 API 错误
4. **监控日志**：启用适当的日志级别进行调试
5. **测试重连**：测试策略在连接中断期间的行为

#### 生产环境部署

- 使用 Docker 化网关进行自动化部署
- 实施适当的监控和告警
- 设置日志聚合和分析
- 仅在必要时使用实时数据订阅
- 实施熔断器和持仓限制

#### 订单管理

- 提交前始终验证订单
- 实施适当的仓位管理
- 为策略使用合适的订单类型
- 监控订单状态并处理拒绝
- 实施订单操作的超时处理

### 调试技巧

#### 启用调试日志

```python
logging_config = LoggingConfig(
    log_level="DEBUG",
    log_component_levels={
        "InteractiveBrokersClient": "DEBUG",
    },
)
```

#### 监控连接状态

```python
# 在策略中检查连接状态
if not self.data_client.is_connected:
    self.log.warning("Data client disconnected")
```

#### 验证金融工具

```python
# 确保在交易前金融工具已加载
instruments = self.cache.instruments()
if not instruments:
    self.log.error("No instruments loaded")
```

### 支持与资源

- **IB API 文档**：[TWS API Guide](https://ibkrcampus.com/ibkr-api-page/trader-workstation-api/)
- **NautilusTrader 示例**：[GitHub Examples](https://github.com/nautechsystems/nautilus_trader/tree/develop/examples/live/interactive_brokers)
- **IB 合约搜索**：[Contract Information Center](https://pennies.interactivebrokers.com/cstools/contract_info/)
- **市场数据订阅**：[IB Market Data](https://www.interactivebrokers.com/en/pricing/market-data-pricing.php)

## 贡献

:::info
如需更多功能或为 Interactive Brokers 适配器做贡献，请参阅我们的
[贡献指南](https://github.com/nautechsystems/nautilus_trader/blob/develop/CONTRIBUTING.md)。
:::
