# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added
- **CI/CD**: GitHub Actions workflow for automatic macOS (Universal Binary) and Windows installer builds on tag push
- **JSON Formatter**: Full syntax highlighting — keys (blue), strings (green), numbers (red), booleans (purple), null (gray), with dark/light theme support
- **Markdown Editor**: Preview links now open in the system browser via `tauri-plugin-opener`; protocol whitelist restricts to `http://` and `https://` only

### Fixed
- **Markdown Editor**: HTML output sanitized with DOMPurify to prevent XSS
- **Markdown Editor**: File size capped at 10 MB for `open_md_file_by_path` (consistent with `open_md_file`)
- **JSON Formatter / Minifier**: Input capped at 5 MB to prevent UI freeze
- **JSON Formatter**: Number regex now uses negative lookbehind (`(?<!\d)`) to prevent false matches inside expressions like `1-2`
- **FileSearch**: `listen` event handler now cleans up correctly on unmount, eliminating listener leaks
- **TextToQr**: Copy and download failures now display a red error indicator instead of silently failing
- **TextToQr**: QR background color reads from CSS variable `--bg-card` to always match the active theme
- **SqlInBuilder**: Single and double quotes inside values are now correctly escaped
- **PasswordGenerator**: Slider and count input are clamped to `[1, 100]` in real time; localStorage state is also clamped on restore
- **Settings**: Disabling a tool while it is the active page now automatically navigates to Settings; index rebuild failures roll back the UI toggle state

### Changed
- Light mode UI contrast improved across FileSearch, JsonFormatter, SqlInBuilder, TextToQr, MarkdownEditor, and PasswordGenerator
- Placeholder text color standardized to `slate-400` (light) / `slate-500` (dark) across all input fields
- Custom slider styles (`.pwd-slider`, `.qr-slider`) extracted to `index.css` for reliable cross-browser rendering

---

## [1.0.0] — 2025-05-01

### Added
- **JSON Formatter**: Format and minify JSON with Rust backend; brace depth color coding; auto-format on paste; copy with feedback
- **Text to QR Code**: Generate QR codes with adjustable error correction and resolution; copy to clipboard and save as PNG
- **Password Generator**: Configurable length, character sets, custom symbols, exclusion filters, multi-generation, and history; settings persisted to localStorage
- **SQL IN Builder**: Build `IN (...)` clauses from pasted lists; configurable delimiter, quote style, and deduplication; duplicate detail panel
- **Markdown Editor**: Edit and preview Markdown side-by-side or full-screen; drag-and-drop file open; CSP-safe image loading
- **File Search**: Full-text search powered by SQLite FTS5; real-time filesystem watcher; content search with filename/glob filter; recent searches; Windows OneDrive and network drive support
- **Dark / Light Theme**: Global theme toggle persisted to localStorage; all tools support both themes
- **Internationalization**: English and Simplified Chinese (中文) with runtime switching
- **Window State**: Window position and size remembered across restarts via `tauri-plugin-window-state`
- **Settings Page**: Per-tool enable/disable toggles; sidebar visibility controlled by toggle state

### Fixed
- macOS production build CSP issue preventing external images from loading
- Windows: command prompt window no longer flashes when revealing files in Explorer
- Windows: OneDrive and network drive scan stability improvements
- FTS5 rebuild made atomic to prevent index corruption
- RwLock deadlock in auto-indexing path resolved
- WAL mode enabled to prevent first-launch UI freeze on Windows
- FTS5 readiness check optimized; mixed-script (CJK + Latin) search handled correctly

### Performance
- File indexing uses streaming to reduce memory usage
- SQLite physical file deletion used instead of `DROP TABLE` for faster index rebuilds
- Windows drive accessibility checks parallelized with a unified timeout
- FTS5 rebuild batched to avoid blocking the UI thread

---

[Unreleased]: https://github.com/mayuanfei/mtool/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/mayuanfei/mtool/releases/tag/v1.0.0
