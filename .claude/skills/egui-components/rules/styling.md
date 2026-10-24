# 主题、图标与反馈

修改外观、图标、加载和错误状态时读本文件。正确示例使用 egui 0.37.0；先查项目已有主题、字体与组件，不另建一套样式系统。

## 语义样式优先于固定颜色

错误：浅色主题的黑色正文切到深色后不可读；错误只靠红色表达，读屏和色觉差异用户无法区分。

```rust,ignore
ui.colored_label(egui::Color32::BLACK, "正文");
ui.colored_label(egui::Color32::RED, "!");
```

正确：默认正文跟随主题，错误使用宿主语义色并给出文字。没有对应语义色时复用项目约定，不臆造 `Visuals::success_color` 等字段。

```rust
pub fn feedback(ui: &mut egui::Ui, error: Option<&str>, busy: bool, progress: Option<f32>) {
    ui.label("正文跟随宿主主题");
    if let Some(error) = error {
        ui.colored_label(ui.visuals().error_fg_color, format!("保存失败：{error}"));
    }
    if busy {
        ui.horizontal(|ui| { ui.spinner(); ui.label("正在保存…"); });
    }
    if let Some(progress) = progress {
        ui.add(egui::ProgressBar::new(progress.clamp(0.0, 1.0)).show_percentage());
    }
}
```

- 已知进度用 `ProgressBar`，未知时长用 `Spinner`，不要伪造百分比。
- 保存失败保留草稿并提供重试入口，不能只闪过 Toast。空状态、加载中、无搜索结果、失败分别表达。
- 普通按钮、Checkbox、Separator 用内置控件，不用 Painter 重画；`Frame::group` 组合卡片，无内置 Alert/Badge/Skeleton/Toast，先复用项目实现。
- 图片用 `Image`；解码/网络加载是否可用取决于宿主已安装的 loader，不能假设传 URL 就一定显示。

## 禁用必须阻止交互，不只是降低透明度

错误：灰色按钮仍能点击、键盘激活，导致重复提交。

```rust,ignore
if ui.add(egui::Button::new(egui::RichText::new("保存").weak())).clicked() {
    save_again();
}
```

正确：控制交互状态；仅返回意图，不在绘制过程中执行请求。

```rust
pub fn save_action(ui: &mut egui::Ui, busy: bool, valid: bool) -> bool {
    ui.add_enabled(!busy && valid, egui::Button::new(if busy { "保存中…" } else { "保存" })).clicked()
}
```

## 图标首先是资源与语义问题

先查项目字体初始化、图像资源与图标封装；不要把另一项目的私用区码点直接复制过来。字形依赖对应字体与 fallback 顺序，否则显示方框或错误字符。没有可靠资源时，文本按钮比乱码图标正确。

错误：Painter 画出垃圾桶，但没有点击区域、键盘焦点和可访问名称；只有 Tooltip 也不能保证无障碍名称。

```rust,ignore
ui.painter().text(pos, egui::Align2::CENTER_CENTER, "\u{E000}", font, color);
```

正确：默认用带文字的 Button；产品确需纯图标时，复用项目组件并显式提供名称、角色、焦点与禁用行为。

```rust
pub fn delete_action(ui: &mut egui::Ui, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new("删除"))
        .on_hover_text("打开删除确认")
        .clicked()
}
```

自定义 Widget 使用内置交互结果，不额外处理同一个 Space/Enter 造成双触发；`Response::clicked()` 之后更新值并 `mark_changed()`，注册 `WidgetInfo`，绘制可见焦点。颜色、大小使用宿主 Style；不要每帧安装字体或修改全局主题。

## 尺寸与验收

逻辑 point 不是屏幕像素。间距优先使用 `ui.spacing()`，在当前作用域调整 Style，避免局部组件污染后续界面。浅深色、禁用、键盘焦点、长文案、窄窗口与高 DPI 均要在目标运行环境观察；几何测试不能证明字体、对比度或读屏正确。

[Style](https://docs.rs/egui/0.37.0/egui/struct.Style.html)、[Visuals](https://docs.rs/egui/0.37.0/egui/struct.Visuals.html)、[Button](https://docs.rs/egui/0.37.0/egui/struct.Button.html)、[Image](https://docs.rs/egui/0.37.0/egui/struct.Image.html)。
