# C++/C API 设计审查

> 审查日期：2026-08-22
> 范围：`src/` 全部公共头文件（25 个）、`src/foundation/`、`src/capi/` 生成的 C ABI 头
> 关注点：设计方向、一致性、统一性。不含实现层 bug 审查。
>
> 注：`id_allocator.h` 与 `handle_table.h` 中对 "DESIGN_REVIEW §P0-4 / §4.2" 的历史引用，
> 对应关系见文末[附录](#附录既有代码引用的对应关系)。

## 优先级约定

| 级别 | 含义 |
|------|------|
| **P0** | 方向性决策，决定 codegen 与三个语言绑定的形状，改得越晚代价越大 |
| **P1** | 一致性问题，可机械化 sweep，但需要先有 P0 的决策做依据 |
| **P2** | 文档腐烂 / 低风险清理，随时可做 |

---

## 一、统一设计方向（P0）

### D1. 事件归属模型不统一 ⭐ 影响最大

同一库中并存四种事件/回调模型：

| 类 | 模型 |
|----|------|
| `Window` | **不是** EventEmitter，`WindowEvent` 由 `WindowManager` 集中发 |
| `TrayIcon` / `Menu` / `MenuItem` | 各自继承 `EventEmitter`，对象自己发事件 |
| `Shortcut` | 双轨：`std::function` 回调（`ShortcutOptions::callback`）+ `ShortcutManager` 层的 `ShortcutActivatedEvent` |
| `KeyboardMonitor` | 手动 `Start()/Stop()`，不走 EventEmitter 的 `StartEventListening()` 自动生命周期 |

传导到 C ABI：`native_tray_icon_add_listener` 挂在对象上，window 事件却只有
`native_window_manager_add_listener`（`window_c.h` 定义了 `native_window_event_callback_t`
但 window 自身没有 add/remove listener）。

**建议**：
- [ ] 定规则：对象级事件挂对象（`Window` 也应可 `AddListener`），系统级事件（增删、全局监控）挂 manager。
- [ ] `Shortcut` 的 function 回调收敛为事件模型（或反之，二选一）。
- [ ] `KeyboardMonitor` 改用 `StartEventListening()/StopEventListening()` 自动生命周期，移除公开的 `GetInternalEventEmitter()`。

### D2. 错误处理策略五种并存

| 风格 | 代表 |
|------|------|
| 结构化 result（错误码+消息） | `UrlOpener::Open` → `UrlOpenResult`（全库唯一） |
| 返回 bool | `TrayIcon::SetVisible`、`Application::SetIcon/SetMenuBar` |
| 返回 void，无失败通道 | `Window` 全部 setter |
| 返回 nullptr | `Image::FromFile/FromBase64`、各 manager 的 `Get` |
| 抛异常 | `DisplayManager::GetPrimary`（`std::runtime_error`，全库唯一） |

C 层没有 `native_get_last_error` 之类的统一通道；无效 handle 时静默返回默认值，
调用方无法区分「成功返回默认值」与「handle 已失效」。

**建议**：
- [ ] 在 IR 层统一错误模型（候选：状态码 + out 参数；或 thread-local last_error）。
- [x] 移除 `GetPrimary()` 的异常（跨 C ABI 异常必须在边界吞掉，核心层不如不抛）—— 随 D3 改为返回 `shared_ptr`，失败返回 nullptr。

### D3. 对象身份 / 生命周期模型不统一 ✅ 已解决

| 类 | 模型 |
|----|------|
| `Window` / `TrayIcon` / `Menu` / `MenuItem` | shared_ptr 身份对象 + 整数 ID |
| `Display` | ~~可拷贝值类型（pimpl 深拷贝），ID 是 `std::string`~~ → 身份对象 + `DisplayId`，manager 按平台身份 key 缓存去重 |
| `Shortcut` | ~~无 pimpl 的普通值成员类~~ → 已禁拷贝禁移动（pimpl 下沉归 C8） |
| `LaunchAtLogin` / `Preferences` / `SecureStorage` | 禁拷贝禁移动的实例类（无集合 ID 的身份对象） |

**决议**（规则全文见 workspace 仓库 `../specs/object-model.md`）：
- [x] 明确两类对象：**身份对象**（shared_ptr + `IdAllocator` 整数 ID，禁拷贝，重复查询返回同一实例）与**值对象**（Point/Color 等纯数据）。
- [x] `Display` 归入身份对象：`DisplayId` 整数 ID，`GetName()` 保留字符串；`DisplayManager::GetAll/GetPrimary` 返回 `shared_ptr`，`GetPrimary` 失败返回 nullptr（顺带消掉 D2 提到的全库唯一异常）；`DisplayChangedEvent` 不再携带 old/new 快照。

### D4. 单例实现三种写法

- Meyer's：`WindowManager` / `Application` / `ShortcutManager` / `TrayManager` / `HandleTable`
- 手写静态裸指针：`DisplayManager`（`static DisplayManager* instance_`，且无 pimpl）
- 能力型 API 形态分裂：`UrlOpener` 单例、`LaunchAtLogin` 多实例、`AccessibilityManager` 单例但仅两个方法

**建议**：
- [ ] `DisplayManager` 改 Meyer's + pimpl，与其余 manager 对齐。
- [ ] 无状态能力型 API（`UrlOpener`、`AccessibilityManager`）统一为 static 函数集或保留单例，但只选一种。

### D5. 能力检查命名 / 形态四种

`TrayManager::IsSupported()`（实例、非 const）、`ShortcutManager::IsSupported()`（实例）、
`LaunchAtLogin::IsSupported()`（static）、`SecureStorage::IsAvailable()`（static、异名）、
`UrlOpener::IsSupported() const`。

- [ ] 统一为 `static bool IsSupported()`。

### D6. 对象创建 / 注册模式不统一

- `Window()` 构造函数隐式注册进 `WindowRegistry`；
- `TrayIcon` 文档要求「直接 `make_shared`」，但 `TrayManager` 又维护 `trays_` 映射（注册路径不明）；
- `Shortcut` 必须走 `ShortcutManager::Register`；
- `Image` 只能走静态工厂。

多处文档示例调用**不存在**的 `WindowManager::GetInstance().Create(options)`
（`window_manager.h`、`application.h`、`positioning_strategy.h`）。

**建议**：
- [ ] 二选一：补 `WindowManager::Create(WindowOptions)`（Electron 风格，同时解决 Window 构造参数无处传的问题），或清理全部文档示例。
- [ ] 明确 `TrayIcon` 的注册语义（构造即注册，或显式 `TrayManager::Add`）。

### D7. Manager 职责越界

- `WindowManager` 公开暴露平台 swizzle 内部机制（`SetWillShowHook` / `HandleWillShow` /
  `CallOriginalShow` 等），并被 codegen 原样导出为 8 个 C 函数；
- `DisplayManager::GetCursorPosition()` 与显示器管理无关（应属未来 Cursor/Mouse API）。

**建议**：
- [ ] 给 codegen IR 增加 internal/exclude 标注机制，将平台钩子从公共 C ABI 摘除（另见 A4）。

---

## 二、C++ API 一致性（P1）

### C1. const 正确性
- `TrayIcon`：`GetId` / `GetTitle` / `GetTooltip` / `GetContextMenu` / `GetContextMenuTrigger` /
  `GetBounds` / `IsVisible` 全部非 const；`Window` / `MenuItem` 对应 getter 是 const。
- 各 manager 的 `Get` / `GetAll` 非 const；`AccessibilityManager::IsEnabled()` 非 const。
- [ ] 全库 getter 一次 sweep 补 const。

### C2. optional 语义
- `TrayIcon::SetTitle(optional<string>)` 可清空；`Window::SetTitle(std::string)` 不可清空。
- `MenuItem::SetAccelerator` 收 optional，`GetAccelerator` 却返回裸 `KeyboardAccelerator`（get/set 不对称，靠 `IsEmpty()` 兜底）。
- `Storage::Get(key, default_value="")` 无法区分「不存在」与「空值」（建议返回 `optional<string>`；另见 A2 的 C 层折叠问题）。
- [ ] 定规则：可清空属性 setter/getter 对称使用 `std::optional`。

### C3. setter 返回值
`SetVisible → bool`、`Window::SetXxx → void`、`Application::SetIcon/SetMenuBar → bool`、
`LaunchAtLogin::SetDisplayName → bool`（"stored locally" 语义）。
- [ ] 随 D2 的错误模型统一。

### C4. 可见性 idiom 两种
`Window::Show/ShowInactive/Hide` vs `TrayIcon::SetVisible(bool)`（后者文档自述
"replaces the previous Show() and Hide()"，说明方向摇摆过）。
- [ ] 二选一并全库统一（倾向保留 Window 的 Show/Hide + IsVisible，TrayIcon 增补对齐或反向）。

### C5. 枚举风格两套
- k 前缀：`DisplayOrientation::kPortrait`、`UrlOpenErrorCode::kNone`；其余 PascalCase。
- `DisplayOrientation` 用角度做枚举值（90/180/270），magic value 并原样漏进 C 枚举。
- [ ] 统一 PascalCase；枚举值从 0 顺序编号，角度另设 `GetRotationDegrees()`。

### C6. 事件类结构
- `WindowEvent` 基类存 id；`TrayIconEvent` 基类为空，三个子类各自重复 `tray_icon_id_`；
- Menu 缺 `MenuItemEvent` 中间基类，`MenuId` / `MenuItemId` 事件混在同层；
- `DisplayEvent` 携带整个 `Display` 拷贝而非 id。
- C 层后果：window 事件 id 在 struct 顶层，tray 事件 id 在每个 union 分支重复一份。
- [ ] 统一为「基类存 id」模式。

### C7. `void*` 包装构造函数
- explicit 不一致：`Window(void*)` / `TrayIcon(void*)` / `Display(void*)` 非 explicit，`Menu(void*)` / `MenuItem(void*)` explicit。
- 这些接管所有权的构造全部被导出到 C ABI（`native_window_create_with_native_window` 等），所有权语义在 C 文档中缺失。
- [ ] 全部加 explicit；C 层文档补所有权说明或随 A4 摘除。

### C8. pimpl 纪律
- 纯 pimpl：`Window` / `TrayIcon` / `Menu` / `Preferences` 等。
- 混合（状态成员裸露在公共头，破坏封装与 ABI 稳定）：`Application`（`initialized_` / `running_` /
  `exit_code_` / `primary_window_`）、`TrayManager`（`trays_` / `next_tray_id_` / `mutex_`）、
  `ShortcutManager`（两个 map + mutex + `enabled_`）、`MessageDialog`（`modality_`）、
  `AccessibilityManager`（`enabled_`，且文档声称线程安全但无任何同步）。
- `ShortcutManager::Impl`、`UrlOpener::Impl` 是 **public** 嵌套类（实现接口泄漏进公共头）。
- `KeyboardMonitor` 成员名 `impl_` 而非 `pimpl_`。
- [ ] 状态成员全部下沉 pimpl；Impl 一律 private；命名统一 `pimpl_`。

### C9. ID 基础设施双轨
- `WindowId` 在 `window.h`（typedef）与 `window_registry.h`（using）重复定义；`ShortcutId` 在
  `shortcut.h` / `shortcut_manager.h` 重复；typedef 与 using 混用。
- `TrayManager::next_tray_id_`、`ShortcutManager::next_shortcut_id_` 与 `IdAllocator` 并存。
- [ ] ID 别名每个一处定义（统一 using）；计数器统一走 `IdAllocator`。

### C10. 具体功能缺口 / 零散问题
- [ ] **`MessageDialog` 拿不到用户选择**：无按钮配置、无结果返回（模态 `Open()` 只返回是否成功打开；示例还写着 "Would you like to update?"）。最明显的可用性缺口。
- [ ] `Dialog` 基类无 Id、非 EventEmitter，与 Window/TrayIcon 方向不一致；将来加 FileDialog 时会放大。
- [ ] `AccessibilityManager` 有 `Enable()` 无 `Disable()`；`IsEnabled()` 只读本地 flag，不查系统真实授权状态。
- [ ] `SecureStorage` 头文件自述 stub（"encryption is not yet implemented"），却已通过 C ABI 铺到所有绑定 —— 至少在绑定层同步该警告。
- [ ] `Window::SetSize(Size, bool animate)` 是唯一强制双参的 setter，`animate` 应给默认值。
- [ ] `IsIgnoreMouseEvents` 命名不合语法（其余 Is 系列均为形容词/分词）。
- [ ] `PositioningStrategy::Relative(const Window&)` 内部存**裸 `const Window*`**，与全库 shared_ptr 生命周期模型冲突，存在悬垂风险（C 层允许跨调用持有 strategy handle，放大该风险）。
- [ ] `KeyboardAccelerator` 构造初始化列表顺序与成员声明相反（-Wreorder）。
- [ ] `ModifierKeysChangedEvent` 存裸 `uint32_t` 而非 `ModifierKey`，且向基类塞 `keycode=0`。
- [ ] 按键表示两套并存：事件用 `int` keycode，accelerator 用字符串键名——考虑引入统一键码枚举。
- [ ] `Image` 公开深拷贝构造 + 移动构造（pimpl 深拷贝），与 D3 定下的身份对象禁拷贝规则矛盾（2026-08-31 补记，规则见 `../specs/object-model.md` §2）。修复方向：删掉这两个构造。

---

## 三、C ABI 层（P0/P1，多数应在 codegen 统一）

### A1. 整数宽度不可移植 ⭐ 建议最优先修
- `native_window_id_t` 等全部是 `unsigned int` 而非 `uint32_t`；
- 所有 list 的 `count` 用 `long`；`get_size` 返回 `unsigned long` ——
  Windows LLP64 下 32 位、其他平台 64 位，**同一 ABI 在不同平台宽度不同**，
  对 Dart/C#/Rust FFI（各自硬编码宽度）是实际隐患。
- [ ] codegen 统一映射为 `<stdint.h>` 定宽类型（id → `uint32_t`，count/size → `int64_t` 或明确约定）。

### A2. 空串 / 缺失折叠
`string_utils_c.h` 的 `to_c_str` 对**空字符串返回 nullptr** ——
`optional<string>`（未设置）与空字符串在 ABI 上不可区分，`get_title` 等无法往返。
- [ ] 空串返回合法的 `""` 分配；nullptr 仅表示「无值」。

### A3. 回调 typedef 生成质量
- 同一 `std::function<void()>` 生成三个名字：`native_shortcut_options_callback_t`、
  `native_shortcut_create_with_id_and_accelerator_and_callback_t`、`native_shortcut_set_callback_t`。
- `native_window_manager_set_will_show_hook_callback_t` 参数名泄漏为 `arg0`，类型退化为
  `unsigned int` 而非 `native_window_id_t`。
- [ ] 相同签名共享 typedef；参数名与语义类型在 IR 中保留。

### A4. 内部 API 泄漏进 ABI
- `native_shortcut_create_with_id_*`（C++ 文档明说应由 manager 创建）；
- `native_shortcut_manager_emit_shortcut_activated`（注释 "internal use"）；
- `native_window_manager_handle_will_show/hide`、`call_original_show/hide`、hook 系列（平台 swizzle 内部机制）;
- `native_display_create()`（空 Display 无意义）。
- [ ] 依 D7 的 internal/exclude 标注机制摘除。

### A5. C++ 重载 → C 命名策略
`register_with_accelerator_and_callback` / `unregister_with_id` / `get_with_accelerator`
等机械后缀可读性差。核心 C++ 层减少语义重载（如 `Unregister(id)` 与按 accelerator 反注册拆成两个名字），生成名自然变好。
- [ ] 审一遍所有 C++ 重载，语义不同的拆名。

### A6. 杂项
- [ ] 所有 `get_native_object` 的注释都是 display 的（"NSScreen*, HMONITOR, ..." 被复用到 window/tray/image）——生成器文档模板错误。
- [ ] `FFI_PLUGIN_EXPORT` 在每个头重复定义；`#if _WIN32` 应为 `#ifdef _WIN32`；无 dllimport 分支（使用方 include 语义不对）。应收敛到统一 export 头。
- [ ] `dialog_c.h` 只剩一个 modality 枚举，Dialog 抽象在 C 层消失（随 C10 的 Dialog 设计一并考虑）。
- [ ] C 枚举继承了 C++ 的角度值（`NATIVE_DISPLAY_ORIENTATION_LANDSCAPE = 90` 等，随 C5 修复）。

---

## 四、文档腐烂（P2，建议一次清完）

- [ ] 引用不存在的 API：
  - `WindowManager::GetInstance().Create(options)` —— `window_manager.h`、`application.h`（2 处）、`positioning_strategy.h`
  - `Menu::CreateItem` —— `tray_icon.h` 示例
  - `Image::FromRawData` / `FromSystemIcon` / `IsValid`、`ImagePixelFormat` —— `tray_icon.h` / `menu.h` / `image.h` 示例
  - `FileDialog` —— `dialog.h`
- [ ] 与签名矛盾：
  - `TrayIcon::SetContextMenu` 文档说 "The Menu object is copied internally"，参数实为 `shared_ptr`
  - `MessageDialog` 注释提到不存在的 `Dialog::pimpl_`
- [ ] 残留物：
  - `window.h` / `keyboard.h` 共 6 处空的 "Get the static type index for this event type" 注释
  - `window.h` 中注释掉的 `SetBackgroundColor` 声明（下文已有正式版本）
  - `image.h` 引入未使用的 `<optional>` / `<vector>`
- [x] `id_allocator.h` / `handle_table.h` 引用的 DESIGN_REVIEW 文档缺失 —— 本文件即落点，见附录。

---

## 建议路线

1. **先定调（P0）**：D1 事件模型、D2 错误模型、A1 ABI 整数宽度。三者决定 codegen 与三个绑定的形状。
2. **机械 sweep（P1）**：C1–C5（const / optional / 返回值 / idiom / 枚举风格）可各自一个 commit 完成；C8 pimpl 下沉注意 ABI 影响。
3. **随手清（P2）**：第四节文档项与 A6 杂项。
4. 每一批改动走 `./codegen sync` 全量传播，保持 core 与绑定同步演进。

---

## 附录：既有代码引用的对应关系

| 代码中的引用 | 对应 |
|--------------|------|
| `id_allocator.h`：“DESIGN_REVIEW §P0-4 的 handle-table 工作” | 已实现：`foundation/handle_table.h`（generational handle + 类型 tag 校验），所有权规则见 workspace 仓库 `../specs/handle-ownership.md` |
| `handle_table.h`：“DESIGN_REVIEW §4.2 的 explicit-lifetime 工作（折叠进统一 Context）” | 未实现。与本文 D4（单例统一）同属生命周期议题，待规划 |
