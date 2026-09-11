# 环境搭建 (Environment Setup)

开发时建议使用 PyCharm *Professional* 版 IDE，因为它能解析 Cython 语法。或者也可以使用安装了 Cython 扩展的 Visual Studio Code。

[uv](https://docs.astral.sh/uv) 是管理所有 Python 虚拟环境和依赖的首选工具。

[prek](https://github.com/j178/prek) 用于在提交时自动运行各种 pre-commit 检查、自动格式化工具和 lint 工具。

NautilusTrader 越来越多地使用 [Rust](https://www.rust-lang.org)，因此你的系统上也需要安装 Rust
（[安装指南](https://www.rust-lang.org/tools/install)）。

[Cap'n Proto](https://capnproto.org/) 是序列化 schema 编译所必需的。所需版本在仓库根目录的 `tools.toml` 中指定。Ubuntu 的默认包通常版本过旧，你可能需要从源码安装（见下文）。

:::info
NautilusTrader *必须*能在 **Linux、macOS 和 Windows** 上编译和运行。请注意可移植性
（使用 `std::path::Path`，避免 shell 脚本中的 Bash 特有用法等）。
:::

## 搭建 (Setup)

以下步骤适用于类 UNIX 系统，只需执行一次。

### 快速搭建

对于全新的 Linux 或 macOS 开发机，可将本节作为紧凑的搭建路径。下方各详细小节会逐步解释每个步骤并涵盖替代方案。

首先安装平台工具：

```bash tab="Ubuntu"
sudo apt-get update
sudo apt-get install -y build-essential clang lld curl git make pkg-config
```

```bash tab="macOS"
xcode-select --install
```

然后克隆仓库并安装项目固定的工具：

```bash
git clone --branch develop https://github.com/nautechsystems/nautilus_trader
cd nautilus_trader

curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"

curl -LsSf https://astral.sh/uv/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"

cargo install cargo-binstall --locked
make install-tools
./scripts/install-capnp.sh

uv sync --all-groups --all-extras
source .venv/bin/activate

export PYO3_PYTHON="$PWD/.venv/bin/python"

if [ "$(uname -s)" = "Linux" ]; then
  PYTHON_LIB_DIR="$("$PYO3_PYTHON" -c 'import sysconfig; print(sysconfig.get_config_var("LIBDIR"))')"
  export LD_LIBRARY_PATH="$PYTHON_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

export PYTHONHOME="$("$PYO3_PYTHON" -c 'import sys; print(sys.base_prefix)')"

prek install
make build-debug
```

Windows 用户应先按照[安装指南](../getting_started/installation.md#from-source)中的源码安装步骤操作，然后再使用本指南中的相关命令。

### 1. 安装依赖

按照[安装指南](../getting_started/installation.md)搭建项目，并将最后一条命令修改为安装开发和测试依赖：

```bash tab="uv"
uv sync --active --all-groups --all-extras
```

```bash tab="make"
make install
```

如果你需要频繁开发和迭代，以调试模式（debug mode）编译通常就足够了，而且比完全优化构建*快得多*。
以调试模式安装，使用：

```bash
make install-debug
```

### 2. 安装开发工具

NautilusTrader 固定了每一个开发工具的版本，使所有贡献者和 CI 运行完全相同的版本。
单个 Makefile 目标即可安装整套工具：

```bash
make install-tools
```

这会安装：

- **Cargo CLI**，在 `Cargo.toml` 的 `[workspace.metadata.tools]` 下固定版本：`cargo-audit`、
  `cargo-deny`、`cargo-edit`、`cargo-fuzz`、`cargo-llvm-cov`、`cargo-machete`、`cargo-nextest`、
  `cargo-vet`、`flamegraph`、`lychee`。
- **预构建二进制文件**，在 `tools.toml` 中固定版本：`prek`（pre-commit 运行器）和 `osv-scanner`
  （漏洞扫描器）。
- **uv**，同步到 `pyproject.toml` 所要求的版本。

Cap'n Proto 同样在 `tools.toml` 中固定版本，但需单独安装；参见下方的 [Cap'n Proto](#capn-proto) 小节。

Fuzz 目标在运行时还需要 Rust nightly 工具链，因为 `cargo-fuzz` 使用了
`libfuzzer-sys` 和不稳定的编译器标志：

```bash
rustup toolchain install nightly
```

#### 一次性前置条件：cargo-binstall

`make install-tools` 使用 [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall) 将
`prek` 作为预构建二进制文件拉取，而不是从源码编译。每台机器只需安装一次 `cargo-binstall`：

```bash
cargo install cargo-binstall --locked
```

这是一次性步骤。后续运行 `make install-tools` 会复用已安装的 `cargo-binstall`。

#### 版本的单一真理源

仓库清单文件（manifest）是依赖和工具版本的权威来源。除非没有基于清单文件读取版本的方法，
否则不要将当前的版本号复制到文档、运行器镜像或脚本中。

| 源文件或区段                                     | 定义内容                          |
| ------------------------------------------ | ----------------------------- |
| `rust-toolchain.toml`                      | Rust 工具链。                     |
| `Cargo.toml` 和 `Cargo.lock`                | Rust 工作区依赖及精确解析结果。            |
| `Cargo.toml` `[workspace.metadata.tools]`  | 可通过 Cargo 安装的开发工具。            |
| `pyproject.toml` 和 `python/pyproject.toml` | Python 依赖、支持的 Python 范围以及 uv。 |
| `uv.lock` 和 `python/uv.lock`               | 精确的 Python 依赖解析结果。            |
| `tools.toml`                               | 没有原生清单文件的外部 CLI 和二进制文件。       |

`tools.toml` 中的外部工具固定项包括 `prek`、`pip-audit`、`pypi-attestations`、
`maturin`、`osv-scanner` 和 `capnp`。

Makefile 通过 `scripts/cargo-tool-version.sh`、`scripts/tool-version.sh` 和
`scripts/uv-version.sh` 读取这些版本，因此在源文件中提升某个版本是唯一需要做的版本
变更。要将固定的 cargo 工具版本与 crates.io 进行比对，运行：

```bash
make outdated
```

### 3. 设置 pre-commit

设置 pre-commit 钩子，之后提交时会自动运行：

```bash
prek install
```

在发起 Pull Request 之前，先在本地运行格式化和 lint 套件，以确保 CI 一次通过：

```bash
make format
make pre-commit
```

确保 Rust 编译器报告**零错误** -- 构建失败会拖慢所有人的进度。

### 4. 配置环境变量

**Rust/PyO3 所需（Linux 和 macOS）**：在 Linux 或 macOS 上使用通过 `uv` 安装的 Python 时，
在 `uv sync` 之后从仓库根目录设置以下环境变量：

```bash
# 为 PyO3 设置 Python 可执行文件路径
export PYO3_PYTHON="$PWD/.venv/bin/python"

# 仅 Linux：为 uv 管理的 Python 运行时设置库路径
PYTHON_LIB_DIR="$("$PYO3_PYTHON" -c 'import sysconfig; print(sysconfig.get_config_var("LIBDIR"))')"
export LD_LIBRARY_PATH="$PYTHON_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

# 设置 Python home 路径（Rust 测试所需）
export PYTHONHOME="$("$PYO3_PYTHON" -c 'import sys; print(sys.base_prefix)')"
```

:::note
`LD_LIBRARY_PATH` 导出仅适用于 Linux，macOS 或 Windows 上不需要。

- `PYO3_PYTHON` 告诉 PyO3 使用哪个 Python 解释器，减少不必要的重新编译。
- `PYTHONHOME` 在使用 `uv` 安装的 Python 运行 `make cargo-test` 时是必需的。
  没有它，依赖 PyO3 的测试可能无法找到 Python 运行时。

:::

验证你的环境配置是否正确：

```bash
python -c "import sys; print('Python:', sys.executable, sys.version)"
echo "PYO3_PYTHON: $PYO3_PYTHON"
echo "PYTHONHOME: $PYTHONHOME"
```

## 依赖管理 (Dependency management)

Python 依赖由 [uv](https://docs.astral.sh/uv) 管理。`pyproject.toml` 中的 `[tool.uv]` 区段
强制执行三项供应链安全设置：

- **`required-version`**：所有开发者和 CI 使用相同的 uv 版本。该版本由
  `scripts/uv-version.sh` 提取，供 Makefile、CI 和 Docker 构建使用。如果你本地的 uv 偏离了
  固定版本，`uv lock`/`uv sync` 会以 `Required uv version ... does not match the running
  version ...` 报错。运行 `make update-uv` 安装固定版本（或按照 uv 自身的
  `uv self update <version>` 提示操作）。
- **`exclude-newer = "3 days"`**：`uv lock` 会忽略最近 3 天内发布的包版本。这给了社区
  时间在被入侵的发布版本进入 lockfile 之前检测并隔离它们。该值接受 RFC 3339 时间戳
  （`"2026-03-30T00:00:00Z"`）、友好的时长表示（`"3 days"`、`"1 week"`、`"24 hours"`），
  或 ISO 8601 时长表示（`"P3D"`、`"P1W"`、`"PT24H"`）。uv 0.11.8+ 会将友好/ISO 形式以
  `exclude-newer-span` 存入 `uv.lock`，并在旁边附带一个哨兵 `exclude-newer` 时间戳以保持
  向后兼容；本仓库中的两个 lockfile 都使用该格式。
- **`no-build-package`**：显式列出在 `uv.lock` 中锁定的每一个第三方包。`uv`
  拒绝从源码构建其中任何一个。正常操作中 uv 优先使用 wheel，因此该设置不会生效；
  仅当列出的某个包停止为目标平台发布 wheel 时才会触发，此时 `uv lock` 会失败而不是
  静默地从 sdist 构建。本地工作区包有意不在该列表中，因为它必须由工作区自身的构建
  后端构建。该列表通过 `scripts/check-no-build-packages.sh` 与 `uv.lock` 保持同步，
  当 `uv.lock` 或 `pyproject.toml` 发生变更时该脚本也会作为 pre-commit 钩子运行。

### 绕过冷却期

当某个安全补丁或关键 bug 修复必须立即引入时，可在命令行上覆盖 `exclude-newer`。
所有形式都接受时间戳、友好时长或 ISO 时长；针对单个包的覆盖还额外接受 `false`，
以将某个包完全豁免于冷却期。

```bash
# 为单个包缩短冷却期（友好时长）
uv lock --exclude-newer-package "somepackage=1 day"

# 将单个包固定到一个绝对的截止时间
uv lock --exclude-newer-package "somepackage=2026-03-30T00:00:00Z"

# 将单个包完全豁免于冷却期
uv lock --exclude-newer-package "somepackage=false"

# 为整次解析禁用冷却期
uv lock --exclude-newer "0 seconds"
```

该命令行标志仅对本次调用覆盖 `pyproject.toml` 中的值。后续运行时配置保持不变。

### 更新 uv

要更新固定的 uv 版本，请同时修改 `pyproject.toml` 和 `python/pyproject.toml` 中的
`required-version`，然后更新 `.pre-commit-config.yaml` 中的 `rev` 以保持一致。运行
`make update-uv` 在本地安装新的固定版本。

## 构建 (Builds)

对 `.rs`、`.pyx` 或 `.pxd` 文件做出任何更改后，可以通过以下方式重新编译：

```bash tab="uv"
uv run --no-sync python build.py
```

```bash tab="make"
make build
```

如果你需要频繁开发和迭代，以调试模式编译通常就足够了，而且比完全优化构建*快得多*。
以调试模式编译，使用：

```bash
make build-debug
```

## Cap'n Proto

[Cap'n Proto](https://capnproto.org/) 是序列化 schema 编译所必需的。
所需版本在仓库根目录的 `tools.toml` 中定义。

为你的平台安装正确的版本：

```bash tab="Script (Linux/macOS)"
./scripts/install-capnp.sh
```

```bash tab="macOS (Homebrew)"
brew install capnp
```

```bash tab="Linux (source)"
CAPNP_VERSION=$(bash scripts/tool-version.sh capnp)
cd ~
wget https://capnproto.org/capnproto-c++-${CAPNP_VERSION}.tar.gz
tar xzf capnproto-c++-${CAPNP_VERSION}.tar.gz
cd capnproto-c++-${CAPNP_VERSION}
./configure
make -j$(nproc)
sudo make install
sudo ldconfig
```

```bash tab="Windows (Chocolatey)"
choco install capnproto
```

验证已安装的版本与 `tools.toml` 一致：

```bash
capnp --version
```

安装脚本会确保安装的是固定版本。如果 Homebrew 或 Chocolatey 提供的版本较旧，
请从源码安装，或参阅 [Cap'n Proto 安装指南](https://capnproto.org/install.html)。

## 加速构建 (Faster builds)

cranelift 后端可以显著减少开发、测试和 IDE 检查的构建时间。不过 cranelift 仅在 nightly 工具链上可用，且需要额外配置。安装 nightly 工具链：

```
rustup install nightly
rustup override set nightly
rustup component add rust-analyzer # 安装 nightly LSP
rustup override set stable # 重置为 stable
```

在工作区 `Cargo.toml` 的 dev 和 testing 配置中激活 nightly 特性并使用 "cranelift" 后端。你可以使用 `git apply <patch>` 来应用下面的补丁，推送变更前使用 `git apply -R <patch>` 来撤销它。

:::warning
不要提交这些更改。cranelift 补丁仅用于本地开发，推送后会导致 CI 失败。
:::

```
diff --git a/Cargo.toml b/Cargo.toml
index 62b78cd8d0..beb0800211 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1,3 +1,6 @@
+# This line needs to come before anything else in Cargo.toml
+cargo-features = ["codegen-backend"]
+
 [workspace]
 resolver = "2"
 members = [
@@ -140,6 +143,7 @@ lto = false
 panic = "unwind"
 incremental = true
 codegen-units = 256
+codegen-backend = "cranelift"

 [profile.test]
 opt-level = 0
@@ -150,11 +154,13 @@ strip = false
 lto = false
 incremental = true
 codegen-units = 256
+codegen-backend = "cranelift"

 [profile.nextest]
 inherits = "test"
 debug = false # Improves compile times
 strip = "debuginfo" # Improves compile times
+codegen-backend = "cranelift"

 [profile.release]
 opt-level = 3
```

运行 `make build-debug` 等命令时传入 `RUSTUP_TOOLCHAIN=nightly`，并在所有 [rust analyzer 设置](#rust-analyzer-settings)中包含该变量，以加速构建和 IDE 检查。

## 服务 (Services)

你可以使用位于 `.docker` 目录中的 `docker-compose.yml` 文件来启动 Nautilus 开发环境。这将启动以下服务：

```bash
docker-compose up -d
```

如果你只想运行特定服务（例如 `postgres`），可以使用以下命令启动：

```bash
docker-compose up -d postgres
```

使用的服务包括：

- `postgres`：Postgres 数据库，root 用户为 `POSTGRES_USER`（默认 `postgres`），`POSTGRES_PASSWORD`（默认 `pass`），`POSTGRES_DB`（默认 `postgres`）。
- `redis`：Redis 服务器。
- `pgadmin`：用于数据库管理和维护的 PgAdmin4。

:::info
请仅将此作为开发环境使用。生产环境请使用更正式、更安全的配置。
:::

服务启动后，你需要使用 `psql` 命令行工具登录并创建 `nautilus` Postgres 数据库。
运行以下命令，并输入 Docker 服务配置中的 `POSTGRES_PASSWORD`：

```bash
psql -h localhost -p 5432 -U postgres
```

以 `postgres` 管理员身份登录后，运行 `CREATE DATABASE` 命令创建目标数据库（这里使用 `nautilus`）：

```
psql (16.2, server 15.2 (Debian 15.2-1.pgdg110+1))
Type "help" for help.

postgres=# CREATE DATABASE nautilus;
CREATE DATABASE

```

## Nautilus CLI 开发者指南

## 简介

Nautilus CLI 是一个用于与 NautilusTrader 生态系统交互的命令行界面工具。
它提供了管理 PostgreSQL 数据库和处理各种交易操作的命令。

:::warning
在使用 GNOME 桌面的 Linux 系统上，`nautilus` 命令通常指向 GNOME 文件管理器（`/usr/bin/nautilus`）。
安装 NautilusTrader CLI 后，你可能需要通过以下方式之一确保 Cargo 二进制文件优先：

- 在 shell 配置中添加别名：`alias nautilus="$HOME/.cargo/bin/nautilus"`
- 使用完整路径：`~/.cargo/bin/nautilus`
- 确保 `~/.cargo/bin` 在 `PATH` 中位于 `/usr/bin` 之前

:::

:::note
Nautilus CLI 命令仅在类 UNIX 系统上受支持。
:::

## 安装

你可以使用下面的 Makefile 目标安装 Nautilus CLI，底层使用 `cargo install`。
这会将 nautilus 二进制文件放入系统的 PATH 中，前提是 Rust 的 `cargo` 已正确配置。

```bash
make install-cli
```

## 命令

运行 `nautilus --help` 可以查看 CLI 结构和可用的命令组：

### 数据库

这些命令用于引导（bootstrap）PostgreSQL 数据库。
使用时需要提供正确的连接配置，
可以通过命令行参数或放在根目录或当前工作目录的 `.env` 文件中提供。

- `--host` 或 `POSTGRES_HOST` 用于数据库主机
- `--port` 或 `POSTGRES_PORT` 用于数据库端口
- `--user` 或 `POSTGRES_USERNAME` 用于 root 管理员（通常是 postgres 用户）
- `--password` 或 `POSTGRES_PASSWORD` 用于 root 管理员密码
- `--database` 或 `POSTGRES_DATABASE` 同时用于数据库**名称和新用户**，该用户拥有该数据库的权限
    （例如，如果你提供 `nautilus` 作为值，将创建一个名为 nautilus 的新用户，密码来自 `POSTGRES_PASSWORD`，并以该用户为所有者引导 `nautilus` 数据库）。

`.env` 文件示例

```
POSTGRES_HOST=localhost
POSTGRES_PORT=5432
POSTGRES_USERNAME=postgres
POSTGRES_PASSWORD=pass
POSTGRES_DATABASE=nautilus
```

命令列表如下：

1. `nautilus database init`：引导 schema、角色以及 `schema` 根目录中的所有 sql 文件（如 `tables.sql`）。
2. `nautilus database drop`：删除目标 Postgres 数据库中的所有表、角色和数据。

## Rust analyzer 设置

Rust analyzer 是一个流行的 Rust 语言服务器，集成了许多 IDE。建议将 rust analyzer 配置为与 `make build-debug` 相同的环境变量以加快编译速度。下面提供了 VSCode 和 Astro Nvim 的经过测试的配置。更多信息请参阅 [PR](https://github.com/nautechsystems/nautilus_trader/pull/2524) 或 rust analyzer [配置文档](https://rust-analyzer.github.io/book/configuration.html)。

```json tab="VSCode"
{
    "rust-analyzer.restartServerOnConfigChange": true,
    "rust-analyzer.linkedProjects": [
        "Cargo.toml"
    ],
    "rust-analyzer.cargo.features": "all",
    "rust-analyzer.check.workspace": false,
    "rust-analyzer.check.extraEnv": {
        "VIRTUAL_ENV": "<path-to-your-virtual-environment>/.venv",
        "CC": "clang",
        "CXX": "clang++"
    },
    "rust-analyzer.cargo.extraEnv": {
        "VIRTUAL_ENV": "<path-to-your-virtual-environment>/.venv",
        "CC": "clang",
        "CXX": "clang++"
    },
    "rust-analyzer.runnables.extraEnv": {
        "VIRTUAL_ENV": "<path-to-your-virtual-environment>/.venv",
        "CC": "clang",
        "CXX": "clang++"
    },
    "rust-analyzer.check.features": "all",
    "rust-analyzer.testExplorer": true
}
```

```lua tab="Neovim (AstroLSP)"
config = {
  rust_analyzer = {
    settings = {
      ["rust-analyzer"] = {
        restartServerOnConfigChange = true,
        linkedProjects = { "Cargo.toml" },
        cargo = {
          features = "all",
          extraEnv = {
            VIRTUAL_ENV = "<path-to-your-virtual-environment>/.venv",
            CC = "clang",
            CXX = "clang++",
          },
        },
        check = {
          workspace = false,
          command = "check",
          features = "all",
          extraEnv = {
            VIRTUAL_ENV = "<path-to-your-virtual-environment>/.venv",
            CC = "clang",
            CXX = "clang++",
          },
        },
        runnables = {
          extraEnv = {
            VIRTUAL_ENV = "<path-to-your-virtual-environment>/.venv",
            CC = "clang",
            CXX = "clang++",
          },
        },
        testExplorer = true,
      },
    },
  },
}
```
