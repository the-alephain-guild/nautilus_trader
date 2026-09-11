# 文档风格 (Docs Style)

本指南概述了编写 NautilusTrader 文档（documentation）的风格约定和最佳实践。

## 总体原则

- 我们崇尚简洁胜于复杂，少即是多。
- 我们追求简洁而易读的文字和文档。
- 我们重视约定、风格、模式等方面的标准化。
- 文档应适合不同技术背景的用户阅读。

## 文档类型

大多数页面应归入以下四种类型之一
（[Divio documentation system](https://docs.divio.com/documentation-system/)）。
在单个页面中混合多种类型会使其更难阅读、更难维护。

| 类型               | 用途          | 章节               |
| ---------------- | ----------- | ---------------- |
| **Tutorial**     | 通过完成一项任务来教学 | `tutorials/`     |
| **How‑to guide** | 解决某个具体问题    | `how_to/`        |
| **Explanation**  | 阐明设计和架构     | `concepts/`      |
| **Reference**    | 描述具体的机制     | `api_reference/` |

有两个章节是例外：`getting_started/` 是一条入门路径，它将教程式的演练与
安装说明结合在一起；`integrations/` 页面则将参考内容（能力、符号体系）与
操作指南内容（安装、配置）混合，以使每个交易场所页面都能自成一体。
不针对特定交易场所的独立操作指南内容应放在 `how_to/` 中。

### 选择正确的类型

- **你的页面是否引导新手完成一段学习体验？** 选 Tutorial。
- **它是否为已经了解系统的人回答"我该如何……？"的问题？** 选 How-to guide。
- **它是否解释某件事物为何以这种方式工作？** 选 Explanation。
- **它是否列举类、配置字段、枚举或能力？** 选 Reference。

教程（Tutorial）说的是"先做这个，再做这个，然后做这个"，路径由作者选定。
操作指南（How-to guide）说的是"以下是实现 X 的方法"，读者已经知道
自己想要 X。请保持二者的区别：

- 教程不应假设读者具备先验知识。
- 操作指南不应讲解背景概念。

当一种类型需要引用另一种类型时，应链接过去而不要内联其内容。例如，
一份配置 `TradingNodeConfig` 的操作指南应链接到对应字段定义的 API 参考，
而不是再次列出这些字段。

## 语言和语气

- 尽量使用主动语态（"Configure the adapter" 而非 "The adapter should be configured"）。
- 描述当前功能时使用现在时。
- 仅对计划中的功能使用将来时。
- 避免不必要的行话；技术术语首次使用时给出定义。
- 直接简洁；避免 "basically"、"simply"、"just" 等填充词。
- 列表中使用平行结构；各条目保持一致的语法模式。

## Markdown 表格

### 列对齐和间距

- 根据每列中最宽内容所需空间使用对称的列宽。
- 垂直对齐列分隔符（`|`）以提升可读性。
- 单元格内容周围使用一致的间距。

### 说明和描述

- 所有说明和描述应以句号结尾。
- 说明要简洁且信息丰富。
- 使用句首大写（仅首字母和专有名词大写）。

### 示例

```markdown
| Order Type             | Spot | Margin | USDT Futures | Coin Futures | Notes                   |
|------------------------|------|--------|--------------|--------------|-------------------------|
| `MARKET`               | ✓    | ✓      | ✓            | ✓            |                         |
| `STOP_MARKET`          | -    | ✓      | ✓            | ✓            | Not supported for Spot. |
| `MARKET_IF_TOUCHED`    | -    | -      | ✓            | ✓            | Futures only.           |
```

### 支持状态标识

- 使用 `✓` 表示支持的功能。
- 使用 `-` 表示不支持的功能（不使用 `✗` 或其他符号）。
- 为不支持的功能添加说明时，使用斜体强调：`*Not supported*`。
- 当原因重要时，使不支持的说明更具体：交易场所本身缺失的能力用
  `*Not supported by <venue>*`，适配器缺失的能力用 `*Not currently implemented*`。
- 不需要内容时留空单元格。

## 代码引用

- 内联代码、方法名、类名和配置选项使用反引号。
- 多行示例使用代码块。
- 引用代码位置时，使用 `file_path::function_name` 或 `file_path::ClassName` 而非行号，因为行号会随着代码变化而过时。

## 标题

我们遵循现代文档约定，优先考虑可读性和可访问性：

- 主页标题（仅 # 一级标题）使用标题大写（Title Case）。
- 所有子标题（## 二级及以下）使用句首大写（Sentence case）。
- 无论标题级别如何，专有名词始终大写（产品名、技术名、公司名、缩写词）。
- 确保正确的标题层级（不跳过层级）。

此约定与 Google 开发者文档、Microsoft Docs 和 Anthropic 文档等主要科技公司采用的行业标准一致。
它提升了可读性，降低了认知负担，对国际用户和屏幕阅读器也更加友好。

### 示例

```markdown
# NautilusTrader Developer Guide

## Getting started with Python
## Using the Binance adapter
## REST API implementation
## WebSocket data streaming
## Testing with pytest
```

## 列表

- 无序列表使用连字符（`-`）作为项目符号；避免使用 `*` 或 `+`，以保持整个项目的 Markdown 风格一致。
- 仅在顺序重要时使用有序列表。
- 嵌套列表保持一致的缩进。
- 当列表项是完整句子时以句号结尾。

## 链接和引用

- 使用描述性的链接文本（避免 "click here" 或 "this link"）。
- 适当时引用外部文档。
- 确保所有内部链接使用相对路径且准确。

## 技术术语

- 能力矩阵应基于 Nautilus 领域模型，而非交易所特定的术语。
- 在需要时以括号或说明的形式提及交易所特定的术语。
- 整个文档中使用一致的术语。

## 示例和代码样例

- 提供实用的、可运行的示例。
- 包含必要的导入和上下文。
- 使用真实的变量名和值。
- 为示例中不明显的部分添加注释。

## 提示框

使用提示框块来突出重要信息：

| 提示框          | 用途                  |
| ------------ | ------------------- |
| `:::note`    | 补充性上下文，用于澄清但非必需的信息。 |
| `:::info`    | 读者应了解的重要信息。         |
| `:::tip`     | 有用的建议或最佳实践。         |
| `:::warning` | 潜在的陷阱或重要的注意事项。      |
| `:::danger`  | 可能导致数据丢失或系统故障的严重问题。 |

避免过度使用提示框；过多会削弱其效果。

## MDX 组件

文档站点（fumadocs）提供了内置的 MDX 组件，可在所有 `.md` 文件中使用。
无需任何导入。

### Tabs

为连续的围栏代码块添加 `tab="..."`，以提供针对不同语言或不同变体的代码示例。
将 Rust 列在 Python 之前，以使 Rust 成为默认（最左侧）的标签页。

```markdown
\`\`\`rust tab="Rust"
let params = Params::from([("close_position", true.into())]);
\`\`\`

\`\`\`python tab="Python"
strategy.submit_order(order, params={"close_position": True})
\`\`\`
```

### Steps

使用 `Steps` 和 `Step` 表示顺序执行的步骤。

```markdown
<Steps>
<Step>
Configure the adapter.
</Step>
<Step>
Start the trading node.
</Step>
</Steps>
```

### Accordions

使用 `Accordions` 和 `Accordion` 表示可折叠内容。

```markdown
<Accordions>
<Accordion title="Advanced configuration">
Content here.
</Accordion>
</Accordions>
```

### Files

使用 `Files`、`Folder` 和 `File` 来可视化目录树。

```markdown
<Files>
<Folder name="src" defaultOpen>
<File name="main.rs" />
<File name="lib.rs" />
</Folder>
</Files>
```

### Cards

使用 `Cards` 和 `Card` 来呈现带链接的内容网格。

```markdown
<Cards>
<Card title="Getting started" href="/latest/getting_started" />
<Card title="Concepts" href="/latest/concepts" />
</Cards>
```

### TypeTable

使用 `TypeTable` 来呈现参数或类型文档表格。

## 行长度和换行

- 换行不超过约 100-120 个字符，以提升可读性和代码审查体验。
- 在自然断点处（逗号、连词或短语之后）断行。
- 尽量避免在新行上留下孤立的单词。
- 代码块和 URL 在必要时可以超过行长限制。

## API 文档

- 清晰地记录参数和返回类型。
- 为复杂的 API 提供使用示例。
- 说明任何副作用或重要行为。
- 参数描述要简洁但完整。
