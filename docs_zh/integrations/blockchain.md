# 区块链 (Blockchain)

## 概览

区块链适配器从 EVM 链摄取 DeFi 数据，并通过 NautilusTrader 数据模型对外暴露。它组合了三项服务：

- HyperSync 用于高吞吐地获取历史区块和合约日志。
- HTTP RPC 用于合约调用、Multicall 读取以及最终链上状态的填充。
- Postgres 用于可选的持久化缓存状态、池元数据、解码后的事件以及快照。

HyperSync 与 RPC 承担不同的角色。HyperSync 是快速的事件来源，而 HTTP RPC 仍然是当前合约状态的真理源，包括 Uniswap V3 的 slot 状态、活跃的 tick 以及持仓。

## 核心原语

DeFi 领域模型位于 `nautilus_model::defi`。

### Chain

`Chain` 定义目标区块链及其默认服务端点。

| 字段                         | 类型           | 描述                                                   |
| -------------------------- | ------------ | ---------------------------------------------------- |
| `name`                     | `Blockchain` | 链枚举值，例如 `Ethereum` 或 `Arbitrum`。                     |
| `chain_id`                 | `u32`        | EVM 链 ID，例如 Ethereum 为 `1`。                          |
| `hypersync_url`            | `String`     | HyperSync 端点，默认为 `https://{chain_id}.hypersync.xyz`。 |
| `rpc_url`                  | `Option`     | 存储在链模型上的可选直连 RPC 端点。                                 |
| `native_currency_decimals` | `u8`         | 原生 Gas 代币的小数精度，通常为 `18`。                             |

链可以通过数字 ID 使用 `Chain::from_chain_id` 加载，或通过名称使用 `Chain::from_chain_name` 加载。

| 链族            | 代码   | 名称           | 小数位 |
| ------------- | ---- | ------------ | --- |
| Ethereum 及 L2 | ETH  | Ethereum     | 18  |
| Polygon       | POL  | Polygon      | 18  |
| Avalanche     | AVAX | Avalanche    | 18  |
| BSC           | BNB  | Binance Coin | 18  |

### DEX 与池

DEX 集成会注册工厂地址、事件签名、解析函数以及 AMM 类型。池定义将链、DEX、池合约、代币对、费率档位、tick 间距以及创建区块绑定为一个稳定的 Nautilus instrument ID。

Uniswap V3 及兼容的集中流动性池还会使用：

- `Initialize(uint160,int24)` 用于初始价格状态。
- `Mint` 与 `Burn` 事件用于持仓和 tick 状态的回放。
- `Swap` 事件用于实时的池价格变动。
- 通过 HTTP RPC 读取最终状态，获取 `slot0`、流动性、活跃 tick 以及持仓数据。

## 配置

| 选项                                | 默认值                | 描述                                 |
| --------------------------------- | ------------------ | ---------------------------------- |
| `chain`                           | 必填                 | 目标 `Chain`，例如 Ethereum 或 Arbitrum。 |
| `dex_ids`                         | `[]`               | 要注册并同步的 DEX 集成。                    |
| `http_rpc_url`                    | 必填                 | 用于合约读取和 Multicall 的 HTTP RPC 端点。   |
| `wss_rpc_url`                     | `None`             | 可选的 WSS RPC 端点，用于 RPC 实时流。         |
| `rpc_requests_per_second`         | `None`             | 可选的 RPC 请求限流。                      |
| `multicall_calls_per_rpc_request` | `200`              | 每个 RPC 请求中请求的最大 Multicall 目标数。     |
| `use_hypersync_for_live_data`     | Rust 中为 `false`    | 设为 true 时，实时区块和事件流将使用 HyperSync。   |
| `from_block`                      | `None`             | 可选的历史同步起始区块。                       |
| `pool_filters`                    | `DexPoolFilters()` | 池范围的过滤规则。                          |
| `postgres_cache_database_config`  | `None`             | 可选的 Postgres 缓存配置。                 |
| `proxy_url`                       | `None`             | 可选的 HTTP 和 WebSocket 代理 URL。       |
| `transport_backend`               | `Tungstenite`      | WebSocket 传输后端。                    |

:::note
池快照请求目前需要 Postgres 缓存数据库。内存缓存可以保存代币和池，但最新的池分析器（pool profiler）引导会通过缓存数据库路径读取快照和事件状态。
:::

## 环境

