# 构建

工具链由 `rust-toolchain.toml` 固定为 Rust 1.99.0（含 `rustfmt`、`clippy`），edition 2024，依赖锁定在 `Cargo.lock`。LumaDesk 通过路径依赖使用相邻的 `../LumaKit`（`lumakit-ai`、`lumakit-design`、`lumakit-gpui`、`lumakit-support`、`lumakit-observability`），两个仓库须并排检出。

`main` 分支的界面基于 crates.io `gpui-kit = "=0.9.0"`（默认后端 gpui-pre 0.3.11，不启用 gpui-fast），LumaKit 的 `lumakit-design`、`lumakit-gpui` 锁定同一版本。

设计系统是 LumaKit 的 `lumakit-design`（提炼自 ThinkRail Native `crates/ui`）：主题目录（含暖纸 `warm-light`/`warm-dark`）、颜色角色与 Kit 映射、尺寸/字体令牌、内置字体（Geist、JetBrains Mono、Orbitron，OFL 许可在 `assets/fonts`）、图标源 `UiAssets`、设置页组件 `components::settings`，以及外观偏好与外观页 `appearance`（`src/theme.rs` 只负责存进 `settings.db` 和迁移 schema 1）。视图只用 `LumaTheme::global(cx).roles` 和 `metrics::*`，不写十六进制颜色。

## MSVC

Windows 上统一用便携 MSVC，默认 `D:\Applications\msvc-portable\msvc`，可用 `PORTABLE_MSVC_ROOT` 指向含 `setup_x64.bat` 的目录。与 ThinkRail Native 共用同一份工具链；尚未安装时可用 ThinkRail Native 的 `scripts\msvc\setup-msvc.bat` 下载。

所有 cargo 命令经 `scripts\cargo-msvc.bat` 执行：PATH 上已有 `cl.exe` 时直接运行，否则先注入便携 MSVC。未显式传 `--manifest-path` 时自动指向本项目 `Cargo.toml`，所以检查 LumaKit 时要显式传入：

```powershell
scripts\cargo-msvc.bat build                                   # 开发构建
scripts\cargo-msvc.bat test                                    # 宿主测试
scripts\cargo-msvc.bat test --workspace --manifest-path ..\LumaKit\Cargo.toml
scripts\cargo-msvc.bat clippy --all-targets
scripts\cargo-msvc.bat fmt --check
```

直接运行 `cargo` 可能用到 Git for Windows `usr\bin` 里的 `link.exe`（GNU coreutils，不是链接器），报 `extra operand … Try 'link --help'`。

## 脚本

| 脚本 | 作用 |
| --- | --- |
| `scripts\check.bat` | `--release --locked` 运行 LumaKit workspace 与宿主全部测试 |
| `scripts\build.bat` | 快速 release 编译，产物在 `target\release\luma-desk.exe` |
| `scripts\build-app.bat` | 正式打包：`check.bat` → `release-artifacts.ps1` 构建并发布 → `-Verify` 复核 |
| `scripts\run.bat` | 只启动 `dist\LumaDesk-GPUI\LumaDesk-GPUI.exe`，不编译 |

`build-app.bat` 产物：

```text
dist\LumaDesk-GPUI\LumaDesk-GPUI.exe        日常运行
dist\LumaDesk-GPUI\THIRD-PARTY.txt          依赖许可
dist\gpui\installer\<版本>-<构建标识>\LumaDesk-GPUI_<版本>_x64-portable.zip
dist\gpui\release.json                      版本、包路径与 SHA-256
```

