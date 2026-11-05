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

## 7. AutoNote 自定义工具栏对标（2026-11-04）

- 参考来源更新为 `E:/workspace/notepad-dev/dev/AutoNote`。宿主 `packages/app/src/features/ai/chat/ChatPanel.svelte` 从 `ai.chat.toolbar` 加载并保存顺序；依赖锁定 `@ouraihub/aether-ui 0.1.1`，安装包 `src/chat/ChatPanelToolbar.svelte` 与本机 Aether 对应组件一致，AutoNote 补丁未改该组件。参考包声明 LGPL-3.0-only，本次按可观察交互用 GPUI 原生控件独立实现，未复制 Svelte/CSS 源码或资源。
- 替换原 460px 文字选项面板：＋菜单与自定义页共用 256px 锚定浮层，显示图标、手柄和小号开关；恢复默认在默认值时禁用；Esc/点击外部关闭，无额外“关闭”页脚。仅保留有真实消费者的新对话、网络搜索、上传附件、生成图片，未添加知识库/提示词/MCP 等空操作。
- 显示开关和排序只发出 `SetToolbar`，不创建会话、不修改搜索/生图设置。固定项按用户顺序展示；未固定项排在自定义列表后面并仍可通过＋执行。拖放落下时保存，上下键也可排序；核心去重保留首次出现顺序，失败恢复原顺序。
- 职责拆分：`lumakit-gpui/src/chat.rs` 保留聊天主体，`chat/overlays.rs` 负责菜单/编辑层，`chat/toolbar.rs` 负责工具列表和自定义；三个生产文件均小于 500 行。
- 实窗验证（debug、隔离目录）：开关即时改变底栏且自定义页保持打开；上下键排序、拖动附件至第一项、正常关闭应用并重启后排序保留；恢复默认、Esc 和点击外部关闭通过。320/375/414/768 宽度均检查截图，浮层无横向裁切。
- 测试：既有工具栏持久化回归覆盖顺序、去重、跨会话/重启、未知项、旧默认及保存失败回滚；lumakit-design 47 项、宿主 25 项通过。未测试完整读屏软件和生成中实窗；本次未提交或更新正式分发包。

## 8. AutoNote 网络搜索页对标（2026-11-04）

- 依据：`dev/AutoNote/packages/app/src/features/ai/settings/SearchSection.svelte` 实际使用已安装 `aether-ui 0.1.1` 的 `SearchSettings.svelte`、`SearchProviderCard.svelte`、`SearchAdvancedTools.svelte` 与 `settings-flow.ts`。读取了供应商请求/响应合同；本地使用 GPUI/reqwest 独立实现，不复制 Svelte/CSS 或上游资源。
- 界面：网页搜索、读取网页两张卡片；左侧标题/说明/官网入口、右侧服务商选择；密钥输入旁为管理与检测图标；卡片展示脱敏摘要，密钥增删改/显示/复制放入独立模态窗口；基础地址带复制；保存位于卡片左下。高级设置折叠，数值/压缩横向布局，手动查询与读取另卡展示。窄窗头部主动换行。
- 供应商：搜索支持智谱、Tavily、SearXNG、Exa、Exa MCP、博查、Querit、Brave、Parallel、Jina、Firecrawl；读取支持 Jina、Querit、Firecrawl、直接获取。SearXNG 支持可选 Basic 用户名和加密密码；Exa MCP 支持 JSON-RPC 与 SSE。不是只增加菜单名称。
- 保存：切换默认项独立提交，不保存地址/密钥草稿；卡片仅保存自身配置；密钥管理仅提交密钥；高级设置独立保存。检测先保存当前卡片，再使用固定查询 `web search` 或读取 `https://example.com`；手动工具只使用已保存配置，不隐式保存草稿。写入失败保留草稿与原密文，检测失败不撤销已成功保存的配置。
- 存储：沿用 settings.db 表结构，Basic 用户名单独存入已有 prefs 的 `search.usernames`；旧五项目录扩展新增配置时保持原默认项、稳定顺序和密文。搜索/读取的 Jina、Querit、Firecrawl 分别保存，不共享凭据。现有 URL 限制、响应大小限制、取消及仅 401/403/429 密钥轮换保持不变。
- 模块：`search_settings.rs` 负责状态与任务协调；catalog/stored/persistence/inputs、network/policy/parsing、view/providers/keys/advanced 和 tests 分职责，所有生产文件在 500 行以内。
- 验证：宿主 33 项测试通过（含 10 项配置/存储/请求回归、3 项供应商矩阵）；11 搜索与 4 读取使用本机 HTTP 回环验证请求、认证和结果；MCP 覆盖多结果/段落、JSON/SSE、错误脱敏；图标资源定向测试通过。debug 实窗检查 11/4 选择菜单、密钥添加/修改/显示/删除与取消、关闭/Esc、基础地址复制、检测成功与 HTTP 401 失败、320/375/414/768 宽度。
- 差异与边界：仍沿用宿主主题与设置导航；默认项保留原 Tavily/Jina，不强制改成上游 Exa MCP；密钥删除保留确认步骤。网络任务仍为单请求互斥，不宣称支持同时检测多卡。没有复制上游“优先模型原生工具”开关：当前搜索设置没有接入该聊天路由消费者。保留不跟随重定向等本地安全策略。未使用真实供应商凭据验证公网服务、未测试完整读屏软件；本轮未提交或发布新分发包。

