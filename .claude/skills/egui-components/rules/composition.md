# 布局、导航与弹层组合

修改页面结构、滚动、菜单、弹层时读本文件。正确示例使用 egui 0.37.0；宿主传入根 `Ui`，状态跨帧保留。旧版本可能使用 `TopBottomPanel`/`SidePanel` 和不同的 `show` 参数，先查锁定版本。

## 选择容器而不是照搬 Web 名称

| 需求 | 选择 | 边界 |
| --- | --- | --- |
| 水平、垂直、自动折行 | `ui.horizontal`、`vertical`、`horizontal_wrapped` | 不靠空格或固定坐标排版 |
| 简单表单、少量列 | `Grid`、`ui.columns` | Grid 不是虚拟化表格 |
| 卡片、字段组 | `Frame::group` / `ui.group` + 标题、内容、操作 | 无内置 `Card.Header` 等 API |
| 分隔线 | `Separator` / `ui.separator` | 不为一条线创建布局容器 |
| 可折叠区域 | `CollapsingHeader` / `ui.collapsing` | 不等于页面导航 |
| 工具栏、侧栏、详情栏 | `Panel::top/left/right/bottom` | 先占位，中心内容最后绘制 |
| 中心工作区 | `CentralPanel` | 不用多个中心面板争抢同一区域 |
| 独立滚动列表 | `ScrollArea`；等高大列表用 `show_rows` | 变高行需测量/虚拟化方案，不能假定等高 |
| 页签、导航项 | 单一枚举 + `selectable_value` + 内容分支 | 无内置 `Tabs.Root`、路由或 Breadcrumb |
| 顶部菜单、右键菜单 | `MenuBar`、`ui.menu_button`、`Response::context_menu` | 不手算浮层坐标 |
| 短暂悬停说明 | `Response::on_hover_text` / `on_hover_ui` | 关键操作与错误不能只在 hover 可见 |
| 锚定触发器的交互内容 | `Popup` | 明确内外点击、Esc 与焦点关闭策略 |
| 阻断背景的任务/确认 | `Modal` | 标题、取消、明确确认动作 |
| 可移动非模态工具窗 | `Window` | `.collapsible(false)` 不会让 Window 变成模态 |
| 自定义浮动区域 | `Area` | 定位基础设施，不自动提供对话框语义 |

## 先占用边缘，再绘制中心

错误：中心先吃完可用区域，底部操作区被挤出或覆盖；依赖手算高度，长内容时更容易失效。

```rust,ignore
egui::CentralPanel::default().show(ui, |ui| { ui.label("正文"); });
egui::Panel::bottom("actions").show(ui, |ui| { ui.button("保存"); });
```

正确：操作栏独立占位，正文滚动不带走操作栏。长正文、长错误、窄窗口都需检查实际矩形。

```rust
pub fn page(ui: &mut egui::Ui) {
    egui::Panel::bottom("actions").show(ui, |ui| {
        ui.horizontal(|ui| { let _ = ui.button("保存"); let _ = ui.button("取消"); });
    });
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("body").show(ui, |ui| {
            for row in 0..100 { ui.label(format!("第 {row} 项")); }
        });
    });
}
```

可调整大小的侧栏用 `Panel::left/right(...).resizable(true)`；不要自画拖动条。真正的 Drawer/Sheet 还需实现打开关闭、遮罩、焦点与动画合同，侧栏不自动具备这些行为。

## 关闭请求不等于确认

错误：Esc、遮罩点击也会触发删除。

```rust,ignore
if modal.should_close() { delete_item(); }
```

正确：只有明确按钮返回确认意图；取消、Esc、遮罩只清空打开状态。`item_id` 必须来自用户打开确认框时选定的目标，不能在确认时重新读取可能已变化的当前选择。

```rust
pub fn confirm_delete(ctx: &egui::Context, open: &mut bool, item_id: u64) -> Option<u64> {
    if !*open { return None; }
    let mut confirmed = false;
    let modal = egui::Modal::new(egui::Id::unique(("delete", item_id))).show(ctx, |ui| {
        ui.heading("删除项目？");
        ui.label("删除后无法撤销。");
        ui.horizontal(|ui| {
            if ui.button("取消").clicked() { ui.close(); }
            if ui.button("确认删除").clicked() { confirmed = true; ui.close(); }
        });
    });
    if modal.should_close() { *open = false; }
    confirmed.then_some(item_id)
}
```

该示例只返回意图，不负责实际删除。接入时还需禁用重复提交、展示失败，并在关闭后将焦点还给触发器；内置背景阻断不能代替这些验收。

## 菜单与页签由状态驱动

正确：菜单返回动作，页签共用一个权威值；没有复制多个选中 bool。

```rust
#[derive(Default, PartialEq)]
enum Tab { #[default] General, Advanced }
#[derive(Default)]
pub struct Demo { tab: Tab }
impl Demo {
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut refresh = false;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("操作", |ui| {
                if ui.button("刷新").clicked() { refresh = true; ui.close(); }
            });
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::General, "常规");
            ui.selectable_value(&mut self.tab, Tab::Advanced, "高级");
        });
        ui.separator();
        match self.tab {
            Tab::General => { ui.label("常规设置"); }
            Tab::Advanced => { ui.label("高级设置"); }
        }
        refresh
    }
}
```

## 重排后保留正确实例

错误：可删除/重排的列表使用序号作 ID，焦点、折叠或编辑状态会串到另一项。

```rust,ignore
for (index, item) in items.iter_mut().enumerate() {
    ui.push_id(index, |ui| { ui.text_edit_singleline(&mut item.name); });
}
```

正确：业务 ID 不随位置或名称变化；外层作用域隔离同屏的列表实例。

```rust
pub struct Item { pub id: u64, pub name: String }
pub fn list(ui: &mut egui::Ui, list_id: u64, items: &mut [Item]) {
    ui.push_id(("items", list_id), |ui| {
        for item in items {
            ui.push_id(item.id, |ui| {
                let label = ui.label("名称");
                ui.text_edit_singleline(&mut item.name).labelled_by(label.id);
            });
        }
    });
}
```

## 文档入口

[Panel](https://docs.rs/egui/0.37.0/egui/struct.Panel.html)、[ScrollArea](https://docs.rs/egui/0.37.0/egui/struct.ScrollArea.html)、[Modal](https://docs.rs/egui/0.37.0/egui/struct.Modal.html)、[Popup](https://docs.rs/egui/0.37.0/egui/struct.Popup.html)、[官方示例目录](https://github.com/emilk/egui/tree/main/crates/egui_demo_lib/src/demo)；main 仅用于发现示例，实际签名以项目版本为准。
