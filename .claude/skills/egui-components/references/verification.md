# 隔离组件检查

组件 API 或布局模式不确定时运行技能目录中的 `probes/`。该样本独立锁定 egui 0.37.0，不改变使用方项目依赖；其它版本应建立对应样本验证，不能直接套用其 API。

从 `probes/` 目录执行，使用兼容其依赖的 Rust 工具链；Windows MSVC 目标需要可用的 C++ 构建环境：

```sh
cargo test --locked
cargo run --locked
```

- `test` 检查开关键盘激活/禁用、多尺寸面板占位、模态 Esc 不确认；失败时退出非零。
- `run` 执行 CPU 帧与图元细分，输出实际头部、正文和底部矩形；正常结束输出 `SMOKE PASS`。
- 无渲染器样本显式丢弃纹理增量；正式渲染集成必须应用增量，不能照搬此处理。
- 样本不证明生产开关的焦点外观、保存回滚、OS 读屏或实窗视觉；这些在消费场景验收。升级样本依赖时同步检查 API，不能用其版本替代产品 Cargo.lock。

精确版本资料从 [egui 文档](https://docs.rs/egui) 和 [官方组件示例](https://github.com/emilk/egui/tree/main/crates/egui_demo_lib/src/demo) 中选择对应版本，不直接套用 main/latest 的签名。
