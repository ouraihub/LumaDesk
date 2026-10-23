# LumaDesk

独立的纯 Rust 桌面宿主 App，使用 eframe/egui 绘制原生窗口，并接入 `LumaKit` 的 `lumakit-ui` 与 `lumakit-ai`。

## 运行

```bat
scripts\build-app.bat
scripts\run.bat
```

正式打包先运行框架与宿主检查，再构建 Windows x64 发布程序。产物分工：

- `dist/LumaDesk/LumaDesk.exe`：日常直接运行，`run.bat` 只启动这里，不临时编译。
- `dist/LumaDesk/data/`：运行时数据，重新打包只替换 EXE，不复制或删除这个目录。
- `dist/installer/<版本>-<构建标识>/LumaDesk_<版本>_x64-portable.zip`：便携分发包，只含全新的 EXE，不包含本机密钥、聊天记录或缓存；当前不生成安装器。
- `dist/release.json`：本次发布版本、精确包路径及 SHA-256；执行 `powershell -NoProfile -File scripts/release-artifacts.ps1 -Verify` 可复核 EXE、ZIP 及包内 EXE 的一致性。

`scripts/build.bat` 仅供快速编译，输出仍在 Cargo `target`；正式查看或分发使用 `build-app.bat` 的 `dist` 产物。旧 `target/release/data` 不自动搬迁，避免覆盖现有资料；迁移时先关闭程序，再自行搬移完整 `data` 目录。也可保留 `LUMADESK_DATA_DIR` 显式指定数据路径。

启动台提供对话、工作、笔记、知识库、翻译入口；对话与「设置 → 模型服务 / 网络搜索」可用，其它应用业务未接入。

LumaKit 仍是独立框架工程；本项目通过路径依赖使用它，不属于 LumaKit 的 `examples`。

## 设计与移植规划

[移植方案](docs/01-AI对话移植与LumaKit复用设计.md)与[详细设计及实施结果](docs/02-AI对话移植详细设计.md)：共享会话核心、原生组件、数据兼容、布局与验收。文本聊天已采用共享组件；模型/搜索设置的全面组件化另列后续阶段。

项目级 [egui 组件使用 skill](.claude/skills/egui-components/SKILL.md) 约束版本确认、组件语义、布局、键盘/可访问性和运行验证；包含独立0.37.0预研工程。它不表示生产依赖已经升级，也不代表共享 Switch 已实现。支持项目 skills 的工具在打开本项目时可发现；其它工具需显式读取该文件。

## 对话

在模型服务中填写线路、密钥和模型并启用服务商，再从启动台打开对话，选择模型并新建会话。Enter 换行，Ctrl+Enter 或发送按钮提交；支持流式回复、停止、重试最后问题、重新生成、复制及会话切换/删除。

复用 LumaKit 的会话核心和三协议客户端；后续请求只携带每个问题末次尝试为完整成功的问答，失败和取消的部分内容保留展示，但不作为上下文。重试保留历史尝试，不重复加入上下文；普通重试使用源模型，“换模型重答”为显式选择。回答按 Markdown 显示，支持标题、列表、代码与表格，复制保留源文本；不自动加载图片或打开非 HTTP(S) 链接。

记录与草稿保存在同一数据目录的 `chat-history.json`，重启读回；中断状态会标记失败。发送前保存问题、结束后保存结果；保存失败禁止继续修改或联网，允许重试保存并阻止正常窗口关闭。请求总时限 120 秒，上下文/回答上限各 1 MiB；不会自动截断历史。对话尚未接入图片、工具调用或联网搜索。

聊天头部、消息滚动区和底部输入区独立分配；历史在弹窗中切换，支持展开输入与回到底部。通用界面来自 `lumakit-egui::ChatPanel`，产品只提供已保存模型选项和连接解析。数据目录使用内核文件锁，同一目录不能同时开启多个写入实例；目录不可写时明确报错，不静默新建其它位置的数据。

对话协议检查：在 LumaKit 工程执行 `cargo test -p lumakit-ai --release`；宿主行为检查使用下方 `scripts\check.bat`。

## 设置与验证

- 右上角或左下角进入设置；浅色/深色按钮切换当前窗口主题。
- 模型服务支持多服务商、多线路、加密多密钥、三种协议、真实模型检测、选择同步和手动模型能力设置。检测只统计，不会替换模型；同步保留手动模型和已有能力。
- 内置七牛云、硅基流动、Fenno 三个服务商及获取密钥链接，默认关闭、不带密钥或模型。首次使用或旧配置升级时补入一次；同名服务商保留原配置，已初始化后删除不会在重启时重新添加。
- 网络搜索支持 Tavily、Brave、Exa；网页读取支持直接获取和 Jina Reader。只有显式点击才联网；检测先保存有效草稿。黑名单、结果数和 Unicode 截断在高级设置中配置。
- 密钥使用 Windows 当前用户系统加密；显示/复制为显式操作，离开设置清除已显示明文并取消请求。系统剪贴板由用户管理。401/403/429 才有限轮换同服务商密钥，不跨服务商重试。
- 配置保存在 `lumakit_support::data_root("luma-desk")`，也可用 `LUMADESK_DATA_DIR` 指定目录。新文件为 `model-settings.json`、`search-settings.json`，不覆盖旧版文件；损坏或不支持版本会显示错误，不静默重置。
- 当前未接入模型原生搜索路由，选项禁用；本页策略不宣称控制其它应用。直接读取只处理 UTF-8 文本/HTML，不执行脚本、不跟随重定向；复杂网页可选择 Jina。
- 搜索请求在宿主处理结构化 HTTP 状态，复用 LumaKit 请求合同和结果处理；没有通过错误字符串推测是否换钥。

运行 `scripts\check.bat` 执行框架和宿主行为检查；正式打包使用 `scripts\build-app.bat`，快速编译使用 `scripts\build.bat`。均复用便携 MSVC 环境，可由 `PORTABLE_MSVC_ROOT` 指定。
