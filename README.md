# MTool

> 一款面向开发者的跨平台桌面效率工具箱，基于 **Tauri v2 + React 19 + TypeScript** 构建。

[![Version](https://img.shields.io/badge/version-1.0.4-blue)](https://github.com/mayuanfei/mtool)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey)]()

---

## ✨ 功能概览

| 工具 | 说明 |
|------|------|
| 📁 文件搜索 | 本地全文索引检索，支持模糊匹配、大小过滤与内容全文搜索 |
| 📄 JSON 格式化 | 格式化、压缩、校验 JSON，支持语法高亮与可折叠树形视图 |
| 📝 Markdown 编辑器 | 双栏实时预览，支持代码块高亮，可打开/保存 `.md` 文件 |
| 🔐 密码生成器 | 可配置复杂度、长度、批量生成强密码 |
| 🗄️ SQL IN 构建器 | 从列值快速生成 SQL `IN(...)` 子句，自动去重与转义 |
| 📷 文本转二维码 | 将任意文本/URL 生成高清 PNG 二维码，支持自定义颜色 |
| 🔍 文件对比 | 双栏并排对比两份文本内容，支持行级与词级差异高亮 |
| 📦 JAR 查看器 | 浏览 JAR/ZIP 归档结构，反编译 `.class` 文件查看 Java 源码 |

---

## 🚀 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://www.rust-lang.org/) >= 1.77（通过 `rustup` 安装）
- [Tauri CLI 依赖](https://v2.tauri.app/start/prerequisites/)（平台系统依赖，参见官方文档）
- **JAR 查看器额外要求**：需要安装 [Java JDK/JRE](https://adoptium.net/)，并确保 `java` 命令在系统 PATH 中可用

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

## 📥 下载安装

前往 [Releases 页面](https://github.com/mayuanfei/mtool/releases) 下载最新版本：

- **macOS**：下载 `.dmg` 文件，双击安装
- **Windows**：下载 `.exe` 安装程序，运行即可

应用内置自动更新功能，后续版本会自动提示更新。

---

## 🛠️ 工具使用说明

### 📁 文件搜索

1. 进入「设置」页面，开启「File Search」开关，程序将在后台对选定目录建立全文索引。
2. 切换到「文件搜索」页面，在搜索框中输入查询语句，实时返回匹配的文件。
3. 点击结果可直接在系统文件管理器中定位该文件，或直接打开文件。

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
- **校验**：输入框实时标红非法 JSON，并显示错误位置和行号。
- **折叠/展开**：格式化后的 JSON 支持树形折叠，可以折叠/展开任意层级的对象和数组。
- **复制**：点击右上角「Copy」按钮，快速复制处理结果。

---

### 📝 Markdown 编辑器

- 左侧编辑区输入 Markdown 内容，右侧实时渲染预览。
- 支持代码块语法高亮（基于 highlight.js）。
- 支持从本地打开 `.md` 文件，以及将内容另存为文件。
- 支持同步滚动：编辑区和预览区联动滚动。

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

### 🔍 文件对比

基于 Myers diff 算法实现的双栏文本对比工具。

1. **加载文件**：通过以下三种方式加载要对比的文本：
   - 点击工具栏按钮选择本地文件
   - 直接将文件拖拽到左右两侧面板
   - 从剪贴板粘贴文本内容，或在文本框中直接输入/编辑
2. **查看差异**：加载两侧内容后，自动进行对比并展示结果：
   - 🔴 红色高亮标记删除的行
   - 🟢 绿色高亮标记新增的行
   - 修改行内的具体词级变更也会单独高亮
3. **导航差异**：使用工具栏的上下箭头在各差异区域间快速跳转，右侧 minimap 提供全局差异分布概览。
4. **辅助操作**：支持左右内容互换、重置、隐藏/显示输入面板。

> 支持拖入两个文件同时加载。超过 10,000 行的文件会自动切换为近似对比模式。

---

### 📦 JAR 查看器

浏览 JAR/ZIP 归档文件的内部结构，并自动反编译 `.class` 文件为可读的 Java 源码。

1. **打开文件**：点击「Open File」按钮或直接拖拽文件到界面。
2. **浏览结构**：左侧以树形目录展示归档内容，支持展开/折叠目录。
3. **查看内容**：
   - 点击 `.class` 文件 → 自动调用 CFR 反编译器还原为 Java 源码
   - 点击文本文件（`.xml`、`.properties`、`.yaml` 等）→ 直接展示内容并语法高亮
4. **支持的文件格式**：`.jar`、`.zip`、`.class` 以及常见文本文件格式。

> **前置要求**：反编译 `.class` 文件需要系统已安装 Java 运行环境。CFR 反编译工具已内置于应用中，无需额外下载。

---

## ⚙️ 设置

| 选项 | 说明 |
|------|------|
| 主题 | 在亮色（Light）和暗色（Dark）模式间切换 |
| 语言 | 切换界面语言（中文 / English） |
| 工具开关 | 独立启用/禁用各个工具模块，关闭后从侧边栏隐藏 |
| File Search | 开启后触发文件索引构建；关闭后清除索引数据 |
| 自动更新 | 启动时自动检测新版本，有更新时弹出提示 |

---

## 🗂️ 项目结构

```
mtool/
├── src/                  # 前端源码 (React + TypeScript)
│   ├── pages/            # 各工具页面组件
│   │   ├── FileSearch    # 文件搜索
│   │   ├── JsonFormatter # JSON 格式化
│   │   ├── MarkdownEditor# Markdown 编辑器
│   │   ├── PasswordGenerator # 密码生成器
│   │   ├── SqlInBuilder  # SQL IN 构建器
│   │   ├── TextToQr      # 文本转二维码
│   │   ├── FileDiff      # 文件对比
│   │   ├── JarViewer     # JAR 查看器
│   │   └── Settings      # 设置页面
│   ├── components/       # 公共 UI 组件
│   ├── App.tsx           # 应用主入口与路由状态
│   ├── theme.tsx         # 主题 Provider
│   └── i18n.ts           # 国际化
├── src-tauri/            # Tauri 后端 (Rust)
│   ├── src/
│   │   ├── lib.rs        # Tauri 命令注册
│   │   ├── file_search.rs# 文件索引引擎
│   │   └── jar_viewer.rs # JAR 查看与反编译
│   ├── resources/        # 内置资源 (CFR 反编译器)
│   └── tauri.conf.json   # Tauri 配置
└── package.json
```

---

## 🧱 技术栈

- **前端**：React 19 · TypeScript · Vite 7 · Tailwind CSS v4
- **后端**：Rust · Tauri v2
- **数据库**：SQLite (FTS5 全文索引) via `rusqlite`
- **差异对比**：Myers diff 算法（前端纯 TypeScript 实现）
- **反编译**：CFR (Class File Reader) — Java 字节码反编译器
- **二维码**：`qrcode` crate
- **Markdown**：`marked` + `highlight.js`

---

## 📄 License

MIT © [mayuanfei](https://github.com/mayuanfei)
