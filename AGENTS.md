# 项目约束

开始任何编码任务前，必须读取 `docs/AI工程规范.md`。它约束架构边界、接口、模块、日志、错误、语言、测试和验证。项目局部规则只能扩展它，不得降低安全、错误处理和验证要求。构建、检查与打包命令见 `docs/BUILD.md`。

日志与异常改动必须先读 `docs/日志与异常规范.md`；它规定最终错误所有者、链路终态、持久化失败、取消/关闭、脱敏和 panic 落盘。排查操作见 `docs/日志查看与问题排查指南.md`。

LumaDesk 是 AI agent 工作台。`main` 分支界面用 GPUI（`gpui-kit = "=0.9.0"`）与 `../LumaKit` 的 `lumakit-design`、`lumakit-gpui`；旧 egui 版保留在标签 `legacy-egui`，不再修改。

## 界面令牌与共享组件

- 颜色只取 `LumaTheme::global(cx).roles`，间距、尺寸、圆角、字体样式只取 `lumakit_design::metrics`；不写十六进制颜色，不在本仓库另建令牌或主题。缺的令牌、主题或通用组件加到 `lumakit-design`，并遵守 `../LumaKit/AGENTS.md`。
- 设置页用 `lumakit_design::components::settings`；外观页就是 `lumakit_design::appearance::AppearancePage`，`src/theme.rs` 只负责存入 `settings.db` 和迁移旧格式。
- 新控件要有可访问名称和角色，能用键盘聚焦和操作；读屏软件与 UI 自动化找不到的控件不算完成。

## 设置页：参考 ThinkRail Native

- 设置外框（分组导航 + 详情）与模型服务页（服务商栏 + 详情 + 模型区）以 ThinkRail Native 的设置面板为参考：`E:/workspace/notepad-dev/thinkrail-native/crates/app/src/panels/settings/`（`providers/`、`models/`）。改动前先读对应实现，不自创布局。
- 模型设置的业务规则（独立启用提交、多线路多密钥、检测与同步语义）仍以 `docs/03-模型设置界面与交互改造.md` 为准；ThinkRail 只决定结构和视觉。

## 聊天 UI：参考优先，禁止擅自发挥

这是用户明确要求的长期约束，适用于新增、修改和修复界面。

- AutoNote 是聊天界面的首要参考。先阅读其实际使用的共享组件，再动代码；不能只看宿主包装层，也不能凭印象自行设计。现有总设计：`docs/01-AI对话移植与LumaKit复用设计.md`。
- 参考组件：`E:/workspace/notepad-dev/Aether/packages/ui/src/`（即 AutoNote 所用的 `@ouraihub/aether-ui`）的 `chat/ChatPanel.svelte`、`chat/MessageItem.svelte`、`chat/ToolCallItem.svelte` 和 `providers/ModelSelect.svelte`。原宿主入口 `AutoNote/packages/app/src/features/ai/chat/ChatPanel.svelte` 已不在本机检出中（2026-11-04 核对，AutoNote 只剩 `desktop/src-tauri`）；拿到 AutoNote 前端后，先核对它锁定的 aether-ui 版本与上述目录是否一致。不能把失效路径当成自由设计的理由。
- 移植信息层级、布局、控件归属和交互，不另造一套。外观只用 `lumakit-design` 的主题与令牌；参考行为不等于可以复制许可不明的代码或资源。
- 输入区保持文字区和紧凑工具栏。生图是工具开关/可移除标签，不是独占聊天模式；模型设置不得变成占满整行的双下拉墙。
- 「快捷面板 / 自定义工具栏」用于开关底栏工具的显示，不是提示词插入器。切换显示不能执行工具或改变当前请求设置；选择须持久化。未固定工具仍可从添加菜单执行，不得放置没有消费者的假开关。
- 回答按来源、工具状态、图片、正文、操作分层并留出间距；生成图片内联可见、点击打开。次要动作进入更多菜单。
- 用量明细、工具参数、请求 ID 和原始错误属于按需展开的诊断信息，默认折叠；失败默认显示简明中文说明，不把英文 JSON、逐轮统计和状态堆成调试面板。不得为简洁隐藏失败事实或删除诊断能力。
- 用户没有明确要求时，不增加常驻控件，不改交互结构，不重新选主题。原生技术差异只允许最小必要适配；若会实质偏离参考，先说明差异并取得用户确认。
- UI 验收必须运行实际程序并对照参考检查成功、失败、生成中和窄窗；检查正文/工具栏无遮挡、焦点和禁用状态有效。至少覆盖 320/375/414/768 逻辑宽度的布局检查。编译或测试通过不能代替实窗截图与交互证据；未观察到的结果不能宣称通过。

修改共享聊天组件时，同时遵守 `../LumaKit/AGENTS.md`。不要用局部产品补丁掩盖共享组件的布局问题。
