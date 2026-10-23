---
name: egui-components
description: 指导 LumaDesk/LumaKit 的 Rust egui 组件选型、布局、表单、弹层、键盘与可访问性、异步保存及 UI 验证。涉及 egui/eframe、Switch/Checkbox、Dialog/Modal、ComboBox、聊天或设置界面、shadcn-svelte/Bits UI 原生移植时使用。必须先确认项目锁定版本与已有共享组件，不把新版本 API 或预研示例当作已实现能力。
---

# egui 组件使用规范

目的：阻止“画得像就当等价”“编译过就当可用”。只封装真实需要的差异，优先已存在共享组件，再用 egui 内置组件。

## 1. 开始前必须确认

1. 读取宿主与框架 Cargo.toml/Cargo.lock，确认 egui、eframe、egui_extras、Markdown、图标库的实际版本和 feature。存在 workspace 依赖时同时检查根清单。
2. 读取 `LumaKit/crates/lumakit-egui/src/lib.rs` 的真实导出和相关调用者；不能臆造 `Switch`、`Field`、`AlertDialog` 等共享 API。
3. 核对需求的业务语义：修改草稿还是立即生效、是否单选、是否破坏性、是否可取消、保存失败如何恢复。
4. 调用精确版本的官方文档并读已安装源码；通用搜索和 Context7 的 latest 只作线索，不作旧版签名证据。
5. 实施前列出修改文件、选用组件、补齐的交互及验证场景。参考组件外形不是完整合同。

### 当前版本边界

2026-10-23 核对：产品仍锁定 egui/eframe **0.33.3**，本 skill 不自动升级依赖。**0.37.0 是已发布的正式版本**，隔离预研已通过，不能称它“有 bug 所以不能用”。

| 项目 | 0.33.3 产品 | 0.37.0 预研 |
| --- | --- | --- |
| 面板 | `TopBottomPanel/SidePanel` + `show_inside` | `Panel::top/bottom/left/right` + `show(ui, ...)` |
| 帧入口 | `Context::run` | `Context::run_ui`；无 UI 回调需要另查 run_pass |
| 自定义控件语义 | `WidgetType` | `Role`，可用 `Role::Switch` |
| egui AccessKit | 可选特性，需检查启用情况 | accesskit 是直接依赖，不能在 egui 清单中写 feature `accesskit` |
| headless 输出 | 仍需处理输出 | 忽略未应用纹理 delta 在 debug 测试会 panic；无 renderer 的探针显式 clear，正式集成必须应用纹理 |

跨版本升级须同时迁移宿主与共享控件，避免两份不兼容 `Ui` 类型；重新核对依赖后更新此表。详见 [预研证据](references/research.md)。

## 2. 组件选择：语义先于形状

| 需求 | 首选 | 不能省略 |
| --- | --- | --- |
| 立即生效的二态设置 | 经验证的共享 Switch；未存在时先设计一个最小 Widget | 标签、焦点、键盘、禁用、提交/失败回滚；不要画一个不可操作的圆 |
| 多选项/能力集合/表单确认 | `Checkbox` | checked/unchecked；mixed 状态须单独建模，egui indeterminate 不自动提供三态存储 |
| 少量互斥项 | `radio_value` 或有明确选中语义的分段按钮 | 一个权威值、键盘、标签；不能多个 bool 假装单选 |
| 普通下拉 | `ComboBox` | 当前/无效值、禁用、关闭策略；不要静默选第一项 |
| 可搜索模型选择 | 复用已存在的模型选择实现或组合 popup + 搜索 + 分组列表 | 方向键、高亮、Enter/Esc、焦点；ComboBox 里加输入框不自动等价 Bits Combobox |
| 普通模态内容 | `Modal` | 背景阻断、标题、焦点进入/恢复、关闭规则 |
| 破坏性确认 | `Modal` + 明确确认/取消状态 | 默认安全动作，Esc/遮罩只能取消；普通 Window 不等价 AlertDialog |
| 非模态浮动工具窗口 | `Window` | 不宣称阻断背景，不用它冒充确认弹层 |
| 输入/多行/密码 | `TextEdit` | 稳定 ID、标签、校验、IME、滚动及禁用 |
| 动作/切换按钮 | `Button`，必要时 selected | 区分动作与设置；忙碌态禁用，主次/危险颜色来自语义主题 |
| 说明/错误/空状态 | Label/Frame/布局组合 | 明确归属、可读、长期错误不只 toast；不必各造一套组件框架 |
| 滚动/折叠/菜单/提示 | 内置 ScrollArea/CollapsingHeader/Menu/Tooltip | 实际版本 API、滚动所属区、关闭与焦点；不模拟浏览器 z-index |

