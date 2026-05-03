# Markdown 编辑器拖拽打开文件 — 实现计划

> **For agentic workers:** 使用 superpowers:subagent-driven-development 或 superpowers:executing-plans 来执行此计划。使用 checkbox (`- [ ]`) 跟踪进度。

**目标：** 为 Markdown 编辑器添加拖拽打开 .md/.markdown/.txt 文件的功能，拖入后替换当前内容，并显示视觉反馈。

**架构：** 前端通过 `listen('tauri://drag-drop')` 监听 Tauri 原生拖拽事件获取文件路径，调用 Rust 端新增的 `open_md_file_by_path` 命令读取文件内容。同时在前端通过 DOM 拖拽事件（`onDragEnter`/`onDragOver`/`onDragLeave`）控制视觉覆盖层的显隐。

**技术栈：** Rust (Tauri v2), TypeScript (React 19), `@tauri-apps/api/event` (listen), lucide-react (图标)

---

### Task 1: Rust 端 — 新增 `open_md_file_by_path` 命令

**文件：**
- Modify: `src-tauri/src/lib.rs:165`（在 `save_md_file_as` 之后插入新命令）
- Modify: `src-tauri/src/lib.rs:482-484`（注册到 `invoke_handler`）

- [ ] **Step 1: 在 `save_md_file_as` 之后添加新命令**

在 `save_md_file_as` 函数结束后（第 165 行 `}` 之后），插入：

```rust
#[tauri::command]
fn open_md_file_by_path(path: &str) -> Result<(String, String), String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file: {}", e))?;
    Ok((path.to_string(), content))
}
```

- [ ] **Step 2: 注册新命令到 `invoke_handler`**

在第 482-484 行区域，在 `open_md_file,` 之后添加 `open_md_file_by_path,`：

```rust
open_md_file,
open_md_file_by_path,
save_md_file,
```

- [ ] **Step 3: 验证编译**

```bash
npm run tauri build
```

确认 Rust 端编译通过。

---

### Task 2: 前端 — 拖拽事件监听与视觉反馈

**文件：**
- Modify: `src/pages/MarkdownEditor.tsx`

- [ ] **Step 1: 添加 imports**

在现有的 import 语句中添加 `listen` 和 `Upload`：

```typescript
import { listen } from '@tauri-apps/api/event';
```

在 lucide-react 的 import 中添加 `Upload`：
```typescript
import { Trash2, FolderOpen, Save, Copy, Check, Eye, Edit3, FileText, Download, Upload } from 'lucide-react';
```

- [ ] **Step 2: 添加拖拽状态 state**

在现有 state 声明区域（第 15 行后）添加：

```typescript
const [isDragOver, setIsDragOver] = useState(false);
```

- [ ] **Step 3: 添加拖拽事件监听 useEffect**

在组件中新增一个 `useEffect`（放在键盘快捷键 `useEffect` 之后，约第 128 行后）：

```typescript
useEffect(() => {
  const unlisten = listen<string[]>('tauri://drag-drop', async (event) => {
    const paths = event.payload;
    if (!paths || paths.length === 0) return;
    const filePath = paths[0];
    const ext = filePath.split('.').pop()?.toLowerCase();
    if (!ext || !['md', 'markdown', 'txt'].includes(ext)) return;
    try {
      const result = await invoke<[string, string]>('open_md_file_by_path', { path: filePath });
      setFilePath(result[0]);
      setContent(result[1]);
      setIsDirty(false);
    } catch {
      // read error
    }
  });
  return () => {
    unlisten.then(fn => fn());
  };
}, []);
```

- [ ] **Step 4: 添加拖拽视觉反馈处理器**

在组件中新增拖拽事件处理函数（放在 `handleClear` 之后）：

```typescript
const handleDragOver = useCallback((e: React.DragEvent) => {
  e.preventDefault();
  e.stopPropagation();
  setIsDragOver(true);
}, []);

const handleDragLeave = useCallback((e: React.DragEvent) => {
  e.preventDefault();
  e.stopPropagation();
  setIsDragOver(false);
}, []);

const handleDrop = useCallback((e: React.DragEvent) => {
  e.preventDefault();
  e.stopPropagation();
  setIsDragOver(false);
}, []);
```

- [ ] **Step 5: 在外层容器上绑定拖拽事件并添加覆盖层**

修改外层 `<div>`（第 179 行）：

```tsx
<div
  className="w-full h-full flex flex-col relative"
  onDragOver={handleDragOver}
  onDragEnter={handleDragOver}
  onDragLeave={handleDragLeave}
  onDrop={handleDrop}
>
```

并在该 div 的内部、所有内容之后（`</footer>` 之后、`</div>` 闭合之前），添加拖拽覆盖层：

```tsx
{isDragOver && (
  <div className="absolute inset-0 z-50 flex items-center justify-center bg-indigo-950/80 border-2 border-dashed border-indigo-400 rounded-xl pointer-events-none">
    <div className="flex flex-col items-center gap-3 text-indigo-300">
      <Upload className="w-12 h-12" />
      <span className="text-lg font-semibold">{t('Release to open file')}</span>
    </div>
  </div>
)}
```

- [ ] **Step 6: 添加国际化字符串**

需要在 `src/i18n.tsx` 中添加 `'Release to open file'` 的中英文翻译。

- [ ] **Step 7: 验证构建**

```bash
npm run build
```

确认 TypeScript 编译和 Vite 构建通过。

---

### Task 3: 端到端验证

- [ ] **Step 1: 桌面验证**

```bash
npm run tauri dev
```

1. 打开 Markdown 编辑器页面
2. 从 Finder 拖拽一个 `.md` 文件到编辑器窗口
3. 确认显示拖拽覆盖层（蓝色虚线边框 + "释放以打开文件"）
4. 释放文件，确认内容被替换为文件内容
5. 确认文件名显示在标题栏
6. 测试拖拽 `.txt` 文件也能正常打开
7. 测试拖拽非文本文件（如 `.png`）不会被接受
