# 构建

工具链由 `rust-toolchain.toml` 固定为 Rust 1.99.0（含 `rustfmt`、`clippy`），edition 2024，依赖锁定在 `Cargo.lock`。LumaDesk 通过路径依赖使用相邻的 `../LumaKit`（`lumakit-ai`、`lumakit-design`、`lumakit-gpui`、`lumakit-support`、`lumakit-observability`），两个仓库须并排检出。

当前分支 `experiment/gpui-kit` 的界面基于 crates.io `gpui-kit = "=0.9.0"`（默认后端 gpui-pre 0.3.11，不启用 gpui-fast），LumaKit 的 `lumakit-design`、`lumakit-gpui` 锁定同一版本。

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

- 现有源码未经 rustfmt 统一（`fmt --check` 有大量差异）。按《AI 工程规范》§11，全仓格式化须单独提交；功能改动只格式化新文件，不重排已有文件。
- LumaKit 定义了 `[workspace.lints]`，但 `lumakit-design`、`lumakit-gpui` 与本项目尚未启用；启用同样单独进行。
- `gpui-kit/test-support` 在 LumaKit 里无法与 egui 同时解析（会引入 gpui-fast 对 taffy/unicode-properties 的精确版本），所以 `lumakit-design` 没有带入 ThinkRail 的无头窗口测试；纯逻辑测试照常运行。

## 已知问题

- **Windows Debug 栈溢出**：GPUI 嵌套布局超出默认 1 MiB 主线程栈，`build.rs` 设置 `/STACK:8000000`。
- **打包报 EXE 被占用（拒绝访问）**：`LumaDesk-GPUI.exe` 或 `target` 下的 EXE 仍在运行，先关闭再打包。
- **图标显示为空方块**：Kit 默认 `Assets` 只含其组件用到的 Lucide 图标。宿主必须注册 `lumakit_design::UiAssets`；新用到包外图标时加进 `lumakit-design/src/assets.rs` 的 `Extra` 列表，其测试会核对。
- **Ctrl+Enter 只换行**：Kit `Textarea` 把 Ctrl/⌘+Enter 绑定为 `Enter { secondary: true }`，GPUI 先派发绑定再派发原始按键，所以 `capture_key_down` 拦不到；聊天输入用 `capture_action::<Enter>`。
- **含行内代码的段落显示为空白条**：Kit 0.9.0 的 HTML 导入不给节点源码区间，同一文档里多个含行内代码的段落共用元素 id，只有最后一个能绘制。回答先由 pulldown-cmark 清理成安全 Markdown 再交给 `TextView::markdown`，并用 `image_source` 拒绝一切图片加载。
