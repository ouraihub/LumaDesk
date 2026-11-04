# GPUI Kit 验证详细设计

## 1. 版本和文件边界

`gpui-kit = "=0.9.0"`，默认 component/assets。使用 gpui_kit::*，需要派生宏时显式 alias `extern crate gpui_kit as gpui`。不从滚动文档猜测发行版未提供的 API；本机包源码位于 Cargo registry。共享源与宿主锁文件统一解析同一组件类型。

- `LumaKit/crates/lumakit-gpui/{Cargo.toml,src/lib.rs,src/chat.rs}`：共享聊天；只按实际复杂度拆 Markdown/媒体辅助。
- `LumaDesk/src/main.rs`：GPUI DesktopApp、导航、启动失败视图、异步收尾、关闭保护。
- `LumaDesk/src/chat.rs`：中立会话与 GPUI 面板桥接，动作分发。
- `model_settings.rs`、`model_editor.rs`：保留已有业务函数，替换 egui UI。
- `search_settings.rs`、`mcp_settings.rs`、`theme.rs`：替换渲染并保持原存储合同。
- `settings_store.rs`：默认不改，仍是产品 SQLite 唯一实现。
- scripts / README：切换验证产物目录和运行方式，保留 MSVC 注入。

## 2. 模型设置实体合同

`ModelSettings::create(root, window, &mut App) -> Result<Entity<Self>, String>`，先完成可失败的加载，再通过 cx.new 初始化输入实体；实现 Render。不能在 cx.new 内用 expect 把数据加载失败变成 panic。

宿主继续使用 `available_models()`、`image_models()`、`model_settings(model)`、`connection(model)`、`image_connection(model)`。`clear_secrets()` 清除页面密钥及取消检测；`poll()` 用于页面隐藏时收尾，可返回是否改变状态。模型选项只反映成功保存目录，不能直接反映未提交草稿。

表单字段以稳定 ID 对应 InputState；切换编辑对象时同步一次，不在每帧覆盖用户输入。订阅 Change 更新草稿，Blur 或显式保存沿用原提交时机。密钥输入采用 password 控件，明文显示/复制为显式动作，删除确认。离开设置时清理输入实体中的敏感内容及原 draft，不只清空字符串副本。

保留：服务商筛选/分组、线路管理、协议切换、模型发现与选择同步、模型能力/推理方言/支持状态/上下文与输出/价格分档编辑，检测不替换已保存模型，同步保留手动配置。

## 3. 共享聊天接口与所有权

推荐具体接口：

- `ChatPanel::new(window, cx) -> Self`。
- `ChatPanel::render_session(&mut self, session: &ChatSession, options: &[ModelOption], image_options: &[ModelOption], model_settings: Option<&lumakit_ai::model_config::ModelSettings>, window, cx) -> AnyElement`。
- `EventEmitter<ChatAction>`，控件事件只发动作；不持有服务商密钥、连接或产品数据库。

Panel 实体的 render 本身可由宿主绘制方法提供当前会话引用；若 GPUI Entity 借用约束要求调整，采用一个具体的宿主组合方式，不复制完整 ChatSession 历史快照，不另造泛型后端。

临时 UI 状态：输入编辑实体、历史搜索/确认/展开状态、回答编辑草稿、模型搜索、滚动跟随、Markdown/媒体缓存。对话切换重置应重置的临时状态；业务草稿仍保存在 ChatSession。

头部显示标题与历史/设置。正文按问题及候选层级渲染，保留来源、工具结果、推理、图片、正文、操作、折叠诊断/用量。底部编辑器和工具栏高度可变但不挤掉发送按钮；窄窗允许换行。

## 4. 聊天动作与后台归约

宿主保留现有 ChatAction match 的业务行为：新建、选择/删除、草稿、模型、发送/停止/重试/换模型重试、恢复保存、候选编辑/选择/分支/复制、附件/图片工具及工具栏偏好。无消费者的产品回调不显示为可用操作。

后台 wake 使用线程安全通知/标记。前台长期任务接收唤醒，调用 session.poll()，再通知绘制。文件选择工作线程也通过同一路径完成。任务生命周期由宿主持有；禁用忙碌期间重复提交，窗口关闭调用 can_close，不能绕过保存失败保护。模型检测与搜索完成也不能依赖当前页正在 render。

如果目标版本事件循环桥接需要定时检查，必须仅在存在后台工作时启用或通过标记避免无变化重绘，并记录间隔与限制；不得用无限高速轮询掩盖唤醒问题。

## 5. Markdown、图片和系统边界