Switch 的控件只负责交互与返回变更，不发 HTTP、不保存文件。不能仅把 Checkbox 换外观就宣称功能对齐。

当前产品“启用”修改整份草稿并触发保存；参考实现独立提交启用、期间禁用、失败恢复。若实施对齐，需同时设计此提交边界，保留其它未保存字段。不得为没有业务能力的选项绘制可用开关。

## 3. 布局硬规则

- 聊天/设置根区域有明确可用 Rect；头部和底部控件先消费空间，余下矩形交给滚动消息。不要用 bottom_up 嵌套 top_down 再猜消息高度。
- CentralPanel 在同级其它面板之后。组件内使用当前版本的嵌入方式；顶级面板不要在另一个顶级面板闭包中开启。
- 容器 ID、弹窗、编辑器、列表项按组件实例和稳定业务 ID 隔离；不靠显示文本或可变列表序号保持状态。
- 长名称截断但保留完整提示；正文按可用宽度折行。仅代码/表格的溢出横向滚动，不能把无限宽度交给普通段落。
- 输入、错误说明和工具栏要有预算；长错误不能推走保存恢复/发送按钮。不要用裁剪掩盖重叠。
- 单位是逻辑 point；CSS px 与屏幕物理像素不直接等价。短窗口、高 DPI、长正文、长草稿都要检查。

## 4. 状态、键盘、可访问性

- 区分：default、hover、focus、pressed、disabled、busy、error、success。不是每项都要动画或 toast，但不能只有静态画法。
- 自定义 Widget 复用 `Sense::click`、`Response::clicked`、`mark_changed`；不要再用 key_pressed 单独切一次导致键盘双触发。发布前验证 disabled 和真实鼠标/键盘行为。
- 名称、角色、checked/selected 与 enabled 必须进入控件元数据；视觉标签要关联输入。写了元数据不代表 OS 读屏已验证，宿主 AccessKit 桥接必须另验。
- 普通 Dialog 与 AlertDialog 分开定义确认/取消、Esc、遮罩、焦点进入/恢复及嵌套 popup 优先级；不能把 `Modal::should_close()` 当“用户确认”。
- Ctrl+Enter/Enter 快捷键不抢中文 IME；长按不重复提交。合成测试要给 key-up，不能连续 key-down 后误把系统 repeat 当缺陷。
- IMGUI 可以多 pass：绘制不直接执行业务副作用。收集动作后让控制器做校验/去重，HTTP 与文件 I/O 不在布局闭包中跑。
- 点击即时设置：保存期间禁用；失败回到已提交状态并保留错误；成功才刷新权威快照。更改草稿必须保留明确的保存语义。

## 5. 共享与性能边界

- 已有 ChatPanel 就用 ChatPanel；绘制层不持有模型密钥、文件仓库或网络客户端。
- OS 字体、主题和数据目录由宿主决定；不从通用组件猜产品策略、邀请链接或笔记上下文。
- 不每帧克隆全量模型/历史、解析全部 Markdown 或写文件。缓存有失效依据与上限，当前活动流之外的回答不重复解析。
- 后台任务有取消、唯一终态、唤醒和关闭收尾；有界队列满时不能直接 join。保存失败保留可恢复状态，不“成功关闭”丢资料。
- 没有真实复用需求就不造泛型组件注册表、插件系统、factory 或大而全 trait。

## 6. 验证与交付

1. 最小 runnable 检查证明消费者行为：开关一次激活只变一次、禁用不变、失败回滚、模态取消不确认、ID 不串。
2. 用真实 egui 帧捕获 Rect 验证头部/正文/输入无交叠，按钮在 viewport 内；不是比较源代码文字。
3. 运行改动路径并观察状态。headless 几何/AccessKit 输出不等于窗口视觉/OS 读屏；实际 UI 变更必须实窗检查。
4. 更新了依赖要编译真实宿主与共享消费实例，不能只编译一个 Widget。
5. 报告只写观察到的版本/命令/结果/限制；不可拿预研通过宣称正式应用已升级。

[可重跑的隔离预研](probes/Cargo.toml)验证 **0.37.0**，不是生产组件库，也不提供可直接 import 的 LumaKit Switch。Windows 先按项目方式注入 MSVC，再 `cargo +1.99.0 test --manifest-path .claude/skills/egui-components/probes/Cargo.toml` 和 `cargo +1.99.0 run --manifest-path .claude/skills/egui-components/probes/Cargo.toml`。详细命令与证据边界见 research.md。
