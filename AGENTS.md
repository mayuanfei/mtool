# Repo Notes

## Stack
- Single-package Tauri v2 desktop app with React 19 + Vite 7 + TypeScript.
- Tailwind CSS v4 is loaded from `src/index.css` via `@import "tailwindcss"`; there is no Tailwind config file.

## Source Layout
- Frontend entry: `src/main.tsx` -> `src/App.tsx`.
- UI is a single app shell with local `activePage` state in `src/App.tsx`; there is no router.
- Main frontend areas live in `src/pages/` and shared chrome is in `src/components/`.
- Tauri entry is `src-tauri/src/main.rs`, which delegates to `src-tauri/src/lib.rs`.
- Current Rust side has multiple native APIs (e.g. `format_json`, `minify_json`, `generate_qr`, `build_index`, `search_files`, `disable_file_search`, `reveal_in_explorer`, etc.) plus `tauri-plugin-opener`; consult `src-tauri/src/lib.rs` for the full list.

## Commands
- Install deps: `npm install`
- Frontend-only dev server: `npm run dev`
- Frontend production check: `npm run build`
- Tauri CLI entry: `npm run tauri`
- Desktop dev flow: `npm run tauri dev`
- Desktop production build: `npm run tauri build`

## Verification
- There are no repo scripts for lint, test, or separate typecheck. Use `npm run build` as the baseline verification step.
- `npm run build` runs `tsc && vite build`.
- TypeScript is strict and has `noUnusedLocals` + `noUnusedParameters`; unused React/icon imports currently fail builds quickly.

## Tauri / Vite Quirks
- `src-tauri/tauri.conf.json` already wires Tauri to the frontend: `beforeDevCommand` is `npm run dev`, `beforeBuildCommand` is `npm run build`.
- When validating the desktop app, prefer `npm run tauri dev` instead of manually running Vite and guessing ports.
- Vite dev server is fixed to port `1420` with `strictPort: true`; Tauri HMR uses `1421` when `TAURI_DEV_HOST` is set.
- Vite ignores `src-tauri/**` for file watching; Rust-side changes need Tauri/Rust-aware validation, not just the web dev server.

## Change Guidance
- Keep frontend changes aligned with the existing single-state page switch unless you are intentionally introducing routing.
- If you add a new native command, register it in `src-tauri/src/lib.rs` via `invoke_handler`; nothing else in the repo auto-discovers Rust commands.