复用 pulldown-cmark 的解析能力，维持禁任意图片加载与受限链接策略；GPUI TextView 的解析/点击接口需从实际源码核对。不可把服务端 Markdown 直接交给默认网络加载器。

本地预览仅加载 ChatSession 登记媒体，沿用输入/解码/尺寸/缓存上限。使用 image 已有解码链路，适配 GPUI 图片类型；失败显示错误，不能默默省略已生成图片。缓存绑定媒体/内容版本，避免每次流式 token 都解码图片。

文件选择使用后台系统对话框；打开图片只接受 session.media_path 解析结果。复制用 GPUI 系统剪贴板；用户明确触发后才打开链接。

## 6. 其它设置实体合同

SearchSettings::create(root, window, &mut App) 返回 Result<Entity<Self>, String>，Render，clear_secrets(window,cx)，poll()->bool，is_busy()->bool。沿用现有 Stored/Config/Draft、save/begin/request 函数与结构化 HTTP 错误；保存有效草稿后才检测。服务商切换、地址、增删改显隐复制密钥、默认搜索/读取、黑名单/结果数/压缩与结果列表保持可操作。

McpSettings::create(root, window, &mut App) 返回 Result<Entity<Self>, String>，实现 Render。仍只管理服务器 URL、enabled、trusted；错误必须显示，不新增虚假工具调用。

Appearance::load(root)，apply(window, cx)，render controls 或独立实体均可。serde Preferences 枚举名不改，保留 NativeLight 的浅蓝/舒适初始语义，映射为 Kit 样式。保留强调色/背景/密度，不用更换主题库重置用户偏好。

强调色必须同时映射主操作、焦点/光标以及普通/ghost 按钮、列表、侧栏、页签和表格的选中态；只更新 primary 会让当前导航仍显示固定中性色。实色色块显式设置背景，避免 Kit Custom 变体的默认透明混合。2026-10-26 实窗检查蓝→绿即时变化、紫色保存重启保留，主题测试 6 项通过。

## 7. 启动、布局和发布

gpui_kit::application 初始化 assets，run 中先 init，再 open_window 创建 DesktopApp。中文字体优先系统字体。应用根 Root 由 Kit 负责，不能嵌套多个 Root。窗口不用系统标题栏：`TitleBar::window_options()` 打开窗口，页签栏是 Kit `TitleBar`，可点击部分 `occlude()`，详见 10 §4。

实际发行版 gpui-pre 0.3.11 默认 windows-manifest feature 已嵌 Common-Controls v6 / PerMonitorV2 清单，无需复制 Navop 清单。Windows Debug 聊天布局实测超出默认主线程栈，宿主 build.rs 设置 `/STACK:8000000`。直接返回共享面板元素时，以 `(chat-panel, entity_id)` 隔离根 ElementId，避免与宿主的设置按钮产生重复无障碍节点。

默认验证数据路径为可执行文件旁的 `data-gpui`；显式环境变量优先。现有 support::data_root 的 slug 不隔离目录，因此不能只换 slug。启动失败显示原配置未覆盖与真实原因。数据锁及日志句柄持有到正常关闭。

宽屏保留原 1200×800 启动尺寸；窄窗验收允许缩至组件约束要求的逻辑宽度。切换设置清除敏感展示；关闭聊天页签保留原取消行为。

输出 `dist/LumaDesk-GPUI/LumaDesk-GPUI.exe` 及独立 manifest/ZIP。保持数据目录不删除不打包；许可证收集改为实际 GPUI 资源与依赖声明，不继续宣称只包含旧字体许可。Windows manifest 以 gpui-pre 实际构建行为核对，不照抄 Navop build.rs。

## 8. 验证矩阵

- 构建：宿主 + 共享组件，锁文件一致，最终一次集成检查后修复实际错误。
- 核心：保留已有 AI/存储行为测试；不把 egui 几何断言机械改为源码/字符串断言。
- 设置：新增服务商/线路/密钥/手工模型，保存启用，检测/同步，不泄露明文；网络读取回环页面，配置重启保留；MCP URL 校验和信任持久化。
- 聊天：回环服务真实 SSE、取消、错误响应、重试、历史重启、编辑/候选、工具配置、附件安全边界。
- 视觉：实际 EXE 观察成功/失败/生成中与 320/375/414/768 布局，底栏可达，弹层焦点和中文文本可见。
- 发布：移出开发目录启动，独立数据、正常关闭；不覆盖旧正式产物。

不把回环服务器当成产品 fake fallback；它只在验收时存在，应用仍使用同一真实 HTTP 请求链路。未观察到的供应商、IME 候选/屏幕阅读器结果明确记录。
