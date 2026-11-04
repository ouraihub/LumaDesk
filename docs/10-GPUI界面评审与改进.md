# GPUI 界面评审与改进

2026-11-03 对比 LumaDesk GPUI（`experiment/gpui-kit`，gpui-kit 0.9.0）与 ThinkRail Native（同为 gpui-kit 0.9.0）的布局与视觉。依据：两边 release 实窗截图（隔离数据目录）与 UI 源码通读。本文是后续跟进的唯一记录；完成一项更新一次“进度”。

## 1. 结论

差距的根因不是配色，而是 ThinkRail 有一层所有视图都必须经过的设计系统，LumaDesk 没有：

- ThinkRail 的间距、尺寸、圆角、字体样式、颜色角色集中在 `crates/ui`（`metrics.rs`、`theme/roles.rs`），视图只引用名字，并写成规则（其 04-detail §9.1）。
- LumaDesk 原有令牌只有 `MONO = 12`、`RADIUS = 8`（`theme.rs:7-10`）；间距、字号、层级由各页面临时选择，因此整体平、散、不统一。

## 2. 主要差距

| 方面 | ThinkRail | LumaDesk（评审时） |
| --- | --- | --- |
| 窗口外框 | 无系统标题栏，40px 自绘顶栏承载品牌、面包屑、状态 | 系统标题栏 + 页签栏两层外框（`main.rs:280`、`main.rs:186-189`） |
| 间距与尺寸 | `space::S0…S64` 固定梯度；行 22、表头 28、控件 24 | 每处自选 `p_2`～`p_6`、`gap_1`～`gap_4`；设置导航 `p_3` 与页面 `p_4` 叠加 |
| 字体 | 内置 Geist / JetBrains Mono，22 个命名样式，统一字重与行高 | 不设字族与行高；全仓库只有一处 SEMIBOLD（`chat.rs (kit):133`） |
| 层级 | 文字 default / muted / subtle；表面 workspace / sidebar / content / elevated；`border_muted` 分隔 | 字段标签与正文同字号同色；muted 文字仅约 5 处；侧栏、页签、内容同一米色 |
| 颜色 | 60+ 语义角色映射约 100 个 Kit 颜色槽；强调色克制 | 默认红色强调色与 danger 同值（`theme.rs:123` 与 `:171`）；主按钮 hover/active 等于 primary（`theme.rs:149-151,159-161`） |
| 组件 | PanelHeader、TreeRow、TabStrip、IconButton；设置页统一 group/row | 页签、侧栏、设置导航全是 ghost Button；`●○▸▾×＋←` 字符代替图标；设置操作全是实心按钮 |

实窗可见的问题：

- 启动台五个卡片挤在左上，卡片与背景几乎同色。
- 设置导航居中、无底色；“主题”“强调色”等分组标题与说明无层级。
- 对话输入框无可见边框或底色（`Textarea.appearance(false)`，`chat.rs (kit):204`）；“尚未配置聊天模型”用红字，默认主题下与强调色同色。
- 页签栏“外观”“设置”是带边框按钮，旁边“+”是 ghost，样式不一致。

代码层面：单行样式链常超过 200 字符（`model_settings/view.rs:258-274`、`theme.rs:123,156-170`）；`search_settings.rs` 51.6KB、`model_settings.rs` 47.2KB、`chat.rs (kit)` 40.8KB；聊天的历史、模型、工具、更多菜单都用同一种居中 460px 弹窗；界面露出“宿主未接入”等开发备注。

LumaDesk 做得好的：颜色集中在 `theme.rs` 且有对比度测试；滚动结构正确。暖纸色是风格选择，问题在于只有一层表面。

## 3. 改进计划

按收益排序；1、2 是根本，后续多为机械替换。

