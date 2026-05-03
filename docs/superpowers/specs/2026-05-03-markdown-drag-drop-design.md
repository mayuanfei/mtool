# Markdown 编辑器拖拽打开文件 — 设计文档

**日期：** 2026-05-03

## 概述

为 Markdown 编辑器添加拖拽打开文件功能：用户从系统文件管理器拖拽 `.md`/`.markdown`/`.txt` 文件到编辑器窗口，即可打开文件。

## 需求

- 拖拽文件到编辑器任意位置 → 替换当前内容（与"打开文件"按钮行为一致）
- 拖拽悬停时显示视觉反馈（高亮边框 + "释放以打开文件"提示）
- 整个编辑器页面均为拖拽区域

## 实现方案

采用 Tauri v2 内置 `tauri://drag-drop` 事件。

### Rust 端

新增 `open_md_file_by_path` 命令：

```rust
#[tauri::command]
fn open_md_file_by_path(path: &str) -> Result<(String, String), String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取文件失败: {}", e))?;
    Ok((path.to_string(), content))
}
```

注册到 `invoke_handler`。

### 前端 (MarkdownEditor.tsx)

1. **拖拽事件监听**：`useEffect` 中通过 `listen('tauri://drag-drop')` 监听原生拖拽事件，调用 `invoke('open_md_file_by_path', { path })`
2. **视觉反馈**：外层容器上监听 `onDragEnter`/`onDragOver`/`onDragLeave`/`onDrop`，控制覆盖层显隐
3. **覆盖层**：绝对定位、半透明背景、虚线边框、居中显示"释放以打开文件"

### 不影响

- 现有 `open_md_file`、`save_md_file`、`save_md_file_as` 命令
- 快捷键、视图模式、滚动同步等功能