## 9. 生图入口可达性与实窗验证（2026-11-04）

- 结论：底层已有真实 `generate_image` 工具与 OpenAI 兼容 `/images/generations` 请求，不是空按钮。原入口在未选图片模型时禁用，提示却只有“生图”，且图片模型配置入口隐藏在＋菜单，导致用户无法判断前置条件。
- 对照 AutoNote 已安装的 `ChatPanelToolbar.svelte` 与宿主 `AiSection.svelte`：生图依赖绘图模型和聊天模型工具能力；缺配置/不支持工具需明确提示。LumaDesk 保留每个对话的图片模型快照，不改为 AutoNote 的全局绘图角色；最小原生适配为点击生图直接打开图片模型选择器，没有可用图片模型时提供模型设置入口。无有效聊天模型或明确不支持工具时，不允许从选择器启用生图。
- 工具栏提示现在区分缺聊天模型、缺图片模型和不支持工具；＋中的图片模型选择入口始终可达。固定生图图标时不再重复显示可移除生图标签。选择图片模型后仍通过聊天工具调用，不把生图变成独占聊天模式。
- 实窗发现并修复预览越过容器压到正文：图片和容器都有明确尺寸及裁剪；图片使用 Kit Button，具名且提供键盘/UIA 激活入口。
- 验证：现有七项 `painting_` 回环测试通过，涵盖真实图片请求/历史/重试快照、工具不支持、错误与不安全 URL、调用上限、取消、Responses 后续对话。隔离模型目录与本地 HTTP 上游实窗跑通：固定生图图标→未配置时选择模型→聊天发出 generate_image→图片接口收到请求→有效 PNG 内联预览→正常退出重启后图片仍显示；UIA 读到 240×220 预览且具备 InvokePattern。空目录场景可进入模型设置，没有伪造成功。
- 边界：没有使用用户真实密钥或公网生图服务；本次未增加图片编辑接口，未改现有 4 MiB/HTTPS 下载等安全限制。此前网络搜索变更与本节代码尚未提交、未重新打包。

## 10. ThinkRail 模型与强度组合选择器（2026-11-04）