1. **尺寸与字体令牌**：共享模块提供间距梯度、控件与外框尺寸、圆角、命名字体样式。
2. **颜色角色**：文字三级、表面分层、`border_muted`、danger 与强调色分离、恢复主按钮 hover/active、统一遮罩。
3. 去掉系统标题栏，页签栏改为自绘标题栏。
4. 设置页统一 `group` / `row(label, help, control)`；行内操作改 ghost 或图标按钮；开关改 Switch。
5. 对话页：输入区有底色卡片；正文限制最大宽度并居中；菜单改锚定 Popover；字符图标换 `IconName`。
6. 字体：内置界面字体或显式中文回退，统一行高。

## 4. 第 1、2 项设计

### 位置与规则

> 2026-11-04 起本节的 `lumakit-gpui/src/style.rs` 已删除，由 LumaKit `lumakit-design` 取代（见 §5 最后一行）；以下保留为当时的设计记录。

令牌和角色放在 `LumaKit/crates/lumakit-gpui/src/style.rs`，LumaDesk 与共享聊天面板共用一份。规则：

- 视图的间距、尺寸、圆角只用 `space` / `size` / `radius`；文字层级用 `text::*` 经 `.type_style(..)`；颜色只用 `Roles::of(cx)`。
- 不在视图里写十六进制颜色；`src/theme.rs` 的调色板是唯一颜色来源（外观页的色块、色卡预览本身就是调色板，属于例外）。
- 需要新尺寸时先加令牌；只在一个文件使用的布局值（如模型页断点 760、聊天弹层宽 460）在该文件顶部命名为常量，不在调用处写 `px(..)`。窗口初始尺寸与最小尺寸在 `main` 中保留。

### 尺寸与字体

- `space`：2/4/6/8/12/16/24/32 px。页面内边距 24，分组间 24，组内 8，卡片内边距 12，消息间 16。
- `size`：侧栏 64、设置导航 180、窄窗断点 700、启动台卡片 112×88、色块 40、色卡 112×42。
- `radius`：SM 4 / MD 6 / LG 8；Kit 控件圆角取 LG。
- `font`：基准 14（舒适）/ 13（紧凑），等宽 12。
- `text`：字号以 rem 表示，基准 14px，Kit `Root` 把主题字号设为窗口 rem，所以“紧凑”密度会按比例缩小全部字号。字重与行高固定：

| 样式 | 基准下字号 | 字重 | 行高 | 用途 |
| --- | ---: | --- | ---: | --- |
| `BODY` | 13 | 常规 | 1.5 | 聊天正文、问题气泡 |
| `META` | 11.5 | 常规 | 1.4 | 说明、模型名、状态、诊断 |
| `LABEL` | 13 | 中等 | 1.4 | 字段与选项组标题 |
| `SECTION` | 14 | 半粗 | 1.4 | 面板头、二级标题 |
| `TITLE` | 15 | 半粗 | 1.3 | 页面标题 |

### 颜色角色

`Roles::of(cx)` 从当前 Kit `ThemeColor` 推导，不另设全局状态：宿主只维护 Kit 主题，NativeLight 分支也自动适用。

| 角色 | 来源 | 用途 |
| --- | --- | --- |
| `text` / `text_muted` | `foreground` / `muted_foreground` | 正文 / 说明与元数据 |
| `surface_chrome` | `sidebar` | 页签栏、侧栏、设置导航、聊天头 |
| `surface_content` | `background` | 页面 |
| `surface_elevated` | `popover` | 回答卡片、输入卡片、弹层 |
| `border` / `border_muted` | `border` / `border` 55% 透明度 | 卡片描边 / 面板分隔 |
| `primary` / `selected` | `primary` / `accent`（primary 18% 混合） | 选中强调 / 选中底色与用户气泡 |
| `danger` | `danger` | 错误 |
| `overlay` | `overlay` | 所有遮罩 |

文字只做两级：`muted_foreground` 已按 4.5:1 对比度下限选定，再降一级会不达标。

`theme.rs` 侧：

