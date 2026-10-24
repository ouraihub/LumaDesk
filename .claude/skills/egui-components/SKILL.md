---
name: egui-components
description: 指导 Rust egui UI 项目的组件选型、布局与交互实现。用于控件、表单、弹层、键盘、可访问性修改，以及将其他 UI 框架的交互移植到 egui；不用于纯后端或协议开发。
---

# egui 组件开发

先确认项目版本与已有组件，再按下表选型。涉及某类任务时，编码前读取对应规则；不需要一次加载全部参考。

| 当前任务 | 必读规则 |
| --- | --- |
| 输入、表单、设置、单选多选、数值与搜索选择 | [forms.md](rules/forms.md)：选型、状态、标签、校验与保存正反例 |
| 页面布局、侧栏、滚动、页签、菜单、弹层与列表 ID | [composition.md](rules/composition.md)：组合模式、关闭语义与稳定 ID 正反例 |
| 主题、图标、禁用、加载、进度、错误与空状态 | [styling.md](rules/styling.md)：语义样式、可访问性与反馈正反例 |

例如“做一个带删除确认的设置页”：读取 forms 与 composition；涉及主题或图标时再读 styling。规则中的 Rust 示例按 egui 0.37.0 验证，不意味着使用方必须升级到该版本。

## 开始前

1. 读取项目的 Cargo.toml/Cargo.lock，确认 egui 和实际集成层的版本；核对使用中的渲染、Markdown、图标依赖，避免不兼容的 `Ui` 类型。
2. 查找项目已有的组件模块、公开导出和相关调用者，优先复用；不要假设存在某个目录或共享组件 API。
3. 确认修改的是草稿还是立即生效状态，写清保存、失败恢复和取消行为，再选择组件。
4. 修改具体控件前阅读锁定版本的官方组件文档；参考外观不能替代键盘、焦点、提交和关闭合同。

## 组件选择

| 需求 | 选择 | 交互要求 |
| --- | --- | --- |
| 点击执行动作 | `Button` / `ui.button` | 用 `.clicked()` 返回动作，禁用用 `add_enabled` |
| 独立布尔值 | `Checkbox` / `ui.checkbox` | 需要开关外观时先找项目 Switch；不为普通布尔值默认造组件 |
| 多选/能力集合 | 多个 `Checkbox` | mixed 状态单独建模，indeterminate 外观不等于三态存储 |
| 少量互斥选项 | `ui.radio_value` | 单一权威值，不用多个独立 bool |
| 紧凑模式/页签切换 | `ui.selectable_value` + 内容分支 | 状态跨帧保留；页签不是多个独立 bool |
| 固定候选下拉 | `ComboBox` | 当前值、未知值和禁用状态；不静默选第一项 |
| 搜索选择 | 已有选择器或 `Popup` + `TextEdit` + 候选列表 | 单独实现过滤、高亮、键盘选择和关闭合同 |
| 单行/多行/密码输入 | `TextEdit` | 稳定 ID、关联标签、校验、IME；密码遮罩不是加密 |
| 范围数值/精确数值 | `Slider` / `DragValue` | 根据相对调节或精确编辑选型，领域层仍须校验 |
| 颜色/图片/链接 | 颜色编辑控件、`Image`、`Hyperlink` | 匹配存储色彩空间；图片需要宿主 loader |
| 行列/卡片/分隔/折叠 | `horizontal`、`vertical`、`Grid`、`Frame::group`、`Separator`、`CollapsingHeader` | 优先组合内置控件，不假设有 Card API |
| 边缘栏/中心/滚动 | `Panel`、`CentralPanel`、`ScrollArea` | 边缘先占位，中心最后；大列表按行高选择虚拟化方式 |
| 菜单/右键菜单 | `MenuBar`、`ui.menu_button`、`Response::context_menu` | 复用关闭和定位逻辑，不手绘菜单 |
| 悬停说明/锚定内容 | `on_hover_text`、`on_hover_ui` / `Popup` | 提示与交互浮层分开；关键内容不能只在 hover 可见 |
| 模态任务/删除确认 | `Modal` + 明确确认/取消 | 背景阻断、标题、焦点进入/恢复；Esc/遮罩不执行确认 |
| 非模态工具窗 | `Window` | 不当作模态确认使用 |
| 进度/未知时长 | `ProgressBar` / `Spinner` | 不伪造进度，搭配状态文字 |
| 错误/说明/空状态 | `Label`、`Frame` 和布局组合 | 错误归属清楚，失败不能只用短暂提示 |

上表列的是 egui 内置控件与组合方式，不是需要逐个安装的组件包。Switch、Combobox 搜索行为、Card、Alert、Toast、Skeleton 不应被假定为内置 API。高级表格可评估 `egui_extras::TableBuilder`，绘图可评估 `egui_plot`；先查现有依赖及其 egui 兼容版本，只有真实需求超出内置与现有组合时才引入扩展。

