# 设计原则 (Design Principles)

## 消息不可变性 (Message immutability)

一条消息（请求、响应、事件或命令）一旦被创建，其字段就不得再被修改。关于由此衍生的所有权规则，参见 [Message Bus: message integrity](../concepts/message_bus.md#消息完整性-message-integrity)。

这条不变式保护了系统所依赖的若干属性：

- **确定性 (Determinism)**：每个消费者看到的输入都完全相同。这让行为更容易推理、重放和测试。
- **时间完整性 (Temporal integrity)**：一条消息保留了系统在发出它时所成立的事实。事件和命令因此始终是真实的记录，而不是承载着不断漂移状态的容器。
- **更安全的并发 (Safer concurrency)**：读取方无需通过协调来防止消息负载被后续改写。这消除了围绕共享状态的一类常见竞态来源。
- **更易于调试 (Easier debugging)**：由于消息仍然反映原始负载，日志、追踪、重放工具以及死信检查依然有用。
- **可靠的重放与模拟 (Reliable replay and simulation)**：重放一个序列会产生与原始运行相同的逻辑输入。这支撑了回测、事故重建和回归测试。
- **清晰的所有权边界 (Clear ownership boundaries)**：组件把传入的消息视为输入。如果某个组件需要不同的表示形式，它会显式地派生出新的本地状态或一条新消息。
- **更好的可审计性 (Better auditability)**：系统能够回答它知道了什么、何时知道，以及它基于这些信息做了什么。
- **更健壮的分布式能力 (More robust distribution)**：序列化后的消息本就以副本形式跨越进程和服务边界。同样的所有权规则使内存中的模型与这一现实保持一致。
