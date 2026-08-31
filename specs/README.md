# specs

`core/` 的设计规范。**这里写「为什么这样定」和「新代码必须遵守什么」**；
怎么用某个 API 看头文件的 Doxygen 注释，怎么跑生成器看
[tools/codegen/README.md](../tools/codegen/README.md)。

## 读什么

| 规范 | 回答的问题 | 状态 |
|---|---|---|
| [architecture.md](architecture.md) | 一段新代码放哪一层、哪个目录、叫什么名字 | 已实施 |
| [object-model.md](object-model.md) | 一个公共类型怎么被创建、持有、传递、销毁 | 已实施 |
| [platform-seam.md](platform-seam.md) | 平台相关的状态和代码藏在哪里、怎么藏 | 已实施 |
| [event-system.md](event-system.md) | 事件怎么定义、怎么发、在哪个线程跑 | 已实施 |
| [managers.md](managers.md) | 系统级资源由谁持有、三张查找表怎么分工 | 已实施 |
| [c-abi.md](c-abi.md) | C ABI 长什么样、由谁产出、类型怎么过桥 | 生成管线已实施 |
| [handle-ownership.md](handle-ownership.md) | C ABI 句柄的所有权与失效语义 | 已决策，实施中 |

新增一个跨平台模块，按顺序读 architecture → object-model → platform-seam；
要过 C ABI 再读 c-abi + handle-ownership。

## 与 DESIGN_REVIEW.md 的分工

- **specs/** = 已经定下来的规则，新代码照做。
- **[DESIGN_REVIEW.md](../DESIGN_REVIEW.md)** = 2026-08-22 那次审查列出的**未决**
  不一致和待办项。规范里凡是写「未决 / 待收敛」的地方都会指向它的具体编号。

一条问题在 DESIGN_REVIEW 里被解决后，结论落到对应的 spec，DESIGN_REVIEW 里勾掉。

## 写规范的约定

- 每篇开头给出**状态**、**适用范围**、**核实基准日期**。规范会腐烂，日期让读者知道
  该不该重新核对。
- 断言要能在代码里被验证。写「当前 14 个头文件用 PIMPL」这种可数的事实，
  而不是「大部分类使用 PIMPL」。
- 已知的不一致要写出来并指向 DESIGN_REVIEW 编号，不要粉饰成统一。
- 结尾放检查单，让规范可以当 code review 清单用。