ZIP 只含 EXE 与许可声明，从不打包用户数据。发布替换只换文件，不枚举、复制或删除数据目录；`powershell -NoProfile -File scripts\release-artifacts.ps1 -Verify` 可单独复核。不会覆盖旧的 egui 正式产物 `dist\LumaDesk\`。

发布脚本显式按 UTF-8 解码 Cargo metadata，避免 Windows PowerShell 在中文仓库路径中生成乱码产物路径；模型目录的 OMP MIT 许可同时写入 `THIRD-PARTY.txt`。验证新版界面必须确认运行路径和发布哈希，不在编译尚未完成时启动旧 target EXE。

输入栏的上下文／token／消费状态来自 LumaKit `lumakit-usage-status`；宿主使用请求时保存的用户模型单价快照，只有单价完整有效才显示 `≈$金额`／`≈¥金额`，否则显示 token，不套用目录参考价、不显示美元未知占位。`~` 是上下文估算，USD 用 `$`、CNY 用 `¥`，不换算。Windows 图标已实窗验证，macOS/Linux 仅预留资源与目标映射。高压力按 OMP 的百分比或绝对 token 阈值着色；魔杖只作状态标记。设计见 `../LumaKit/docs/context-usage-status.md`。

## 数据目录

默认是 EXE 旁的 `data-gpui\`；`LUMADESK_DATA_DIR` 显式指定时优先。验证或截图请用临时目录，不要指向正式资料。同一目录有文件锁，不能同时开两个写入实例。

## 换行符

`.gitattributes` 规定源码与文档用 LF，`.bat` / `.cmd` 用 CRLF；批处理还必须是纯 ASCII，否则 cmd.exe 会按 OEM 代码页误解析。

## 格式与 lint 现状

- 2026-11-09 日志续作统一运行 rustfmt 后，宿主与 LumaKit 的 `fmt --all -- --check` 均通过；严格 clippy 的旧警告已按等价修复清除，没有新增 lint 屏蔽。
- LumaKit 定义了 `[workspace.lints]`，但 `lumakit-design`、`lumakit-gpui` 与本项目尚未启用；启用同样单独进行。
- `gpui-kit/test-support` 在 LumaKit 里无法与 egui 同时解析（会引入 gpui-fast 对 taffy/unicode-properties 的精确版本），所以 `lumakit-design` 没有带入 ThinkRail 的无头窗口测试；纯逻辑测试照常运行。

## 后台日志

设置 → 后台日志支持五级实时切换（保存到隔离数据目录的 `logs/level.json`）、TraceId/ME/模块搜索、EV/Layer 交集筛选、复制完整诊断 ID 与筛选结果。只在该页打开时读取最近 1000 行；INFO 下未记录的 DEBUG 历史不能补录。失败消息的复制按钮与聊天折叠诊断中的诊断 ID 可直接粘贴搜索。

模型目录刷新、检测/同步、搜索检测/读取、MCP 配置保存均显式关联日志；MCP 页面目前不提供网络发现动作。成功/失败/取消/超时验证使用临时 `LUMADESK_DATA_DIR` 与本机回环服务，不发送真实密钥。任务和实测记录见 [LumaKit 日志任务](../../LumaKit/docs/Rust-LumaKit-日志-请求-响应-异常封装任务列表.md)。

开发规则见[日志与异常规范](日志与异常规范.md)，用户排查见[日志查看与问题排查指南](日志查看与问题排查指南.md)。2026-11-10 复查发现旧 L6/L7 验收未覆盖普通保存、启动失败与 panic；本轮整改与证据单独记录，不能用既有 DONE 推断新规范全覆盖。

## 模型服务共享界面

服务商栏、详情标题、密钥区、密钥策略、线路区、模型工具栏/分组/行、发现模型行，以及服务商/线路/密钥/模型/发现弹窗的字段、滚动区和页脚统一使用 `lumakit_gpui::providers`。宿主只提供真实业务控件和回调；模型的能力声明、容量、用途、八类模型类型、流式与完整分档价格编辑继续保留，保存原始模型配置，不把来源参考投影写回目录。

首个密钥没有单独的「加密保存」按钮：Enter 或离开输入框提交一次；成功清空，失败保留明文草稿供修改或 Enter 重试，Enter 后的 Blur 不重复提交。密钥管理弹窗只在 Enter/显式添加或替换时提交，不因取消/完成触发 Blur 保存；输入和提交绑定弹窗捕获的服务商/线路，不跟随背景选择。离开设置页清除秘密草稿。密钥单独持久化，不顺带提交未完成地址；启用开关仍独立保存。

密钥管理参考 ThinkRail Native 的紧凑单行输入：可选标签（120 逻辑像素）、遮罩密钥、添加/保存；已保存行显示标签、遮罩、复制、修改、删除和真正的启用 Switch，页脚显示「N / M 已启用」，不再保留「使用此密钥」按钮。主行显示已保存标签，只有主行可以显式查看明文，删除只在管理弹窗。修改预填标签但不解密旧密钥，密钥留空仅更新标签并保持原密文字节及稳定 ID；标签去除首尾空白后为空即未命名，最多 128 个 Unicode 字符且不能含控制字符。成功、取消、切换线路与离开设置同时清空标签和密钥草稿；保存失败保留两者。旧密钥迁移为未命名且启用，不解密或重加密。停用密钥不进入连接解析和检测轮换；存有密钥但全部停用时明确拒绝，不发送匿名请求。

检测/同步只使用已保存配置。进行中保留完整配置展示，在密钥区显示进度与取消；服务商/线路切换、启用、密钥/地址/模型修改和再次请求禁用，取消后仍等待工作线程回收再恢复。后台聊天模型目录刷新保持原有独立生命周期。发现选择即时保存，失败保留选择状态与诊断。

本次按职责拆分：原 1273 行页面已拆为输入事件、宿主动作、服务商、线路、模型、密钥、弹层和同步选择私有模块；超过 500 行的模型编辑器拆为唯一草稿/校验所有者及分类、容量、价格视图。共享组件按表单、页面组合、模型列表、服务商栏分为独立模块；后端目录存储拆为迁移入口、读取、事务写入、校验和独立测试，保持单一事务入口。最终行数与超 300 行职责判断见共享设计的验收记录。

2026-11-10 集成验证：LumaKit catalog相关14项及关联集成、secrets4项、宿主模型设置17项通过；宿主及共享 workspace 严格 clippy 通过。隔离旧库实窗验证schema2→3→4保留ID/密文；标签新增/只改标签/重启保持；SQLite trigger制造标签或Switch保存失败，草稿保留/开关恢复并可重试。Switch已实际切换0/2、1/2，重启状态保持且全部停用时检测无网络请求。320/375/414/768逻辑宽度最终Switch在行边框内，遮罩、标签、动作、提交和关闭可达；检测成功/429失败/生成中取消及键盘服务商选择已操作。截图见 `C:/Users/Administrator/AppData/Local/Temp/provider-label-final-7ey4rrsm/desk-switch-manager-final-*.png`；正式包与哈希由发布脚本输出 `dist/gpui/release.json`。

正式 `scripts/build-app.bat` 完成：LumaKit release全套及宿主43项通过，EXE/ZIP/包内EXE哈希校验通过；构建标识 `0.1.0-0a0f40c28e5647f7ad7a6e2abc781005`。发布后的 `dist/LumaDesk-GPUI/LumaDesk-GPUI.exe` 已用隔离数据实窗打开密钥管理，确认标签、Switch和计数存在并正常退出；截图 `published-release-switch-manager.png`。EXE SHA-256 `5529ac2f8348734338619dbcf6877d2eb44b1e7c145798f93d4bcb3a26e6d3b2`。

## 网络搜索密钥管理

搜索和读取网页服务复用模型服务的密钥行、紧凑输入和弹层组件：可选标签、遮罩密钥、复制/修改/删除、独立启用 Switch 与启用计数。修改时密钥留空仅保存标签；旧 settings.db schema 2 升级为 3，原 ID、密文与首选密钥保持不变，旧密钥默认启用。已停用密钥不解密、不发请求；全部停用时明确拒绝，即使服务商支持匿名请求也不降级。

管理按钮的鼠标按下仅抑制首钥失焦提交，点击后才打开弹层，避免后续鼠标抬起命中新弹层。管理输入仅 Enter 或显式添加/保存提交；关闭/取消不保存。凭据保存不提交地址草稿；标签保存失败保留草稿，Switch 保存失败保持原状态。视图按密钥区（280 行）与弹层（233 行）拆分；输入 112 行、持久化 211 行。搜索状态所有者 420 行、网络协议 420 行、设置存储 377 行各保留既有单一职责，无新增超过 500 行的生产模块。

2026-11-10 搜索续作验证：`test --locked search_settings` 21 项、`test --locked settings_store` 1 项、fmt 检查及严格 clippy 通过。隔离实窗验证密钥留空只改标签且密文不变、SQLite trigger 保存失败/重试及 Switch 回滚、重启状态、仅第二把启用密钥实际发送、全部停用零网络、回环 HTTP 成功/429/取消与进行中禁用。320/375/414/768 逻辑客户端宽度均已截图检查，管理输入、动作、Switch、关闭可达；日志未包含测试密钥。截图目录 `C:/Users/Administrator/AppData/Local/Temp/search-key-consistency-c63rjq46/`，最终布局为 `search-final-client-*.png`。本节验证的是 `target/debug/luma-desk.exe`，不代表上一节已发布 EXE 已包含本次搜索改动。

搜索版正式发布：源码提交 `443ccd9`，`scripts/build-app.bat` 完整通过（Kit release 195 项、宿主 release 49 项）；构建标识 `0.1.0-5e1a0b554afc473d90b261a27abdbd03`。EXE SHA-256 `fa3a3bec88f5591edd1f47be7c9836ab68d93fdac9b5cc358b12af41cc45bda0`；EXE、ZIP 和包内 EXE 校验通过。新发布 EXE 已用隔离数据实窗打开网络搜索密钥管理，确认标签/密钥输入可访问并正常退出（退出码 0）；隔离目录 `C:/Users/Administrator/AppData/Local/Temp/search-release-u31j_l2q/`。上述 debug 验证范围与此正式发布冒烟分别记录，不将前者视为全部 release 场景重测。

## 2026-11-10 日志与异常规范整改

规范已落入 `docs/日志与异常规范.md`，AGENTS 与工程规范索引接入；排查指南和共享设计同步。共享日志使用同一同步 writer 记录安全 panic ERROR，不调用默认 payload stderr hook；源码位置去开发机绝对前缀，损坏日志级别不再静默默认，flush 传播真实写入失败。HTTP/SSE 错误不回传外部正文，URL/request-id 只保存诊断哈希。

聊天保存失败按类型记录稳定码与真实 TraceId，连续未恢复失败不重复报警；请求提交保存由请求 root 独占 ERROR，最终回答保存失败独立报告并保留内容。`ai.request` 成功结果 `NETWORK_COMPLETED_PENDING_SAVE` 不冒充已持久化。设置加载、普通 CRUD/密钥/启停、搜索偏好、外观和用量查询补最终边界；存储错误只返回安全中文。模型和搜索移除 catch_unwind，panic ERROR 归 hook；join WARN、guard UNFINISHED 保留程序中断。取消/隐藏/关闭只丢弃成功，不覆盖已返回失败；失效结果 DEBUG 记录固定原因。

实际命令均经 `scripts/cargo-msvc.bat`：

| 验证 | 实际结果 |
| --- | --- |
| `test --locked` | 宿主63项通过，0失败/ignored；故障writer回归使用独立子进程避免并发subscriber污染 |
| `test --manifest-path ../LumaKit/Cargo.toml --target-dir ../LumaKit/target -p lumakit-ai --locked` | 单元59、model picker13、chat session40、stream5，共117项通过 |
| `test ... -p lumakit-observability --locked` / `--test panic_process` | 15项单元通过；独立panic测试通过debug unwind与另编译release panic=abort、字符串/非字符串载荷、Error级别重载、链路及隐私扫描 |
| 宿主 `clippy --all-targets --locked -- -D warnings`；Kit上述两crate同命令 | 严格clippy通过，无新增lint豁免 |
| 宿主fmt/check all-targets，Kit两crate fmt check；宿主build | 均通过，实际运行 `target/debug/luma-desk.exe` |

实际程序使用隔离目录 `C:/Users/Administrator/AppData/Local/Temp/lumadesk-logging-fixed-i8ygsrvg/`，不访问真实凭据/供应商。无效外观配置：界面显示安全中文、错误码与TraceId，日志仅一个对应ERROR，未误报app.started。搜索：回环成功、429失败、进行中取消（567ms）、SQLite trigger保存失败和移除trigger重试均操作。模型：新增服务商成功，trigger失败保留名称草稿，修复后再次保存成功。打开后台日志实际观察筛选与诊断控件。观察的16个root均恰有一个终态；测试密钥、SQL故障正文、上游正文、URL路径标记均未出现在日志。截图为 `startup-failed-fixed.png`、`search-{success,failure,running,cancelled,save-failed}.png`、`model-save-failed.png`、`log-view-fixed.png`。

规模评审：启动组合抽至startup128行；main356、宿主chat427、theme399（含测试）、settings_store419（含测试）、model状态306/持久化212/worker303/刷新441、search状态349/持久化274/worker233，均保持单一状态/事务/编排所有者，未新增超过500行模块。Kit诊断127、composer486、generation404，日志lib205/format303/writer182。既有AI lib1140行本轮抽出HTTP失败私有职责，未扩大运行时接口；protocol946行仅收紧四处错误出口，继续拆整个编码/解码状态机将扩大日志修复范围，本次作为具名规模例外，下次修改对应协议职责时继续拆分；两者由AI117项与严格clippy覆盖。

代码引用工具：尝试 rust-analyzer references 时服务器退出（code 0），后续按目标符号与全部调用方搜索核对；未宣称语言服务器引用检查成功。

边界：没有改UI布局，未重测320/375/414/768布局矩阵；没有在实际窗口制造panic或等待AI120秒超时，前者用独立子进程、后者沿既有超时合同与定向回归。未重跑无关Kit crate/Native workspace。此次更新源码与debug运行产物，未重新打包 `dist` 正式EXE；不要把此前发布哈希当作本次修复产物。磁盘不可写、系统强杀、访问违规等仍不能保证Rust日志落盘。

## 已知问题

- **Windows Debug 栈溢出**：GPUI 嵌套布局超出默认 1 MiB 主线程栈，`build.rs` 设置 `/STACK:8000000`。
- **打包报 EXE 被占用（拒绝访问）**：`LumaDesk-GPUI.exe` 或 `target` 下的 EXE 仍在运行，先关闭再打包。
- **图标显示为空方块**：Kit 默认 `Assets` 只含其组件用到的 Lucide 图标。宿主必须注册 `lumakit_design::UiAssets`；新用到包外图标时加进 `lumakit-design/src/assets.rs` 的 `Extra` 列表，其测试会核对。
- **Ctrl+Enter 只换行**：Kit `Textarea` 把 Ctrl/⌘+Enter 绑定为 `Enter { secondary: true }`，GPUI 先派发绑定再派发原始按键，所以 `capture_key_down` 拦不到；聊天输入用 `capture_action::<Enter>`。
- **含行内代码的段落显示为空白条**：Kit 0.9.0 的 HTML 导入不给节点源码区间，同一文档里多个含行内代码的段落共用元素 id，只有最后一个能绘制。回答先由 pulldown-cmark 清理成安全 Markdown 再交给 `TextView::markdown`，并用 `image_source` 拒绝一切图片加载。