- 参考是 `E:/workspace/notepad-dev/thinkrail/apps/web/src/chat/ModelEffortPicker.tsx`、`ModelEffortLabel.tsx`、`modelPicker.ts`、`pickerGroups.ts`、`useModelPreferences.ts` 和 `ChatView.tsx`，不是 ThinkRail Native 的旧双选择器。GPUI 使用宿主令牌独立实现，无浏览器依赖。
- 一个紧凑入口显示供应商首字母、模型名、强度条/文字、已保存密钥标记及箭头。360px 锚定浮层包含搜索、固定、最近使用、可折叠供应商分组、两行模型项；模型、端点、服务商和协议组成完整身份。价格/上下文只显示真实配置，没有的认证类型、订阅或价格不编造。
- 点击新模型立即应用并保持打开，悬停不切换；再次选当前模型、Esc、外部点击关闭。支持模糊匹配及 `beta high` 后回车同时选择模型和支持的强度；上下键导航模型，键盘高亮可滚动到可见项。列表独立滚动，不带旧菜单的关闭页脚。
- 强度位于模型列表下面，跟随当前模型；离散滑杆使用原生 Kit 行为与冷暖轨道、携带档位文字的滑块，支持拖动/点击与左右键。提供档位点击、默认强度标记、恢复默认和超高/最大开销提示。没有推理配置的模型不显示强度区。Auto 是单独的自动控制，不当作比 Max 更高的档位；本地继承默认和明确关闭保持区别。
- 收藏/最近/默认组合写入 chat.db 现有 composer 元数据，旧记录用默认空偏好；最近唯一完整身份最多十项。模型/强度/最近记录原子提交，失败回滚；收藏及默认变更不清草稿或执行请求。显式新对话用默认组合，首次输入物化对话保留待用选择，已有对话切换恢复模型；不会改变已发请求和重试来源。
- 打开浮层仅重读本地已保存目录；强制刷新使用有界 HTTP 请求读取每条启用线路的模型目录，成功只新增，保留已有类型和手工配置；失败线路不删除模型、不把错误伪装成刷新成功。刷新状态与部分失败在浮层显示。独立 egui 示例宿主只支持其环境配置的本地重读，明确提示没有 HTTP 发现能力。
- 验证：核心模型选择器 12 项回归通过，覆盖重启、完整身份、组合/收藏/默认写入失败回滚、不兼容强度、首次输入与新对话区别、草稿/附件与已发快照。LumaKit workspace 全套与宿主35项测试通过；宿主新增本地重载/真实HTTP刷新测试，覆盖部分失败和元数据保留。Clippy 执行完成但仍有既有模型设置/搜索测试等告警，未宣称零告警。
- 实窗（debug、隔离目录、本地可控模型目录）：选模型不关闭、左右键调档、拖动至超高、档位恢复、固定、最近、默认、新对话应用、`beta high`+Enter、无推理模型、真实刷新及失败线路提示、320/375/414/768宽度、退出重启后偏好恢复、外部点击/Esc/当前模型关闭均通过。
- 原生适配边界：供应商用首字母标记而非上游品牌SVG，价格/认证仅展示本地实际配置，默认强度标记优先保存的组合否则模型配置首项。未验收触屏与完整读屏软件。前面的搜索、生图与本节修改均尚未提交，未更新dist分发包；真实用户数据未改动。

## 11. OMP 用量统计与模型元数据（2026-11-04）

- 在现有 Rust 请求/SQLite 链路移植统计合同，不引入 Bun 或 Web 仪表盘。逐请求保存稳定 ID、开始时间、耗时、实际模型、输入/输出/缓存/推理和价格来源；输入包含缓存，推理属于输出子集，不重复相加。
- chat.db schema 5 独立账本按请求 ID 去重，复制/分支不增加真实消耗，删除聊天保留已记录账本；无用量/无价格保持未知，不补造取消或失败请求的账单。历史记录无时间/身份时标覆盖限制，无法追溯过去历史副本，时间筛选排除无日期请求。
- 精确模型 ID/规范提供方前缀从内置 OMP MIT 目录补全上下文、可表示的参考价格及协议兼容推理；手配优先，未知/歧义/不支持费率仍未知。GPT-6.1-sol 支持来源 low/medium/high/xhigh/max，不再因未手配 reasoning 隐藏已有滑杆；显式停用和恢复自动可保存。参考价格是估值，不是中转账单。
- 用量入口保留弹层而不新增常驻状态栏：提示词上下文估算、当前对话累计、折叠的上一回答；设置增加全局统计，按全部/7天/30天与模型/服务商/UTC天分组。上下文锚点需模型和实际请求前缀匹配，仅追加本地估算，不替代准入预算。
- 验证：LumaKit workspace 161 项、宿主36项通过；真实回环请求确认选择 max 后发送 reasoning_effort=max，账本保存100输入/20输出/25缓存读取/5推理及 reference 金额0.0003525 USD。独立egui实窗用量弹层及320/375/414/768宽度已观察；GPUI新构建实窗已观察滑杆、用量弹层和全局范围/分组控件。
- 纠正验收流程：最初误启动旧 target 产物、文件占用导致替换失败，不属于新界面缺失；后续只对明确路径的成功构建验收。发布脚本修复中文路径下 Cargo metadata 的 UTF-8 解码，并将 OMP MIT 声明纳入分发许可。

