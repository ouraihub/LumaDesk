# 组件规则的预研证据

核对日期：2026-10-23。正式项目仍使用 egui/eframe 0.33.3，没有因编写本 skill 自动升级。`probes` 是隔离的 0.37.0 可重跑工程，不是生产控件库，不进入发布 ZIP。

## 必要性与组织

已发生的聊天布局重叠和启用控件选型偏差，说明仅靠通用审美建议不够。skill 必须约束版本、组件语义、布局占位、状态提交及运行验收，而不是提供另一份“看起来好看”的教程。

结构参考现有 `.claude/skills/shadcn-svelte` 的入口与按需参考组织，不照搬 Web 的 DOM、Field、Tailwind、CLI 规则。skill 只能提供可核验约束，不能保证模型生成代码永远正确。

## 官方来源与本地证据

- [egui 0.37.0 正式发布](https://github.com/emilk/egui/releases/tag/0.37.0)：非预发布；主题插件属于 experimental，不等于整个版本不可用。
- [0.37.0 控件导出](https://github.com/emilk/egui/blob/0.37.0/crates/egui/src/widgets/mod.rs)：没有基础库直接导出的 Switch。
- [官方 toggle 教程](https://github.com/emilk/egui/blob/0.37.0/crates/egui_demo_lib/src/demo/toggle_switch.rs)：allocate、Sense、Response、绘制四步；示例不是正式产品组件。本探针自主实现最小交互，并用 Role::Switch 命名。
- [0.37.0 Context](https://docs.rs/egui/0.37.0/egui/struct.Context.html)、[Panel](https://github.com/emilk/egui/blob/0.37.0/crates/egui/src/containers/panel.rs)、[Modal](https://github.com/emilk/egui/blob/0.37.0/crates/egui/src/containers/modal.rs)：核对 run_ui、面板空间消费和 should_close。
- [Bits UI Switch](https://www.bits-ui.com/docs/components/switch)、[Checkbox](https://www.bits-ui.com/docs/components/checkbox)、[shadcn Switch](https://shadcn-svelte.com/docs/components/switch)：二态开关与支持 mixed 的勾选框不能仅按外形替换。
- AutoNote 实际安装源码 `node_modules/@ouraihub/aether-ui/src/providers/ProvidersSettings.svelte:207–221,458–464`：启用独立提交、期间禁用、失败恢复，不提交整份服务商草稿。
- 实际 `bits-ui/dist/bits/switch/switch.svelte.js:25–54`：Space/Enter、disabled、role=switch、aria-checked。
- 当前 LumaDesk `src/model_settings.rs`：启用仍通过整份草稿保存；这是待改的产品行为，不应写成“已经实现的共享 Switch”。
- 当前 `lumakit-egui/src/lib.rs` 只导出聊天相关类型；不能指导模型调用尚不存在的通用 Switch API。

## 隔离实测

环境：Windows x64，Rust 1.99.0，egui 精确 0.37.0，显式 default_fonts。

运行过：

```bat
call D:\Applications\msvc-portable\msvc\setup_x64.bat
cargo +1.99.0 test --manifest-path .claude\skills\egui-components\probes\Cargo.toml
cargo +1.99.0 run --manifest-path .claude\skills\egui-components\probes\Cargo.toml
```

MSVC 路径按机器调整；它不是额外下载工具的要求。probe 自带独立 workspace；不会改产品 Cargo.lock。重复研究其它版本应修改隔离工程，不直接改生产依赖。

观察结果：

- 3 个行为检查通过：开关 Space/Enter、disabled 和 AccessKit 输出生成；多尺寸长内容面板占位；模态 Esc 只取消不确认。
- CPU 烟测实际运行3帧并细分绘制图元。860×560逻辑区域得到 header `(0,0)–(860,50)`、transcript `(0,50)–(860,450)`、composer `(0,450)–(860,560)`；分区不重叠。
- 首次依赖解析证明 egui 0.37 没有可选 `accesskit` feature；去掉该 feature 后可编译。
- 首次 headless 帧因丢弃未应用 TexturesDelta 而 panic；无 GPU 探针显式 clear 后通过。此处 clear 只适用于没有 renderer 的测试，正式应用必须应用纹理更新，不能 suppress 掉实际渲染。

## 不代表已验证的内容

- 没有升级正式应用、没有完成生产 Switch，也没有生成新的应用 EXE。
- CPU 帧/合成键盘不等于真实窗口、系统读屏或全部 DPI 的视觉验证；AccessKit 有输出不等于已核对每个 OS 辅助功能行为。
- 本开关探针缺少产品级焦点视觉、动画/减弱动画、保存错误策略和标签参数化，不能直接复制为完成的生产组件。
- 没有声称 Modal 自动等价 Bits Dialog 的所有焦点行为；进入/恢复焦点、Tab 环绕、嵌套 popup、遮罩点击均需消费场景单独验收。
- 没有声称 ComboBox 里放一个输入框就获得完整命令式 Combobox 键盘行为。

## 升级依赖条件

已查发布清单：egui_commonmark 0.26.0 依赖 egui 0.37；egui-phosphor 0.14.0 仍依赖 egui 0.36。两者是清单证据，不是整个项目升级成功证据。图标字体方案需要独立适配；不得通过引入第二份 egui::Ui 类型来凑编译。实现时重新核对发布版本，不把日期快照当永恒事实。

## 规则回归场景

评审一个模型改动时至少问：

1. 请求加“启用”时，是否先读实际保存合同，而非一律画 Checkbox 或一律 Switch？
2. 请求加搜索下拉时，是否列出键盘缺口，而非把搜索文本框当完整 Combobox？
3. 请求删除确认时，是否区分 Window、Modal 和显式确认，Esc 不执行删除？
4. 请求升级0.37时，是否拒绝复制0.33的 WidgetType/面板/feature API，先在隔离工程验证？
5. 请求查看效果时，是否区分 headless 几何与真实窗口，而不是说“测试通过所以 UI 没问题”？

这些是评审用场景，不宣称已经跑过模型行为评测。新增 API 片段需先编译并跑对应路径，再加入 skill。
