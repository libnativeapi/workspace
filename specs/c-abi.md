# C ABI 规范：生成管线与类型映射

> 状态：生成管线已实施；错误模型与整数宽度未决（DESIGN_REVIEW.md D2 / A1）
> 适用范围：`core/src/capi/`、`tools/codegen/`
> 核实基准：2026-08-25，28 个 capi 头中 27 个、27 个实现中 26 个为生成产物

本规范回答：**C ABI 长什么样、由谁产出、C++ 类型怎么过桥。**
句柄的所有权与失效语义是独立一篇：[handle-ownership.md](handle-ownership.md)。

## 1. 第一条：不要手写 `capi/`

`src/capi/` 里**唯一**手写的是 `string_utils_c.h` / `string_utils_c.cpp`。其余全部
带横幅：

```
// AUTO-GENERATED. DO NOT EDIT.
// Any manual changes WILL BE LOST when this file is regenerated.
```

改 C ABI 的正确路径永远是：

```bash
# 1. 改 core/src/ 下的 C++ 头
# 2. 需要新增模块时，把头文件加进 tools/codegen/shared/src/lib.rs 的 API_HEADERS
# 3. 在 workspace 根目录
./codegen
```

想让 C ABI 长成某个样子，就去改 C++ 头或改生成器，**不要改产物**。
`./codegen check` 是只读校验，产物过期时非零退出（CI 用）。

一个模块要进 C ABI，必须显式加进 `API_HEADERS`（当前 34 条）。这是刻意的：绑定仓库
里生成文件与手写文件同目录，隐式纳入会静默覆盖别人手写的封装。

## 2. 类型映射

| C++ | C | 形态 |
|---|---|---|
| 身份对象（`shared_ptr<Window>` 等） | `native_window_t` = `uint64_t` | 句柄，见 §3 |
| 值对象（`Point`/`Size`/`Rectangle`/`Color`） | 同名 `native_*_t` struct | 按值 |
| `XxxId`（`IdAllocator::IdType`） | `native_xxx_id_t` = `unsigned int` | 按值 |
| `std::string` | `char*` | 调用方所有，见 §4 |
| `std::vector<shared_ptr<T>>` | `native_x_list_t` | 见 §5 |
| `bool` / 整数 / 浮点 | `stdbool.h` / `stdint.h` 对应类型 | 按值 |
| 监听器 id | `native_listener_id_t` = `uint64_t` | `common_c.h` |

分界线就是 [object-model.md](object-model.md) 的那条：**身份对象走句柄，值对象走
struct。** 新类型过不了桥，先回去确认它的归属，而不是在生成器里开特例。

`common_c.h` 承载跨模块共享的定义（`FFI_PLUGIN_EXPORT` 导出宏、
`native_listener_id_t`、`NATIVE_INVALID_LISTENER_ID`），它本身也是生成的。

## 3. 句柄

`typedef uint64_t native_<类型>_t;`——不透明整数，不是指针。编码为
`[世代 32 位 | 槽位 32 位]`，解析时校验槽位存在、世代匹配、类型 tag 匹配。
失效句柄上的任何操作安全失败，不解引用悬垂内存。

完整规则（谁负责释放、回调参数的例外、世代失效语义）见
[handle-ownership.md](handle-ownership.md)。

### 3.1 释放函数的三种形态

实际落地的不是「统一改名 `_release`」，而是按语义分成三个函数：

| 函数 | 语义 |
|---|---|
| `native_<x>_free(handle)` | 释放调用方持有的**那一份引用**。对无效或已释放的句柄调用是安全的。 |
| `native_<x>_list_free(list*)` | 释放数组，**并**释放其中每一个句柄。 |
| `native_<x>_list_release(list*)` | **只**释放数组；其中的句柄交给调用方接管。 |

`_free` 与 `_list_release` 的区别不是历史包袱，是「要不要连带释放元素」的真实分叉。
绑定层从列表里取走句柄自行管理时用 `_list_release`，一次性用完时用 `_list_free`。

> 这一条取代 [handle-ownership.md](handle-ownership.md) §2.5 提出的「全部改名
> `_release`」——那个方案没有区分数组与元素两级所有权，实施时被上表替换。

## 4. 字符串

返回 `char*` 的 getter，**所有权归调用方**，用 `free_c_str()` 释放
（`string_utils_c.h`，仓库里唯一手写的 capi 模块）。

传入方向用 `const char*`，库内立即拷贝，不留引用。

回调参数里的字符串**只在回调期间有效**，回调返回即失效——需要留存就自行拷贝。

## 5. 列表

```c
typedef struct {
  native_display_t* displays;
  long count;
} native_display_list_t;
```

> `count` 目前是 `long`——Windows 上 32 位、其余平台 64 位，同一个 ABI 宽度不一致。
> 这是 DESIGN_REVIEW A1，待收敛为固定宽度整数。新写生成器代码时不要沿用 `long`。

## 6. 事件与回调

事件通过 `add_listener` / `remove_listener` 函数对暴露，注册返回
`native_listener_id_t`，失败返回 `NATIVE_INVALID_LISTENER_ID`（即 0）。

每个领域生成一个事件类型枚举（`NATIVE_DISPLAY_EVENT_TYPE_ADDED` 等）加一个事件
struct，C++ 侧的 `dynamic_cast` 层级在 C 侧摊平成 tag + 联合字段。

回调签名统一带 `void* user_data` 尾参。回调里拿到的句柄和字符串**都不需要也不应该
释放**（[handle-ownership.md](handle-ownership.md) §2.6 的例外条）。

## 7. 已知未决

写生成器或改 ABI 前先看这几条，避免把问题复制到下游：

| 编号 | 问题 |
|---|---|
| DESIGN_REVIEW D2 | 错误处理五种并存，C ABI 没有统一错误码 |
| DESIGN_REVIEW A1 | 整数宽度不可移植（`long count`、`native_*_id_t`） |
| DESIGN_REVIEW A3 | 回调 typedef 生成质量 |
| DESIGN_REVIEW A4 | 内部 API 泄漏进 ABI（`WindowManager` 的平台钩子被导出成 8 个 C 函数） |
| DESIGN_REVIEW A5 | C++ 重载 → C 命名策略 |

## 8. 检查单

- [ ] 没有手工编辑带 AUTO-GENERATED 横幅的文件。
- [ ] 新模块已加入 `API_HEADERS`，并跑过 `./codegen`。
- [ ] 新类型的归属明确（身份对象 → 句柄，值对象 → struct）。
- [ ] 身份对象已在 `IdTypeTag` 注册表登记（句柄表的类型校验依赖它）。
- [ ] 返回字符串的函数已在文档里写明由 `free_c_str()` 释放。
- [ ] 返回列表的函数已说明该配 `_list_free` 还是 `_list_release`。
- [ ] 改动经 `./codegen sync` 传播到三个绑定（见 workspace `AGENTS.md`）。