请在仓库之外设置 HyperSync token 和 RPC URL。不要提交包含密钥的 `.env` 文件。

```fish
set -x ENVIO_API_TOKEN "<envio-token>"
set -x RPC_HTTP_URL "https://your-rpc.example"
set -x RPC_WSS_URL "wss://your-rpc.example"
```

本地使用 `.env` 时：

```dotenv
ENVIO_API_TOKEN=<envio-token>
RPC_HTTP_URL=https://your-rpc.example
RPC_WSS_URL=wss://your-rpc.example
```

Rust HyperSync 客户端要求提供 `ENVIO_API_TOKEN`。缺失或格式错误的 token 会在发送任何查询之前导致客户端构造失败。

## 本地服务

开发用的 compose 文件会启动 Postgres、Redis 和 pgAdmin。

```fish
make start-services
make init-db
```

默认的 Postgres 服务监听 `127.0.0.1:5432`，数据库为 `nautilus`，用户为 `nautilus`，密码为 `pass`。

检查 schema 是否存在：

```fish
docker exec nautilus-database psql -U nautilus -d nautilus -Atc \
    "select count(*) from information_schema.tables where table_schema='public'"
```

对于具有破坏性的 DeFi 测试运行，请使用单独的数据库或可重置的 Docker 卷。池发现和快照测试可能会向 `token`、`pool`、`pool_*_event`、`pool_snapshot`、`pool_position` 和 `pool_tick` 写入大量行。

## 数据流

### 池发现

池发现从 HyperSync 流式获取 DEX 工厂事件，通过 RPC 获取 ERC-20 元数据，并将有效的代币和池存入缓存。可以通过 `DexPoolFilters` 过滤掉代币元数据无效或为空的池。

### 实时数据

当 `use_hypersync_for_live_data` 为 true 时，适配器会通过 HyperSync 订阅区块，然后为已订阅的池获取匹配的 DEX 合约事件。当为 false 时，在存在流式实现的情况下使用 WSS RPC。

### 快照引导

对于 Uniswap V3 快照，引导采用两阶段流程：

- 从 HyperSync 回放历史的 Initialize、Mint 和 Burn 事件，以重建 tick 和持仓。
- 通过 HTTP RPC 和 Multicall 获取最终链上状态，然后从该快照恢复分析器。

如果最终的 RPC 填充失败，适配器必须以失败关闭（fail closed）方式处理。它绝不能发出由回放事件构建、但携带过时价格状态的快照。

### 快照引导守卫

当某个池分析任务应仅基于本地快照缓存准备有限回放时，使用 `--require-existing-snapshot`。该命令会在同步池事件之前，检查目标区块及之前的最新有效 `pool_snapshot`。如果不存在可用的快照，或唯一匹配的是创建区块处那个不含持仓或 tick 的空快照，它会返回 `needs_bootstrap` 并跳过该池从创建区块到目标区块的引导。

```fish
nautilus blockchain analyze-pools \
    --chain ethereum \
    --dex UniswapV3 \
    --addresses-file pools.txt \
    --to-block 25218797 \
    --require-existing-snapshot \
    --rpc-url "$RPC_HTTP_URL"
```

`analyze-pools` 会为每个池打印一条 JSON 结果。需要首次引导的池具有如下形态：

```json
{
  "chain": "Ethereum",
  "dex": "UniswapV3",
  "pool_address": "0x1111111111111111111111111111111111111111",
  "target_block": 25218797,
  "status": "needs_bootstrap"
}
```

单池的 `analyze-pool` 命令保留其既有的成功行为，仅在 `needs_bootstrap` 结果时打印这段 JSON。

### 回测回放

在回测模式下，适配器不响应实时快照请求，因此池分析器必须从回放数据中提供的快照进行初始化。`load_pool_snapshot` 从 Postgres 缓存读取一个池快照，并按选定区块重建其完整的持仓和 tick 状态：

```python
from nautilus_trader.adapters.blockchain import load_pool_snapshot

snapshot = load_pool_snapshot(
    pg_config=postgres_config,
    chain_id=chain_id,
    pool_address=pool_address,
    before_block=replay_start_block,  # 该区块及之前的最新快照
)
```