- danger 改为胭脂红 `#b01c45` / 深色 `#ff8db0`，朱红强调色保持不变。测试断言 danger 与每个强调色色相相差至少 20°，并在页面、外框、卡片三种表面上达到 4.5:1。
- 主按钮与 danger 的 hover / active 由原色加深 10% / 18% 得到；两个调色板分支共用 `apply_state_colors`。
- Kit `overlay` 设为 40% 黑，聊天弹层与模型对话框共用。

### 视图迁移（本轮范围）

只换令牌与角色，不改结构、控件、文案和交互（遵守 `AGENTS.md`）：

- 外框 `main.rs`：页签栏、侧栏、设置导航用 `surface_chrome` 与 `border_muted`；去掉导航与页面的双重内边距。Kit Button 固定居中内容，设置导航用 `nav_item` 让标签占满整行后左对齐。
- 标题栏（第 3 项，用户 2026-11-03 要求）：去掉系统标题栏，页签栏即窗口标题栏（Kit `TitleBar`，高 `size::TITLE_BAR` 40），右侧由 Kit 绘制最小化/最大化/关闭。页签、“+”、外观/设置各自 `occlude()`，其余空白处拖动窗口；启动失败页也带同一标题栏，否则无法移动或关闭窗口。窗口标题仍为“LumaDesk GPUI”，用于任务栏与 Alt+Tab。
- 启动台、外观、MCP、网络搜索、模型服务、模型编辑：标题 `TITLE`，字段标签 `LABEL`，说明 `META` + `text_muted`；分隔线 `border_muted`；原来无色的卡片描边补上 `border`。
- 共享聊天 `lumakit-gpui/src/chat.rs`：聊天头 `surface_chrome`；回答卡片 `surface_elevated`，描边不再因选中变成强调色（选中由“已选上下文”按钮表示）；文字区与工具栏合为一张有底色、有描边的输入卡片；提示与诊断用 `META`。

### 验收

- 现有主题测试与新增测试通过；LumaDesk、LumaKit 全部测试通过。
- 实窗截图对比启动台、外观设置、对话页，改动前后各一张。
- 被迁移视图里不再有颜色字面量和未命名的 `px(..)`。

## 5. 进度

| 日期 | 项 | 状态 | 说明 |
| --- | --- | --- | --- |
| 2026-11-03 | 评审 | 完成 | 本文 §1–§3 |
| 2026-11-03 | 1、2 | 完成 | 令牌与角色见 §4；LumaDesk 26 项、LumaKit 全部测试通过 |
| 2026-11-03 | 3 | 完成 | 自绘标题栏；实窗验证：标题栏按钮可点、空白处拖动窗口移动 (+120, +60)、320/414/768 宽度下标题栏与输入卡片可见 |
| 2026-11-03 | 4 | 完成 | 共享 `settings_ui`（page/header/group/row/field/help）；模型服务按 doc 03 改为字母头像 + 状态点列表、Switch 启用、分组卡片与图标行内操作；网络搜索、MCP、外观同一套分组；去掉字符图标 |
| 2026-11-03 | 5 | 完成 | 正文与输入卡片限宽 760 居中；菜单锚定在触发按钮上/下方（编辑回答仍居中弹窗），点外部或 Esc 关闭；字符图标换 Lucide，包外图标由 `lumakit_gpui::assets` 补齐；Ctrl+Enter 改为拦截 Kit `Enter` 动作（原实现实际只换行）；回答 Markdown 改走 `TextView::markdown`（HTML 导入使含行内代码的段落显示为空白条） |
| 2026-11-03 | 6 | 完成 | 根视图 `interface_font()`：Segoe UI + 微软雅黑等中文回退，默认行高 1.5；Kit 主题字体同步 |
| 2026-11-04 | 设计系统 | 完成 | 用户要求把 ThinkRail Native 的界面提炼为 LumaKit 可复用资产：新增 `lumakit-design`（ThinkRail `crates/ui` 的主题目录、角色、Kit 映射、令牌、字体、图标与组件，另加强调色覆盖、密度缩放、中文回退、暖纸主题对与 `components::settings` 设置页组件）。删除 `lumakit_gpui::{style,assets,controls}` 与 LumaDesk `settings_ui.rs`；聊天、外框、模型服务（ThinkRail 服务商页结构）、网络搜索、MCP、外观全部改用新系统。外观偏好升为 schema 2（主题 id + 跟随系统 + 强调色 + 密度），旧暖纸/纯白 × 浅/深/系统组合逐一迁移；新装默认 ThinkRail 深色。ThinkRail 仓库未改动 |

