# GPUI Kit 验证概要设计

## 目标

在 experiment/gpui-kit 分支以 GPUI Kit 原生 UI 实现当前 LumaDesk 功能。业务、数据库格式和安全规则不因 UI 改变；不嵌入 egui，不复制 Navop 自有代码。功能清单与验证边界见 07。

## 组件与责任

- LumaDesk DesktopApp：GPUI 根业务实体，持有应用导航、模型设置实体、搜索/MCP/外观实体、ChatState。管理文件锁、日志、任务与窗口关闭。
- LumaDesk ModelSettings：保存目录和编辑草稿唯一所有者，直接复用已有目录/请求/验证逻辑；通过只读方法提供模型选项和连接，不把密钥传播给 UI 选择器。
- LumaDesk ChatState：拥有 ChatSession，执行 ChatAction、解析连接、选择文件与打开媒体；向共享 ChatPanel 提供引用和只读模型目录。
- LumaKit lumakit-gpui：只负责聊天控件、临时编辑/弹层/滚动状态、Markdown 和安全图片展示；复用 lumakit-ui 的动作，不依赖 LumaDesk。
- LumaDesk SearchSettings/McpSettings/Appearance：GPUI 实体管理页面输入状态；沿用已有持久化与请求实现。
- LumaKit lumakit-ai/support/ui/observability：保持业务中立，通常无需修改。

## 状态与事件

GPUI InputState/EditorState 由页面实体长期持有，不在 render 中重建，订阅句柄保留在实体中。所有业务修改发生在前台实体更新；后台线程仅发送事件/唤醒，不直接访问 UI。

聊天界面发出 ChatAction 后由宿主执行；网络事件由 ChatSession::poll 归约。归约任务独立于 render，最小化仍能处理完成与保存；仅有变化时通知 UI。已有模型检测和搜索线程同样必须在页面不可见时收尾，离开设置时取消并清理密钥明文。

## UI 结构

保留启动台、顶部页签、左侧导航、设置入口。设置宽屏双栏、窄屏堆叠或紧凑导航。聊天固定头部/可滚动正文/底部输入三段；输入工具栏紧凑可换行，不使用模型双下拉占满整行。使用 Kit 原生控件与主题，维持既有信息层级和产品颜色，而非复制 Navop 工作台。

后台任务、完整错误和用量按需展开，生成图片内联，不用仅“打开图片”按钮代替。安全确认、保存失败与无效模型状态必须可见。

## 可复用与重写

保留 CatalogStore、ModelCatalog、ChatSession、ChatAction、settings_store、DPAPI 与请求函数。重写 eframe 生命周期、egui 控件绘制、纹理与主题映射。混合文件原地分离绘制与操作，不再复制一份业务后端；禁止为了验证创建平行数据库或协议层。

## 构建与回退

锁定 gpui-kit 0.9.0，LumaDesk 与共享 GPUI adapter 使用同一版本与类型。默认后端，不启用 gpui-fast，不引入 Fast fork 补丁。保留原 MSVC 环境注入；必要 Windows 清单与资源按实际发行版核对。验证脚本输出独立 EXE/ZIP，不覆盖既有正式版和数据。

回退通过两个仓库切回原 main，不能依靠产品运行时保留两套 UI。旧 egui 示例仍是 LumaKit 的独立消费者。

## 验收门槛

必须跑通完整应用，而非只有单页编译。执行真实 loopback 请求、输入/发送/停止、重启持久化与失败恢复，观察实际窗口和布局。对生产供应商、IME 候选、辅助技术等未观察结果单独标明。最终文档记录具体版本、命令、运行路径与结果。