默认只返回经过链上状态校验的快照；传入 `require_valid=False` 可接受未校验的快照。当缓存中没有匹配的快照时，函数返回 `None`，这应被视为一个配置错误，而不是在缺少分析器状态的情况下进行回放。将快照包装为 `DefiData.PoolSnapshot(snapshot)`，并连同要回放的事件一起传给 `BacktestEngine.add_defi_data`。数据引擎会从快照恢复分析器，缓冲流中早于该快照的任何池事件，并在分析器就绪后再应用它们。请从快照所在区块起向前回放每一个池事件：早于第一个被回放事件的快照会使分析器处于过时状态。

缓存中的区块时间戳会以 UNIX 纳秒形式加载到 Nautilus 数据对象中。以秒级精度写入的缓存行，在加载快照和池事件时会被归一化为纳秒，而纳秒级的行则保留其存储精度。

## 合约

### 基础合约与 Multicall3

`BaseContract` 通过位于 `0xcA11bde05977b3631167028862bE2a173976CA11` 的 Multicall3 批量打包合约调用。

- 调用使用 `allow_failure: true`，以便可以报告单个合约调用的失败。
- 读取针对单个区块上下文执行。
- 传输和 provider 的失败会以 RPC 错误的形式呈现。

### ERC-20 元数据

`Erc20Contract` 通过 Multicall 读取 `name`、`symbol` 和 `decimals`。非标准的代币合约可能返回格式错误的字符串、原始字节或空字段。适配器可以跳过代币未通过元数据校验的池。

### Uniswap V3 池

`UniswapV3PoolContract` 读取池的全局状态、活跃 tick 以及持仓。如果在单个 RPC 调用中打包了过多的 tick 或持仓，大型池可能超出 provider 限制。当前的安全行为是在填充失败时以失败关闭方式处理；超大型池能否成功交付取决于 provider 的限制，或取决于未来的分块/最小化填充工作。

## 冒烟测试

### HyperSync 认证

```fish
curl -fsS --max-time 15 \
    -H "Authorization: Bearer $ENVIO_API_TOKEN" \
    https://1.hypersync.xyz/height
```

预期结果：返回带有数字 `height` 的 JSON。

### 小型 HyperSync 查询

```fish
set query (string join '' \
    '{"from_block":25170900,' \
    '"to_block":25170901,' \
    '"include_all_blocks":true,' \
    '"field_selection":{"block":["number","timestamp","hash"]}}')

curl -sS --max-time 30 \
    -H "Authorization: Bearer $ENVIO_API_TOKEN" \
    -H "Content-Type: application/json" \
    --data "$query" \
    https://1.hypersync.xyz/query/arrow-ipc \
    -o /dev/null \
    -w "http_code=%{http_code} size_download=%{size_download}\n"
```

预期结果：HTTP `200`，且响应大小非零。

### 适配器编译检查

```fish
cargo check -p nautilus-blockchain --features hypersync
```

### 实时失败关闭回归测试

这个被忽略的测试使用真实的 HyperSync 回放，针对 Ethereum 的 WETH/USDT Uniswap V3 池，并刻意使用一个无效的本地 HTTP RPC URL。它验证最终 RPC 填充失败时会返回错误，而不是让过时的快照通过构造路径。

```fish
cargo test -p nautilus-blockchain --features hypersync \
    live_hypersync_bootstrap_fails_closed_when_rpc_hydration_fails \
    -- --ignored --nocapture
```

预期结果：一个被忽略的测试通过。在真实网络上这可能需要几分钟。

## 运维说明

- 使用 HyperSync 进行大批量的历史日志扫描。
- 使用 HTTP RPC 获取最终合约状态并进行校验。
- 对于大型 Uniswap V3 池，使用付费或高限额的 RPC provider。
- 将 `ENVIO_API_TOKEN`、RPC 密钥以及 Postgres 凭据保存在版本控制之外。
- 对会写入池快照的可重复 DeFi 测试运行，使用单独的 Postgres 数据库。
- 对发出的快照，将最终状态填充失败视为硬性失败。

## 当前限制

- 在最终状态的 Multicall 填充期间，超大型 Uniswap V3 池仍可能触及 provider 的负载、超时或速率限制。
- `multicall_calls_per_rpc_request` 记录了预期的批处理上限，但部分最终快照路径仍需要在分块方面进行加固。
- 一次完整成功的 WETH/USDT 或 WETH/USDC 交付测试，需要一个能够提供最终状态读取的真实 HTTP RPC provider，否则适配器需要先实现最小化/分块填充。
