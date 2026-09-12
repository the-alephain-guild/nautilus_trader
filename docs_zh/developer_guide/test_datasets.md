# 测试数据集 (Test Datasets)

用于规范测试夹具（test fixtures）所使用的外部数据集的整理、存储与消费的目标标准。
新数据集应遵循这些标准。早于本策略制定的既有数据集，记录在[遗留数据集](#遗留数据集)一节。

## 数据集分类

**小数据**（< 1 MB）直接签入 `tests/test_data/<source>/`，并附带一个 `metadata.json` 文件。这些文件无需网络访问即可随时使用。

**大数据**（> 1 MB）以 Parquet 格式托管在 R2 测试数据存储桶中。
其 SHA-256 校验和记录在 `tests/test_data/large/checksums.json` 中。
`ensure_test_data_exists()` 辅助函数会在首次使用时下载该文件并校验其完整性。

**用户自取数据**用于这样的情况：当某个供应商的许可证、授权模式或访问控制不允许 NautilusTrader 通过公共仓库或公共 R2 存储桶重新分发该数据时。
在这种模式下，仓库只存储一份清单（manifest）、获取说明以及转换代码。每个用户用自己的供应商账号下载源数据，并在本地完成转换。

当满足以下任一条件时，应使用用户自取模式：

- 供应商要求每个用户持有自己的账号、API key 或历史数据许可证。
- 许可证允许内部使用，但并未明确允许重新分发派生的夹具。
- 该数据集适合用于示例或自愿加入（opt-in）的集成测试，但不适合默认 CI。

## 必需的元数据

每个存储或重新分发具体产物（artifact）的整理过的数据集，都必须包含一个 `metadata.json`，至少包含以下字段：

| 字段             | 描述                    |
| -------------- | --------------------- |
| `file`         | 数据集的文件名。              |
| `sha256`       | 文件的 SHA-256 哈希值。      |
| `size_bytes`   | 文件大小（字节）。             |
| `original_url` | 原始源数据的下载 URL。         |
| `licence`      | 许可证条款及任何重新分发约束。       |
| `added_at`     | 数据集整理时的 ISO 8601 时间戳。 |

这些字段与 `scripts/curate-dataset.sh` 的输出相对应。为提供更丰富的来源信息（provenance），推荐补充以下字段：

| 字段              | 描述                                          |
| --------------- | ------------------------------------------- |
| `instrument`    | 所涵盖的金融工具代码（symbol）。                         |
| `date`          | 所涵盖的交易日期。                                   |
| `format`        | 存储格式（例如 "Nautilus OrderBookDelta Parquet"）。 |
| `original_file` | 转换前的原始供应商文件名。                               |
| `parser`        | 用于转换的解析器（例如 "itchy 0.3.4"）。                 |

用户自取数据集在适用的字段上使用相同的元数据字段。它们还应包含以下字段：

| 字段                  | 描述                          |
| ------------------- | --------------------------- |
| `distribution`      | 必须为 `"user-fetch"`。         |
| `fetch_method`      | 用户获取源数据的方式（API、网页门户、CLI 等）。 |
| `fetch_reference`   | 面向用户的下载流程的 URL 或文档引用。       |
| `auth`              | 所需的凭据或授权（如有）。               |
| `transform_version` | 用于构建最终文件的本地转换流水线版本。         |
| `redistribution`    | 简要说明该数据集的重新分发限制。            |
| `public_mirror`     | 对于受限的供应商数据集，必须为 `false`。    |

对于没有单一已签入或已镜像产物的用户自取数据集，`metadata.json` 中可以省略 `file`、`sha256` 和 `size_bytes`。在这种情况下，`manifest.json` 中的 `target_files` 才是本地输出文件的权威来源。对于用户自取数据集，当具体文件是按每个用户账号或每次请求生成时，`original_url` 可以指向供应商的下载入口点，而非确切的文件 URL。

其他元数据字段在适用时仍然推荐填写。尤其是 `licence` 和 `added_at`，对于用户自取数据集仍应记录。

## 存储格式

新数据集应存储为 **Nautilus Parquet**（而非原始的供应商格式）。
这能确保：

- 所有测试数据集之间的数据类型保持一致。
- 测试时无需解析供应商格式。
- 在许可证方面具有清晰的衍生作品（derivative-work）状态。

使用 ZSTD 压缩（级别 3），行组（row group）大小为 100 万行。

用户自取数据集在本地转换步骤完成后，也应最终成为 Nautilus Parquet。
原始供应商文件应保留在仓库之外、以及公共 R2 存储桶之外。

## 命名约定

```
<source>_<instrument>_<date>_<datatype>.parquet
```

示例：

- `itch_AAPL_2019-01-30_deltas.parquet`
- `tardis_BTCUSDT_2020-09-01_depth10.parquet`
- `histdata_EURUSD.SIM_2020-01_quotes.parquet`

## 整理工作流

### 简单文件（单次下载）

使用 `scripts/curate-dataset.sh`：

```bash
scripts/curate-dataset.sh <slug> <filename> <download-url> <license>
```

这会创建一个带版本号的目录（`v1/<slug>/`），其中包含该文件、`LICENSE.txt` 以及包含上述必需字段的 `metadata.json`。

### 复杂流水线（解析 + 转换）

对于需要格式转换的数据集（例如将二进制 ITCH 转为 Parquet）：

1. 在 `crates/testkit/src/<source>/` 中编写一个整理函数，用 `#[cfg(test)]` 或 `#[ignore]` 测试进行隔离。
2. 该函数应：下载、解析、过滤、转换为 NautilusTrader 类型、写入 Parquet。
3. 将 Parquet 文件和 `metadata.json` 输出到本地目录。
4. 手动上传到 R2，然后将校验和添加到 `checksums.json`。

### 用户自取流水线（受限的重新分发）

对于 NautilusTrader 无法重新分发的数据集：

1. 提交一份清单和 `metadata.json`，但不要提交真实的供应商数据或派生的 Parquet 输出。
2. 提供一个本地获取命令或辅助工具，使用用户自己的供应商凭据、授权或购买的历史文件。
3. 在本地将供应商数据转换为 Nautilus Parquet。
4. 将生成的文件存储在被 git 忽略的本地缓存路径中。
5. 让测试和示例采用自愿加入（opt in）的方式。当数据集缺失时，它们应能干净地跳过。

新数据集的默认分发优先级顺序为：

1. 签入的小数据。
2. 公共 R2 大数据。
3. 用户自取数据。

只有当前两个选项在供应商条款下不可接受时，才选择用户自取。

不要：

- 将受限的供应商数据集上传到公共 R2 存储桶。
- 在重新分发权利不明确时，将真实的供应商派生 Parquet 文件提交到仓库。
- 让默认 CI 依赖供应商凭据或付费的历史数据访问。

当许可证允许内部共享时，你可以为内部 CI 或员工维护一个私有镜像。请将其视为一条独立的运维路径，而非公共测试数据标准的一部分。

## 添加新数据集

1. 按照上述工作流整理数据。
2. 编写包含所有必需字段的 `metadata.json`。
3. 对于小数据：提交到 `tests/test_data/<source>/`。
4. 对于大数据：将 Parquet 上传到 R2，并将校验和添加到 `tests/test_data/large/checksums.json`。
5. 对于用户自取数据：仅提交清单和获取说明。将源数据和派生数据保留在仓库之外、以及公共 R2 存储桶之外。
6. 当需要共享的 testkit 访问时，向 `crates/testkit/src/common.rs` 添加路径辅助函数。
7. 编写消费该数据集的测试。

对于用户自取数据，推荐使用以下布局：

```text
tests/test_data/<source>/<slug>/
  metadata.json
  manifest.json
  README.md
```

将 `tests/test_data/local/<source>/<slug>/` 用作生成产物的标准本地缓存路径。当需要本地保留时，将原始供应商下载文件放在同一缓存路径下的同级 `vendor/` 目录中。

清单应当是机器可读且稳定的。它应捕获在另一台机器上重现获取与转换步骤所需的最小信息。

`metadata.json` 是来源、许可证和重新分发规则的权威来源。
`manifest.json` 是获取输入、命令、缓存位置和输出文件的权威来源。

推荐的清单字段：

| 字段                  | 描述                                             |
| ------------------- | ---------------------------------------------- |
| `slug`              | 稳定的数据集标识符。                                     |
| `vendor`            | 供应商或交易场所名称。                                    |
| `source_type`       | `api`、`portal-download`、`purchased-archive` 等。 |
| `source_filters`    | 代码（symbol）、事件 ID、市场 ID、日期范围或文件名。               |
| `target_files`      | 转换后预期产出的 Nautilus Parquet 文件。                  |
| `cache_dir`         | 相对于 `tests/test_data/local/` 的本地输出位置。          |
| `fetch_command`     | 建议的命令或脚本入口点。                                   |
| `transform_command` | 建议的本地转换命令。                                     |
| `env`               | 所需的环境变量。                                       |
| `notes`             | 面向用户的简短运维说明。                                   |

依赖用户自取数据的测试应当：

- 与默认 CI 测试单独标记或分组。
- 当本地数据集不存在时，以清晰的消息跳过。
- 除非用户明确选择加入，否则避免网络访问。
- 复用一个稳定的本地缓存路径，使每台机器只需获取一次。

对于基于 pytest 的测试，推荐使用如下守卫：

```python
if not filepath.exists():
    pytest.skip(f"User-fetched test data not found: {filepath}")
```

对于需要手动准备数据集的 Rust 测试，当该测试不期望在默认 CI 中运行时，推荐使用 `#[ignore]`。

## 测试运行器序列化

下载大数据文件的测试会跨测试二进制文件共享目标路径。
由于 `nextest` 在独立进程中运行每个二进制文件，对同一路径的并发下载可能发生竞争。
位于 `.config/nextest.toml` 的 nextest 配置定义了一个 `large-data-tests` 分组，并设置 `max-threads = 1`，以将这些二进制文件串行化。

当添加一个会下载大型共享文件的新测试二进制文件时，将其添加到分组过滤器中：

```toml
[[profile.default.overrides]]
filter = 'binary(grid_mm_itch) | binary(orderbook_integration) | binary(your_new_binary)'
test-group = 'large-data-tests'
```

## 重新生成数据集

当某个 schema 变更使大型 Parquet 文件失效时，使用下文的整理测试从原始源数据重新生成它。重新生成后：

1. `sha256sum /tmp/<output_file>.parquet`
2. 用新的哈希值更新 `tests/test_data/large/checksums.json`。
3. 更新对应的 `metadata.json`（sha256、size_bytes）。
4. 将 Parquet 文件上传到 R2。
5. 提交 `checksums.json` 和 `metadata.json`（这也会使 CI 缓存失效）。

### ITCH AAPL L3 deltas

来源：来自 NASDAQ EMI 的 `01302019.NASDAQ_ITCH50.gz`（约 4.4 GB）。

```bash
# 下载源文件（保留一份本地副本，这是个大文件）
wget -O ~/Downloads/01302019.NASDAQ_ITCH50.gz \
  "https://emi.nasdaq.com/ITCH/Nasdaq%20ITCH/01302019.NASDAQ_ITCH50.gz"

# 整理测试期望源文件位于 /tmp
ln -sf ~/Downloads/01302019.NASDAQ_ITCH50.gz /tmp/01302019.NASDAQ_ITCH50.gz

# 重新生成 parquet（输出：/tmp/itch_AAPL.XNAS_2019-01-30_deltas.parquet）
cargo test -p nautilus-testkit --lib test_curate_aapl_itch -- --ignored --nocapture
```

### Tardis Deribit BTC-PERPETUAL L2 deltas

来源：来自 [Tardis](https://tardis.dev/) 的 `tardis_deribit_incremental_book_L2_2020-04-01_BTC-PERPETUAL.csv.gz`。每月首日的数据作为免费样本提供（无需 API key）。

```bash
# 下载源文件（免费样本，无需 API key）
wget -O tests/test_data/large/tardis_deribit_incremental_book_L2_2020-04-01_BTC-PERPETUAL.csv.gz \
  "https://datasets.tardis.dev/v1/deribit/incremental_book_L2/2020/04/01/BTC-PERPETUAL.csv.gz"

# 重新生成 parquet（输出：/tmp/tardis_BTC-PERPETUAL.DERIBIT_2020-04-01_deltas.parquet）
cargo test -p nautilus-tardis test_curate_deribit_deltas -- --ignored --nocapture
```

## 教程测试数据

部分教程会加载用户提供的市场数据。`NAUTILUS_DATA_DIR` 环境变量会覆盖这些教程使用的基础数据路径。测试套件将该变量设置为 `tests/test_data/local/`，以便教程针对本地存储的小型样本文件运行。

### 目录布局

```text
tests/test_data/local/
  Binance/
    BTCUSDT_T_DEPTH_2022-11-01_depth_snap.csv
    BTCUSDT_T_DEPTH_2022-11-01_depth_update.csv
  Bybit/
    2024-12-01_XRPUSDT_ob500.data.zip
  HISTDATA/
    DAT_ASCII_EURUSD_T_202001.csv.gz
```

`tests/test_data/local/` 目录被 gitignore 忽略。当数据缺失时，测试会跳过。

### 获取数据

**Binance 深度快照（depth snapshots）**可从 [Binance 公共数据门户](https://data.binance.vision/) 获取。下载 2022-11-01 的 BTCUSDT T_DEPTH 文件，并将快照（snap）和更新（update）的 CSV 放置在 `tests/test_data/local/Binance/` 下。出于测试目的，行的一个子集（例如前 10,000 行）就足够了。

**Bybit ob500 订单簿数据**可从 Bybit CDN 获取：

```bash
curl -L "https://quote-saver.bycsi.com/orderbook/linear/XRPUSDT/2024-12-01_XRPUSDT_ob500.data.zip" \
  -o tests/test_data/local/Bybit/2024-12-01_XRPUSDT_ob500.data.zip
```

完整文件约 360 MB。出于测试目的，提取前几百行并重新打包为一个较小的 zip。

**HISTDATA tick 数据**可从 [histdata.com](https://www.histdata.com/) 获取。下载任意月份的 EUR/USD ASCII tick 数据，并将 CSV（或 `.csv.gz`）放置在 `tests/test_data/local/HISTDATA/` 下。

### 运行测试

```bash
pytest tests/docs_tests/test_tutorials.py::test_tutorial_with_local_data -v
```

当对应的数据子目录为空或缺失时，测试会带消息跳过。

## 遗留数据集

这些数据集早于本策略，使用原始的供应商格式（CSV/CSV.gz），且没有 `metadata.json`。它们对既有测试仍然有效。新数据集应遵循上述 Parquet 标准。

| 数据集                        | 来源       | 格式         | 位置                        | 状态  |
| -------------------------- | -------- | ---------- | ------------------------- | --- |
| Tardis Deribit L2 deltas   | Tardis   | Parquet（大） | `tests/test_data/large/`  | 已整理 |
| ITCH AAPL L3 deltas        | NASDAQ   | Parquet（大） | `tests/test_data/large/`  | 已整理 |
| HISTDATA EURUSD.SIM quotes | HISTDATA | Parquet（大） | `tests/test_data/large/`  | 已迁移 |
| Tardis Deribit L2          | Tardis   | CSV（签入）    | `tests/test_data/tardis/` | 遗留  |
| Tardis Binance snapshots   | Tardis   | CSV.gz（大）  | `tests/test_data/large/`  | 遗留  |
| Tardis Bitmex trades       | Tardis   | CSV.gz（大）  | `tests/test_data/large/`  | 遗留  |

原先的 `nautechsystems/nautilus_data` 目录映射到上文的 HISTDATA EURUSD.SIM Parquet 文件。原始的 HISTDATA CSV 文件仍为用户自取。