控件只返回变更或动作，不直接发请求、保存文件。立即生效设置独立提交，不能意外保存其它草稿；保存期间禁用，失败保留恢复入口。

## 最小用法

状态由宿主持有，绘制函数只改草稿并返回动作。多个实例由调用者用 `push_id` 隔离。

```rust
#[derive(Default)]
pub struct Options { enabled: bool, name: String }
impl Options {
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        ui.checkbox(&mut self.enabled, "启用");
        let label = ui.label("名称");
        ui.add(egui::TextEdit::singleline(&mut self.name).id_salt("name"))
            .labelled_by(label.id);
        ui.add_enabled(!self.name.trim().is_empty(), egui::Button::new("保存")).clicked()
    }
}
```

完整的校验、忙碌状态与失败保留见 forms；持久化由宿主处理，不能把返回的点击意图当作保存成功。

## 操作流程

1. 从 Cargo.toml、Cargo.lock、现有 UI 入口确认实际集成与导入习惯；使用 eframe 的项目优先沿用 `eframe::egui`，不另引入不同版本的 `Ui`。
2. 用组件表选内置控件，查项目已有封装，再读对应规则与锁定版本文档。API 在 [egui 文档](https://docs.rs/egui) 按版本定位；通过 [官方组件示例](https://github.com/emilk/egui/tree/main/crates/egui_demo_lib/src/demo) 查找组合思路，签名不要直接照抄 main。
3. 新增依赖前确认版本、features、平台与宿主兼容；出现两套 egui 类型时用 `cargo tree -d` 定位重复版本，再检查各依赖约束。内置 Button、TextEdit 等不需要安装操作。
4. 在现有模块实现，保留已有主题与资源；升级仅限任务需要的依赖，不覆盖用户自定义组件，不顺便刷新整个锁文件。
5. 按下节验证行为与实际界面；交付列出实际验证和未验证项。

## 布局与状态

- 头尾面板先占位，`CentralPanel` 最后使用余下空间。不要混用反向布局和正向子布局猜测消息高度。
- 列表项、编辑器、弹窗和滚动区使用组件实例 + 稳定业务 ID；不依赖可变序号或显示文字。
- 正文按可用宽度折行；代码/表格局部横向滚动。长草稿、错误和名称不能推走发送/保存按钮。
- 使用逻辑 point 与宿主主题；检查浅深色、短窗口和高 DPI，不硬套 CSS 物理像素。
- 优先使用宿主 `Visuals`/`Style` 的语义颜色与已有尺寸；图标复用宿主资源，不在控件内另装字体或硬编码另一套浅深色。纯图标按钮必须有可访问名称，不能只依赖悬停提示。
- 自定义 Widget 使用 `Sense::click`、`Response::clicked`、`mark_changed`；别再单独处理按键造成双触发。提供名称、角色及选中/禁用元数据，焦点必须可见。
- 区分 hover、focus、pressed、disabled、busy、error；不为每个状态强加动画。
- 快捷键避开 IME 组合输入，长按不重复提交。`Modal::should_close()` 表示关闭请求，不是确认动作。
- 绘制可多 pass：收集动作后由控制器处理副作用。使用 eframe 且当前版本提供 `App::logic` 时，在该阶段归约后台结果；其它集成由事件循环提供唤醒与收尾，不能依赖窗口可见时才处理。
- 不每帧复制大型视图数据、重解析未变化的内容或写盘。缓存需失效条件和上限；凭据、持久化和网络不进入共享绘制层。

## 验证与交付

1. 按改动覆盖实际行为：单次激活、禁用、失败恢复、取消、焦点与实例隔离；只加能捕获消费者缺陷的检查。
2. 捕获真实 egui 帧的区域矩形，检查不重叠、按钮可达；长内容和短窗口都要覆盖。
3. 在目标运行环境观察改动；原生窗口与浏览器集成分别验收。headless 几何不代替视觉验收，AccessKit 输出不代替系统读屏验收。
4. 依赖改动验证实际应用；存在共享组件时也验证其消费宿主。报告实际结果与未验证项，不把编译成功当成 UI 正确。

## 按需参考

- 需要验证命令和证据边界时读 [检查说明](references/verification.md)。
- 新增自定义开关时读 [交互样本](probes/src/lib.rs) 的 `switch` 及键盘/禁用检查；该样本不是生产组件，需补齐标签、焦点视觉和提交合同。
- 布局占位不确定时读 [帧与面板样本](probes/src/main.rs)，再在实际组件中检查矩形。

只读取与当前任务相关的参考；不要把整个样本工程当作默认上下文，也不要直接接入产品。
