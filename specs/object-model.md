# 对象模型规范：身份对象与值对象

> 状态：已实施
> 适用范围：`src/` 全部公共类型，以及 codegen 生成的 C ABI 与各语言绑定

本规范回答一个问题：**库中的一个公共类型，应该以什么方式被创建、持有、
传递和销毁。**每个公共类型必须归入下面两类之一；新增类型时先决定归属，
再写代码。

## 1. 分类总览

| | 身份对象 | 值对象 |
|---|---|---|
| 代表什么 | 一个唯一的底层资源 | 一段纯数据 |
| 拷贝 | 禁止（拷贝"身份"没有意义） | 自由拷贝 |
| 持有方式 | `std::shared_ptr` | 按值 |
| 标识 | 整数 ID（若被集合管理） | 无 |
| 状态 | 从底层资源活读，不做快照 | 自身即状态 |
| 跨 C ABI | handle（见 [handle-ownership.md](handle-ownership.md)） | 转换为对应的 C struct |

## 2. 身份对象（identity object)

代表一个唯一的底层资源：一个窗口、一个托盘图标、一个物理显示器、
一次快捷键注册。

### 规则

1. **以 `std::shared_ptr` 管理和传递。**类内 `delete` 拷贝构造、拷贝赋值、
   移动构造、移动赋值四件套，并在注释中说明"share the shared_ptr instead"。
2. **被 manager/registry 以集合管理的类型持有一个整数 ID。**
   - 类型别名在该类型自己的头文件中定义一次：
     `typedef IdAllocator::IdType XxxId;`
   - ID 在实例构造时从 `IdAllocator::Allocate<T>()` 分配，之后不变；
     `GetId()` 返回它。
   - 类型必须先在 `foundation/id_allocator.h` 的 `IdTypeTag<T>` 注册表中
     登记 tag（**只可追加，不可改号**）；漏登记是编译错误。
3. **同一底层资源的重复查询必须返回同一个实例。**manager 负责按底层
   资源的平台身份做实例缓存与去重；实例存活期间 ID 因而稳定。
4. **属性活读。**getter 每次从底层资源读取当前状态；持有的实例永远反映
   现状，不保存快照。底层资源消失后 getter 返回类型默认值。
5. **生命周期与底层资源解耦但单向感知。**资源消失（如显示器拔出）时
   实例从 manager 缓存移除，已被外部持有的 `shared_ptr` 仍安全可用，
   只是读到默认值；同一资源重新出现得到**新实例、新 ID**。

### 成员

`Window`、`TrayIcon`、`Menu`、`MenuItem`、`Shortcut`、`Display`、`Image`。

### 无集合 ID 的身份对象

`Preferences`、`SecureStorage`、`LaunchAtLogin`、`KeyboardMonitor`、
`MessageDialog` 等实例类：同样禁拷贝、以 handle 跨 ABI，但不进入任何
manager 集合，因此不定义 `XxxId` 别名、不调用 `IdAllocator::Allocate`。
它们仍需要 `IdTypeTag` 登记——那只服务于 handle 表的类型校验。

## 3. 值对象（value object）

纯数据，没有底层资源身份。

### 规则

1. 可自由拷贝；需要相等性时按成员值比较。
2. 不进 handle 表；跨 C ABI 时按值转换为对应的 C struct。
3. 不持有 ID，不注册 `IdTypeTag`。

### 成员

- 几何与外观：`Point`、`Size`、`Rectangle`、`Color`
- 输入描述：`KeyboardAccelerator`
- options 类：`ShortcutOptions`、`WindowOptions` 等
- 全部 Event 类（见第 5 节）

## 4. 案例：Display 的归属

`Display` 曾是可拷贝值类型（pimpl 深拷贝、平台字符串做 ID），是身份对象
规则的最佳反例，现按本规范归入身份对象：

- `DisplayId`（整数）在实例创建时分配；人类可读名称保留在 `GetName()`。
- `DisplayManager` 按**平台身份 key**（macOS 的 `CGDirectDisplayID`、
  Windows 的设备名等）缓存实例。key 是私有实现细节，不出现在公共 API。
- 显示器保持连接期间，`GetAll()` / `GetPrimary()` 每次返回同一
  `shared_ptr`，ID 稳定；断开后实例从缓存移除（`DisplayRemovedEvent`
  携带最后一份引用），重新连接得到新实例、新 ID。
- 平台层只实现原生枚举（`EnumerateNativeDisplays()`）；缓存、diff、
  事件发射是共享代码（`display_manager.cpp`）。
- `DisplayChangedEvent` 只携带发生变化的 display 本身，不携带 old/new
  两份"快照"——身份对象属性活读，旧状态快照本就无法成立（规则 4 的
  直接推论）。

新类型拿不准归属时，对照这个案例：**"两个实例可能指同一个东西吗？"**
可能——身份对象；不可能——值对象。

## 5. 事件中的对象引用

Event 类本身是值对象（按值构造、跨线程传递、进回调），但它可以引用
身份对象。允许两种形态：

- 携带 `std::shared_ptr<T>`（如 `DisplayEvent`）：事件让对象多活一程，
  适合"资源即将消失、监听者还需要读它"的场景（如 removed 事件）。
- 只携带整数 ID（如 `WindowEvent`）：监听者按需通过 manager 解析，
  适合对象必然还活着的场景。

事件**不得**按值内嵌身份对象（那要求身份对象可拷贝，与第 2 节矛盾）。

## 6. 新增类型检查单

- [ ] 决定归属：身份对象还是值对象？
- [ ] 身份对象：删除四件套拷贝/移动；`IdTypeTag` 登记；
      需要集合管理时定义 `XxxId` 并在构造时分配。
- [ ] 身份对象：确定谁负责实例去重（哪个 manager、按什么平台 key）。
- [ ] 值对象：确认无 ID、无 handle、C ABI 有对应 struct 映射。
- [ ] 事件引用身份对象时，选 `shared_ptr` 或 ID，不按值内嵌。
