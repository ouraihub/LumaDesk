# 表单与输入

修改输入、设置、选择器时读本文件。示例使用 egui 0.37.0；状态由宿主持有并跨帧保留，不要在绘制函数里重新初始化。其它版本先查对应 API。

## 如何选择

| 需求 | 使用 | 不要用 |
| --- | --- | --- |
| 独立布尔值 | `Checkbox` / `ui.checkbox` | 两个互斥按钮和两个 bool |
| 产品明确需要滑动开关外观 | 项目已有 Switch；否则自定义 Widget | 假设 egui 内置 `Switch` |
| 少量互斥选项，全部可见 | `ui.radio_value` | 多个 Checkbox |
| 紧凑模式切换、页签 | `ui.selectable_value` | 手绘选中背景但漏键盘行为 |
| 较长的固定候选列表 | `ComboBox` | 用显示文字当持久 ID |
| 搜索大量候选 | 已有选择器；否则 `Popup` + `TextEdit` + 候选列表 | 以为 ComboBox 自动支持过滤与方向键高亮 |
| 单行、多行、密码 | `TextEdit::singleline` / `multiline` / `.password(true)` | 把占位文字当标签；把 password 当加密 |
| 有限范围、重视相对位置 | `Slider` | 无上限滑块 |
| 精确数值、紧凑编辑 | `DragValue` | 将所有数字当字符串手写解析 |
| 颜色 | `ui.color_edit_button_srgba` 等匹配存储类型的颜色控件 | 存储色彩空间不明却任意转换 |

## 互斥选择只维护一个权威值

错误：勾选“源码”不会取消“预览”，出现两个模式同时启用。

```rust,ignore
ui.checkbox(&mut visual, "预览");
ui.checkbox(&mut source, "源码");
```

正确：单一枚举表达互斥关系。

```rust
#[derive(Default, PartialEq)]
enum Mode { #[default] Visual, Source }
#[derive(Default)]
pub struct Demo { mode: Mode, enabled: bool }
impl Demo {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.enabled, "启用自动保存");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.mode, Mode::Visual, "预览");
            ui.radio_value(&mut self.mode, Mode::Source, "源码");
        });
    }
}
```

混合勾选状态是独立的领域状态；`Checkbox::indeterminate` 只负责外观，不能代替三态模型。

## 输入标签、校验和保存动作分开

错误：局部字符串每帧清空；只有 hint，输入后就看不到字段名称；每次重绘都可能写盘。

```rust,ignore
let mut name = String::new();
ui.add(egui::TextEdit::singleline(&mut name).hint_text("名称"));
save_to_disk(&name);
```

正确：状态跨帧保留，标签关联输入，非法或忙碌时禁止提交。返回保存意图，调用者在绘制之外执行保存；失败保留草稿并显示错误，成功后才更新已保存状态。多个实例放在不同 `push_id` 作用域。

```rust
#[derive(Default)]
pub struct Demo { pub name: String, pub busy: bool, pub error: Option<String> }
impl Demo {
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        let label = ui.label("名称");
        ui.add(egui::TextEdit::singleline(&mut self.name).id_salt("name"))
            .labelled_by(label.id);
        let valid = !self.name.trim().is_empty();
        if !valid { ui.colored_label(ui.visuals().error_fg_color, "名称不能为空"); }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_enabled(valid && !self.busy, egui::Button::new("保存")).clicked()
    }
}
```

即时生效设置只提交该字段，不顺便提交其它草稿。`Response::changed()` 表示值有变化，不代表校验或持久化成功。不要用 `lost_focus()` 代替保存意图；它也可能来自取消或切换窗口。

## 下拉选项与数值控件

正确：保存稳定值，显示文本另行映射；未知值明确显示，不静默选第一项。下例的整数只是固定选项 ID，不是可重排列表的索引。

```rust
#[derive(Default)]
pub struct Demo { format: u8, opacity: f32, retries: u32 }
impl Demo {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::new("format", "格式")
            .selected_text(match self.format { 0 => "文本", 1 => "Markdown", _ => "未知格式" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.format, 0, "文本");
                ui.selectable_value(&mut self.format, 1, "Markdown");
            });
        ui.add(egui::Slider::new(&mut self.opacity, 0.0..=1.0).text("不透明度"));
        ui.horizontal(|ui| {
            let label = ui.label("重试次数");
            ui.add(egui::DragValue::new(&mut self.retries).range(0..=10)).labelled_by(label.id);
        });
    }
}
```

数值控件的范围不是信任边界校验；文件、网络、IPC 输入仍须在领域层验证。快捷键提交须避开 IME 组合输入和按键重复，不能给所有 TextEdit 全局抢占 Enter。

## 文档入口

按项目版本查询 [TextEdit](https://docs.rs/egui/0.37.0/egui/struct.TextEdit.html)、[ComboBox](https://docs.rs/egui/0.37.0/egui/struct.ComboBox.html)、[Slider](https://docs.rs/egui/0.37.0/egui/struct.Slider.html)、[DragValue](https://docs.rs/egui/0.37.0/egui/struct.DragValue.html)。
