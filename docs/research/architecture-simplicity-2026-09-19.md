# 架构简化：公开工程资料核验

核验日期：2026-09-19。问题：如何让一次操作容易追踪、修改局部化，避免只会转发的多层包装？

本笔记只保存资料依据和项目推论，不是平行规范；项目执行规则唯一入口仍是 [统一工程规范](../engineering-standards.md)。以下来源均为官方公开资料，不能据此推断这些公司的所有内部系统都采用同一架构。

## 核验结论

用户提出的方向有明确工程依据：可读性、职责内聚、显式依赖、封装和避免过度设计。衡量对象是理解与修改的成本，不是机械限制调用栈只能一两层。资料也支持有收益的抽象，不能把简化等同于取消所有边界。

## 来源原意与 Kivio 推论

### 1. Google 代码审查：复杂到难理解、易改错，就是问题

原意：审查整体设计及组件交互；复杂性包括读者不能迅速理解、调用或修改时容易引入错误。应解决已知需求，不为猜测中的未来需求提前泛化；测试本身也不能无端复杂。[Google Engineering Practices：What to look for](https://google.github.io/eng-practices/review/reviewer/looking-for.html#complexity)

项目推论：审查不能只看类型、目录和测试是否通过，还要沿真实用户动作读路径。仅为统一名称而增加 Controller / Service / Manager 转发，不构成收益。

### 2. Google Go 指南：减少无益抽象，同时承认必要复杂性

原意：简单代码能顺序阅读，值与决策传播清楚，没有不必要抽象；维护性要求减少耦合、按问题结构选择抽象。接口有理解成本，必须有足够收益；实现稍复杂但更容易正确使用的 API 也可能值得保留。[Google Go Style Guide：Simplicity](https://google.github.io/styleguide/go/guide.html#simplicity)、[Maintainability](https://google.github.io/styleguide/go/guide.html#maintainability)

项目推论：TS/Rust 可借鉴这些设计判断，不照搬 Go 语法规范。优先现有模块与普通函数；保留真正隐藏协议、资源生命周期或并发细节的接口，合并不能减少调用方知识负担的包装。

### 3. Android 官方架构：不强迫简单操作穿过空用例层

原意：该指南的 Domain layer 是可选的，用于复杂业务或真实复用。强制所有数据访问经过用例，即便只是简单函数转发，会增加复杂性而收益有限；是否限制直达取决于项目。它也允许有实际复用的多级用例，因此不是“最多两层”的规定。[Android Developers：Domain layer](https://developer.android.com/topic/architecture/domain-layer)、[Data layer access restriction](https://developer.android.com/topic/architecture/domain-layer#data-access-restriction)

项目推论：Kivio 不必为每个动作补齐相同层级。简单查询可使用现有类型化命令入口；需要多步协作的动作才需要集中编排。不能借此绕过已有权限、设置保存一致性或 IPC 契约；也不照搬 Android 的类名、状态归属和生命周期规则。

### 4. Microsoft 架构原则：封装使内部修改不牵连调用方

原意：按职责分离，通过明确接口协作；外部契约不变时，内部实现应能独立调整。依赖应显式表达，状态应通过定义良好的操作修改。相同概念的规则保持单一权威，但偶然相似的代码不应被错误抽象绑在一起。[Microsoft：Encapsulation](https://learn.microsoft.com/en-us/dotnet/architecture/modern-web-apps-azure/architectural-principles#encapsulation)、[Explicit dependencies](https://learn.microsoft.com/en-us/dotnet/architecture/modern-web-apps-azure/architectural-principles#explicit-dependencies)、[DRY](https://learn.microsoft.com/en-us/dotnet/architecture/modern-web-apps-azure/architectural-principles#dont-repeat-yourself-dry)

项目推论：目标是业务规则修改集中在其所属模块，而不是把全部动作挤进一个大文件。若改内部细节还必须修改多个页面、重复参数拼装或安排其他领域的清理顺序，说明边界泄漏。这里借鉴原则，不引入该文针对 .NET Web 应用讨论的项目分层或微服务。

## 用于核对简化效果的问题

以下是资料落地的审查提示，不另设审批流程；执行要求以统一工程规范为准。

1. 从动作入口能否看出正常、失败与结束路径，还是必须追踪多个隐式回调和事件？
2. 改一个业务规则，是否主要修改所属模块？跨模块改动是契约确实变化，还是调用方知道太多内部细节？
3. 每个保留的间接层隐藏了什么真实复杂性？去掉它，是否反而让重复规则或生命周期散落到调用方？
4. 是否保留了正确性保护，并用行为测试确认简化前后结果一致，而不只统计文件或函数减少了多少？
