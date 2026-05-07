# MTool

> 一款面向开发者的跨平台桌面效率工具箱，基于 **Tauri v2 + React 19 + TypeScript** 构建。

[![Version](https://img.shields.io/badge/version-1.0.0-blue)](https://github.com/mayuanfei/mtool)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey)]()

---

## ✨ 功能概览

| 工具 | 说明 |
|------|------|
| 📁 文件搜索 | 本地全文索引检索，支持模糊匹配与快速定位 |
| 📄 JSON 格式化 | 格式化、压缩、校验 JSON，支持语法高亮 |
| 📝 Markdown 编辑器 | 双栏实时预览，支持代码块高亮 |
| 🔐 密码生成器 | 可配置复杂度、长度、批量生成强密码 |
| 🗄️ SQL IN 构建器 | 从列值快速生成 SQL `IN(...)` 子句 |
| 📷 文本转二维码 | 将任意文本/URL 生成高清 PNG 二维码 |

---

## 🚀 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://www.rust-lang.org/) >= 1.77（通过 `rustup` 安装）
- [Tauri CLI 依赖](https://v2.tauri.app/start/prerequisites/)（平台系统依赖，参见官方文档）

### 安装依赖

```bash
npm install
```

### 开发模式（桌面应用）

```bash
npm run tauri dev
```

> 仅启动前端预览（无 Tauri 原生功能）：
> ```bash
> npm run dev
> ```

### 生产构建

```bash
npm run tauri build
```

构建产物位于 `src-tauri/target/release/bundle/` 目录。

---

## 🛠️ 工具使用说明

### 📁 文件搜索

1. 进入「设置」页面，开启「File Search」开关，程序将在后台对选定目录建立全文索引。
2. 切换到「文件搜索」页面，在搜索框中输入查询语句，实时返回匹配的文件。
3. 点击结果可直接在系统文件管理器中定位该文件。

#### 🔍 搜索语法

文件搜索支持多维度组合查询，语法格式为：

```
<glob 模式>  [关键词]  [size:<条件>]  [content:<关键词>]
```

| 修饰符 | 说明 | 示例 |
|--------|------|------|
| `*.<扩展名>` | 按文件类型过滤（glob 通配符） | `*.md`、`*.yml`、`*.jpg` |
| `<关键词>` | 按文件名匹配（模糊搜索） | `学习`、`config` |
| `size:><数值><单位>` | 文件大小大于指定值 | `size:>10M`、`size:>500K` |
| `size:<<数值><单位>` | 文件大小小于指定值 | `size:<1M` |
| `content:<关键词>` | 按文件内容全文搜索 | `content:icbc-config` |

#### 📌 示例

```
*.md 学习
```
> 查询所有扩展名为 `.md`、文件名包含「学习」的文件

```
*.jpg size:>10M
```
> 查询所有 `.jpg` 图片中，文件大小超过 10MB 的文件

```
*.yml content:icbc-config
```
> 查询所有 `.yml` 文件中，**内容**包含 `icbc-config` 的配置文件

```
*.log size:>1M content:ERROR
```
> 查询大于 1MB 且内容含 `ERROR` 的日志文件（多条件组合）

> **注意**：首次建立索引需要等待一段时间，索引完成后搜索速度极快。`content:` 全文搜索依赖 FTS5 索引，仅对文本类文件生效。

---

### 📄 JSON 格式化

- **格式化**：粘贴原始 JSON，点击「Format」，输出带缩进的可读格式。
- **压缩**：点击「Minify」，去除多余空白，生成紧凑格式。
- **校验**：输入框实时标红非法 JSON，并显示错误位置。
- **复制**：点击右上角「Copy」按钮，快速复制处理结果。

---

### 📝 Markdown 编辑器

- 左侧编辑区输入 Markdown 内容，右侧实时渲染预览。
- 支持代码块语法高亮（基于 highlight.js）。
- 支持从本地打开 `.md` 文件，以及将内容另存为文件。

---

### 🔐 密码生成器

1. 选择字符集：大写字母、小写字母、数字、特殊符号（可多选）。
2. 拖动滑块或输入数字设定密码长度（1–128 位）。
3. 设置生成数量（1–100 条）。
4. 点击「Generate」批量生成，点击单条密码可快速复制。

---

### 🗄️ SQL IN 构建器

1. 在左侧文本框中粘贴数据（每行一个值，或以逗号/制表符分隔）。
2. 选择引号风格：单引号、双引号或无引号。
3. 点击「Build」，右侧自动输出 `IN('val1', 'val2', ...)` 格式语句。
4. 点击「Copy」将结果复制到剪贴板。

> 工具会自动去重，并对引号字符进行转义，确保 SQL 安全。

---

### 📷 文本转二维码

1. 在「Raw Payload」区域输入任意文本或 URL（最多 2048 字符）。
2. 选择容错级别：`L (7%)` / `M (15%)` / `Q (25%)` / `H (30%)`，容错越高越耐损。
3. 选择分辨率：256px / 512px / 1024px / 2048px。
4. 在「颜色配置」中选择预设颜色或手动输入 HEX 值自定义二维码前景色。
5. 点击「Copy Image」复制图片到剪贴板，或「Download」下载为 PNG 文件。

> 二维码背景色会自动跟随应用主题（亮色/暗色）切换。

---

## ⚙️ 设置

| 选项 | 说明 |
|------|------|
| 主题 | 在亮色（Light）和暗色（Dark）模式间切换 |
| 语言 | 切换界面语言（中文 / English） |
| 工具开关 | 独立启用/禁用各个工具模块，关闭后从侧边栏隐藏 |
| File Search | 开启后触发文件索引构建；关闭后清除索引数据 |

---

## 🗂️ 项目结构

```
mtool/
├── src/                  # 前端源码 (React + TypeScript)
│   ├── pages/            # 各工具页面组件
│   ├── components/       # 公共 UI 组件
│   ├── App.tsx           # 应用主入口与路由状态
│   ├── theme.tsx         # 主题 Provider
│   └── i18n.ts           # 国际化
├── src-tauri/            # Tauri 后端 (Rust)
│   ├── src/
│   │   ├── lib.rs        # Tauri 命令注册
│   │   └── file_search.rs# 文件索引引擎
│   └── tauri.conf.json   # Tauri 配置
└── package.json
```

---

## 🧱 技术栈

- **前端**：React 19 · TypeScript · Vite 7 · Tailwind CSS v4
- **后端**：Rust · Tauri v2
- **数据库**：SQLite (FTS5 全文索引) via `rusqlite`
- **二维码**：`qrcode` crate
- **Markdown**：`marked` + `highlight.js`

---

## 📄 License

MIT © [mayuanfei](https://github.com/mayuanfei)