## 12. Cherry 模型编辑分组（2026-11-04）

- 读取 Cherry 本机 `ModelDrawer/EditModelDrawer.tsx`、ModelBasicFields、ModelPurposeFields、ModelClassificationControls、ModelContextWindowFields、ModelPricingFields 与 ProviderSettingsDrawer；按信息层级独立用 GPUI 实现，没有复制 React/CSS 或资源。
- 紧凑单列、ID内联复制、名称/分组、更多设置下的类型/能力、输入模态、容量限制、流式及价格分组；标题和保存/取消固定，表单独立滚动。用途最初按参考使用卡片，用户后续要求改为紧凑下拉，说明仅显示选中项。保留现有原子保存/取消，不照搬自动保存。
- 最大输入 Token 已接入请求前预算与正整数/窗口关系校验；手工覆盖清空恢复自动。推理显示自动来源而不把未修改推断值保存为覆盖；无运行消费者的图像编辑、音频/视频明确不可用。
- 实窗已验证名称修改取消不落盘、保存名称/输入上限生效而推断reasoning/context/pricing仍未写成覆盖；预设200K/400K及自动清空生效。320/375/414/768宽度下固定保存/取消均在窗口内，内容滚动后页脚不动。完整读屏软件及公网供应商未验收。

### 推理能力与使用参数分离

Cherry `ModelSpeedControl.tsx` 从模型能力派生可用档位，选择持久化到助手/会话；`shared/ai/reasoning.ts` 单独加入 default，`reasoningSerializers.ts` 按服务商 wire profile 解释 default/none/auto/具体档位。ThinkRail `ModelEffortPicker.tsx` 由 onSelectLevel 处理聊天使用参数，并非模型编辑表单。

按用户纠正，模型页不再编辑推理档位、默认强度、协议方言或预算，只保留能力分类。聊天里的“默认”是独立状态，不自动选支持列表第一档 low；未显式指定则不发强度参数，具体档位由使用时选择。已有保存能力数据保留，不将缺配置伪造成关闭。用途继续使用用户已确认的紧凑下拉，其他编辑分组按最新 Cherry 截图收敛。

最终修正实窗：全新隔离配置下 `gpt-6.1-sol` 滑杆读到“默认”，首次回环请求不含 reasoning/reasoning_effort；点击 max 后下一次真实 HTTP 请求含 reasoning_effort=max；点击默认恢复不指定档位。编辑页仅显示推理能力，不再有推理参数输入或方言/档位选择表单。定向后端99项、宿主35项通过；之前完整 workspace 验证仍记录于上文。

发布完成：`dist/LumaDesk-GPUI/LumaDesk-GPUI.exe` 与便携包 `0.1.0-6160c615e7864d01b319592e03662096` 已生成，`release-artifacts.ps1 -Verify` 核对 EXE/ZIP/包内EXE 哈希通过；实际启动该 dist 路径，模型编辑入口、用途下拉及无推理参数表单已复核。只使用隔离目录与本地回环服务，未修改真实用户数据。

## 13. 模型能力图标与统一元数据（2026-11-04）