验证（release EXE，隔离数据目录，回环 OpenAI Chat 服务）：经界面新建服务商、线路、密钥，启用、同步并加入模型；对话成功（列表、行内代码、代码块）、上游 500 失败、生成中与停止、更多/工具/历史菜单、375/768/1200/1600 宽度均截图检查。LumaDesk 26 项、LumaKit 全部测试通过。

设计系统迁移验证（2026-11-04，release EXE，隔离数据目录）：新装默认 ThinkRail 深色，启动台、模型服务（服务商栏 + 详情 + 模型区）、网络搜索、MCP、外观、对话页实窗截图检查；预置 schema 1 `appearance.json`（浅色 + 暖纸 + 朱红）启动后外观页选中“暖纸浅色”与朱红，界面为暖纸配色；640 宽窄窗设置导航改为横排。实窗发现并修复：设置导航与服务商行缺少按钮角色，读屏和 UIA 看不到。LumaDesk 28 项、lumakit-design 40 项、LumaKit 全部测试通过。未验证：系统浅/深切换时“跟随系统”的实时切换、高对比度主题在读屏下的表现。

最小化问题：新版本上原序列重跑 10 次、完整验收流程约 60 次点击（每次点击后检查 `IsIconic`）均未复现，仍未定位；人工使用时如再出现请记录前一步操作。

## 6. 模型同步弹窗对标与修复（2026-11-04）

- 参考：ThinkRail Native `providers/models/picker.rs` 的搜索/批量操作行、类型筛选、按 ID 前缀排序分组、440px 独立滚动列表，以及 `core/src/model_catalog.rs` 的类型关键词规则。LumaDesk 保持现有逐项即时保存和失败回滚合同；关闭按钮使用有可访问名称的 Kit Button。
- 修复：新发现的 ID 不再统一回退聊天类型；已有模型的保存类型优先，不覆盖用户设置。类型识别是名称提示，不是接口能力探测；无关键词的 ID（例如 `BAAI/bge-m3`）仍按参考回退聊天，用户可手工改类型。此前错误保存为聊天的模型不会自动重分类。
- 布局：弹层不再跟随添加后的背景列表增长，模型列表独立滚动，标题提供 × 关闭。仍沿用宿主页内弹层，未切换 ThinkRail 的 `window.open_dialog`；不能据此宣称整个模型服务页已完全对齐。
- 职责拆分：`model_settings.rs` 471 行（状态、保存、目录请求），`model_settings/view.rs` 488 行（页面和输入绑定），`model_settings/view/dialogs.rs` 130 行（编辑/删除/同步弹窗），`model_settings/tests.rs` 276 行（单元与回环测试）。没有为压行数合并代码。
- 验证：`scripts\cargo-msvc.bat test model_settings::tests` 12 项通过，宿主 `test` 25 项通过；LumaKit `infer_classifies_catalog_ids_by_name` 定向测试通过。新增回归覆盖新模型类型推断及已有自定义类型/名称保留。
- 实窗：隔离数据目录与本机 HTTP 回环返回 32 个 ID；批量移除/添加、添加后 × 关闭、重新同步、图片筛选四条、815×613 窗口关闭均通过。数据库读回：聊天21、图片4、向量2、重排1、语音识别2、语音合成1、视频1。布局截图来自 release 初版和最终 debug 构建；最终 debug 构建的关闭按钮可被 UIA 查到并激活。
- 未验收：完整读屏软件、键盘全流程、更小窗口及正式分发包；未修改真实用户数据或关闭用户实例。