- Cherry `ModelTagsWithLabel.tsx` → `ModelTag.tsx` 按 capabilities/inputModalities 选择标签；AutoNote/Aether `EndpointModels.svelte` 使用 canVision、reasoning、supportsTools，筛选与图标共用谓词。原设置列表仅读原始手工值且渲染文字标签，聊天选择器无能力图标，导致自动识别未贯通。
- 新 `ModelEntry::effective_settings(protocol)` 在设置列表、能力筛选、聊天选项和请求解析共用；精确参考 input:image 补全视觉，明确工具声明补全工具。GPT-6 Sol/6.1 Sol/6 Astra 的源目录均声明 image 输入、freeform patch 与 computer-use 工具支持；未知模型不点亮。显式 supports_tools=false、vision_override=false 优先，旧true配置保留，推断不写回用户覆盖。
- 共享 `lumakit_gpui::model_capability_badges` 渲染眼睛/大脑/扳手/联网图标、主题色背景、工具提示和可访问名称；设置列表与聊天模型选择器复用。编辑页各按钮补 Type/Image/Boxes/ArrowUpDown/Brain/Wrench/Eye/Ear/Video 图标，资源由 UiAssets 注册，分组标题严格为“模型类型、模型能力、输入模态”。
- 验证：目录元数据8项、运行目录解析3项、图标资源1项及宿主35项通过。隔离配置下三种GPT模型各显示视觉/推理/工具三图标，unknown-model无图标；手工关闭GPT-6.1-sol视觉并保存后只该行眼睛消失，聊天选择器同步只显示剩余两项。320/375/414/768宽度能力图标均在窗口内；未使用真实凭据。
- 已发布 `dist/LumaDesk-GPUI/LumaDesk-GPUI.exe`，便携包构建标识 `0.1.0-099ab4b0faf043399f5ea09cb01eacb3`；发布哈希校验通过，实窗使用的确为该dist路径。

## 14. Cherry 可编辑能力与 ThinkRail 力度刻度

- 纠正只读能力误解：Cherry 的 ModelClassificationControls 通过按钮切换 capabilities/inputModalities 集合，能力声明可编辑，不等同于聊天请求强度。本地推理、工具、视觉、音频、视频均可点击切换并原子保存/取消；推理能力声明与可发送的协议/档位独立，未知模型声明推理不会凭空生成 wire 参数。明确关闭优先于目录，列表/选择器图标同步反映声明。
- 实窗点击五个能力控件均可用；取消后原数据未改，保存后数据库读回推理/工具/视觉false、音频/视频true，随后重新启用已知能力，聊天选择器同步显示。音频/视频仅为模型元数据，目前未接入上传链路，界面明确说明。
- ThinkRail `ModelEffortPicker` 只绘制宿主 `thinkingLevels`，由固定依赖 pi-ai1.5.0 的 getSupportedThinkingLevels 过滤。其官方GPT-6.1-sol thinkingLevelMap中off/minimal为null，故不能把其他中转截图的关闭/最低强塞到官方模型。对齐中文关闭/最低/低/中/高/极高/最高、分档图标及提示，标题“力度 · 模型名”；默认恢复移出刻度行，具体支持档位保持模型真实合同。
- 宿主36项（含能力覆盖与序列化回归）通过，实窗确认能力点击、保存/取消及力度刻度/恢复默认分离。发布时不关闭用户实例、不读改用户数据；Windows映射旧EXE时用无备份原子替换，避免删除占用旧备份导致发布清单未提交。

### ThinkRail 档位核对补充

- ThinkRail 直接传输 host 的 `thinkingLevels`，不是前端自行列出所有档位；`toWireModel` 调用 `getSupportedThinkingLevels`，官方 Pi 规则按 `thinkingLevelMap` 过滤。截图对应的实际有效列表是 `off / minimal / low / medium / high`，显示为“关闭 / 最低 / 低 / 中 / 高”。
- 本地 GPT-6.1 Sol 的 OMP 公开目录仍保留 xhigh/max 参考档位，但当前 ThinkRail 对标线路按上述五档展示；请求参数映射保持 off/minimal/low/medium/high 原值，默认仍不发送显式档位。UI 测试必须核对可访问标签和实际列表，不只看滑杆视觉。
