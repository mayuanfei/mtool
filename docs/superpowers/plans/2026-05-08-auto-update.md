# Auto-Update Feature Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add tauri-plugin-updater-based auto-update to mtool, with a Settings UI showing changelog, download progress, and install confirmation.

**Architecture:** Rust registers `tauri-plugin-updater` + `tauri-plugin-process`; a `useUpdater` hook on the frontend wraps `check()` / `downloadAndInstall()`; Settings page gains an Updates card; Sidebar shows a dot indicator when an update is available; CI generates and uploads `latest.json` on every tag push.

**Tech Stack:** tauri-plugin-updater v2, tauri-plugin-process v2, @tauri-apps/plugin-updater, @tauri-apps/plugin-process, Node.js (gen-latest-json.js script), GitHub Actions

---

## Chunk 1: Rust & Config Infrastructure

### Task 1: Add Rust dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add tauri-plugin-updater and tauri-plugin-process to Cargo.toml**

In the `[dependencies]` section, add:

```toml
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
```

- [ ] **Step 2: Verify Cargo resolves correctly**

```bash
cd src-tauri && cargo fetch
```

Expected: no errors (dependencies download successfully).

---

### Task 2: Register plugins in lib.rs

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Register both plugins in the Tauri builder**

Find the `tauri::Builder::default()` chain in `lib.rs` (search for `.invoke_handler`). Add `.plugin(tauri_plugin_updater::Builder::new().build())` and `.plugin(tauri_plugin_process::init())` before `.invoke_handler(...)`.

The builder chain should look like:

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_opener::init())
    .plugin(tauri_plugin_window_state::Builder::default().build())
    .plugin(tauri_plugin_updater::Builder::new().build())
    .plugin(tauri_plugin_process::init())
    .setup(|app| { ... })
    .invoke_handler(tauri::generate_handler![...])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```

---

### Task 3: Configure tauri.conf.json

**Files:**
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Add updater plugin config**

Add a `"plugins"` key at the top level of `tauri.conf.json` (alongside `"bundle"`, `"app"`, etc.):

```json
"plugins": {
  "updater": {
    "pubkey": "PLACEHOLDER_REPLACE_WITH_REAL_PUBKEY",
    "endpoints": [
      "https://github.com/mayuanfei/mtool/releases/latest/download/latest.json"
    ],
    "windows": {
      "installMode": "passive"
    }
  }
}
```

> **Note:** `PLACEHOLDER_REPLACE_WITH_REAL_PUBKEY` must be replaced with the real public key after running the key generation step (see Chunk 4 Task 11). The build will succeed with the placeholder — the updater will simply fail gracefully at runtime until the real key is set.

---

## Chunk 2: Frontend Core

### Task 4: Install npm packages

**Files:**
- Modify: `package.json` (auto-updated by npm)

- [ ] **Step 1: Install updater and process plugins**

```bash
npm install @tauri-apps/plugin-updater @tauri-apps/plugin-process
```

Expected: both packages added to `dependencies` in `package.json`.

---

### Task 5: Create useUpdater hook

**Files:**
- Create: `src/updater.ts`

- [ ] **Step 1: Create the hook**

```typescript
import { useState, useCallback } from 'react';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

export interface UpdateInfo {
  version: string;
  notes: string;
  date: string;
}

export interface UseUpdaterReturn {
  hasUpdate: boolean;
  updateInfo: UpdateInfo | null;
  checking: boolean;
  downloading: boolean;
  progress: number;
  error: string | null;
  autoUpdate: boolean;
  setAutoUpdate: (v: boolean) => void;
  checkForUpdate: () => Promise<void>;
  startInstall: () => Promise<void>;
  dismissUpdate: () => void;
}

export function useUpdater(): UseUpdaterReturn {
  const [hasUpdate, setHasUpdate] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);

  const [autoUpdate, setAutoUpdateState] = useState<boolean>(() => {
    const saved = localStorage.getItem('mtool_auto_update');
    return saved !== null ? saved === 'true' : true;
  });

  const setAutoUpdate = useCallback((v: boolean) => {
    setAutoUpdateState(v);
    localStorage.setItem('mtool_auto_update', v.toString());
  }, []);

  const checkForUpdate = useCallback(async () => {
    if (import.meta.env.DEV) return;
    setChecking(true);
    setError(null);
    try {
      const update = await check();
      if (update?.available) {
        setHasUpdate(true);
        setPendingUpdate(update);
        setUpdateInfo({
          version: update.version,
          notes: update.body ?? '',
          date: update.date ?? '',
        });
      } else {
        setHasUpdate(false);
        setUpdateInfo(null);
        setPendingUpdate(null);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setChecking(false);
    }
  }, []);

  const startInstall = useCallback(async () => {
    if (!pendingUpdate) return;
    setDownloading(true);
    setProgress(0);
    setError(null);
    let downloaded = 0;
    let total = 0;
    try {
      await pendingUpdate.downloadAndInstall((event) => {
        if (event.event === 'Started') {
          total = event.data.contentLength ?? 0;
        } else if (event.event === 'Progress') {
          downloaded += event.data.chunkLength;
          setProgress(total > 0 ? Math.round((downloaded / total) * 100) : 50);
        }
      });
      setProgress(100);
      await relaunch();
    } catch (e) {
      setError(String(e));
      setDownloading(false);
    }
  }, [pendingUpdate]);

  const dismissUpdate = useCallback(() => {
    setHasUpdate(false);
    setUpdateInfo(null);
    setPendingUpdate(null);
  }, []);

  return {
    hasUpdate, updateInfo, checking, downloading, progress, error,
    autoUpdate, setAutoUpdate, checkForUpdate, startInstall, dismissUpdate,
  };
}
```

- [ ] **Step 2: Verify TypeScript compiles**

```bash
export PATH="/opt/homebrew/bin:$PATH" && npm run build
```

Expected: build passes (or only pre-existing warnings).

---

### Task 6: Add i18n keys

**Files:**
- Modify: `src/i18n.tsx`

- [ ] **Step 1: Add English keys**

In the `en` object, add after the Settings section:

```typescript
// Updates
'Updates': 'Updates',
'Auto-update': 'Auto-update',
'Automatically check for updates on startup.': 'Automatically check for updates on startup.',
'Current version': 'Current version',
'Check for Updates': 'Check for Updates',
'Checking...': 'Checking...',
'Up to date': 'Up to date',
'available': 'available',
'Install & Restart': 'Install & Restart',
'Downloading...': 'Downloading...',
'Update error': 'Update error',
'What\'s new in': 'What\'s new in',
'Cancel': 'Cancel',
```

- [ ] **Step 2: Add Chinese keys**

In the `zh` object, add the same keys with Chinese values:

```typescript
// Updates
'Updates': '更新',
'Auto-update': '自动更新',
'Automatically check for updates on startup.': '启动时自动检查更新。',
'Current version': '当前版本',
'Check for Updates': '检查更新',
'Checking...': '检查中...',
'Up to date': '已是最新版本',
'available': '可更新',
'Install & Restart': '安装并重启',
'Downloading...': '下载中...',
'Update error': '更新出错',
'What\'s new in': '新版本内容',
'Cancel': '取消',
```

---

### Task 7: Create UpdateModal component

**Files:**
- Create: `src/components/UpdateModal.tsx`

- [ ] **Step 1: Create the modal**

```tsx
import { X } from 'lucide-react';
import { useI18n } from '../i18n';
import type { UpdateInfo } from '../updater';

interface UpdateModalProps {
  open: boolean;
  updateInfo: UpdateInfo;
  downloading: boolean;
  progress: number;
  error: string | null;
  onClose: () => void;
  onInstall: () => void;
}

export function UpdateModal({ open, updateInfo, downloading, progress, error, onClose, onInstall }: UpdateModalProps) {
  const { t } = useI18n();
  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="th-bg-card border th-border rounded-2xl shadow-2xl w-full max-w-lg mx-4 overflow-hidden">
        {/* Header */}
        <div className="px-6 py-4 border-b th-border flex items-center justify-between th-bg-surface-h">
          <h2 className="text-base font-semibold th-text">
            {t('What\'s new in')} v{updateInfo.version}
          </h2>
          {!downloading && (
            <button onClick={onClose} className="th-text-muted hover:th-text-2 transition-colors rounded p-1 th-hover-surface">
              <X className="w-4 h-4" />
            </button>
          )}
        </div>

        {/* Changelog */}
        <div className="px-6 py-4 max-h-64 overflow-y-auto">
          {updateInfo.notes ? (
            <pre className="text-sm th-text-3 whitespace-pre-wrap font-sans leading-relaxed">
              {updateInfo.notes}
            </pre>
          ) : (
            <p className="text-sm th-text-muted italic">No changelog provided.</p>
          )}
        </div>

        {/* Progress */}
        {downloading && (
          <div className="px-6 pb-4">
            <div className="w-full bg-slate-200 dark:bg-slate-700 rounded-full h-2 overflow-hidden">
              <div
                className="bg-indigo-500 h-2 rounded-full transition-all duration-300"
                style={{ width: `${progress}%` }}
              />
            </div>
            <p className="text-xs th-text-muted mt-2 text-center">
              {t('Downloading...')} {progress}%
            </p>
          </div>
        )}

        {/* Error */}
        {error && (
          <div className="px-6 pb-4">
            <p className="text-xs text-red-400">{t('Update error')}: {error}</p>
          </div>
        )}

        {/* Actions */}
        <div className="px-6 py-4 border-t th-border flex justify-end gap-3 th-bg-surface-h">
          {!downloading && (
            <button
              onClick={onClose}
              className="px-4 py-2 text-sm th-text-3 th-bg-input-alt th-border-subtle border rounded-lg th-hover-surface transition-colors"
            >
              {t('Cancel')}
            </button>
          )}
          <button
            onClick={onInstall}
            disabled={downloading}
            className="px-4 py-2 text-sm font-medium bg-indigo-600 hover:bg-indigo-700 disabled:opacity-60 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
          >
            {downloading ? `${t('Downloading...')} ${progress}%` : t('Install & Restart')}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify TypeScript compiles**

```bash
export PATH="/opt/homebrew/bin:$PATH" && npm run build
```

---

## Chunk 3: UI Integration

### Task 8: Update Settings.tsx — add Updates card

**Files:**
- Modify: `src/pages/Settings.tsx`

- [ ] **Step 1: Import useUpdater, UpdateModal, and RefreshCw icon**

Add to the imports at the top of `Settings.tsx`:

```typescript
import { Globe, Wrench, Palette, RefreshCw } from 'lucide-react';
import { useUpdater } from '../updater';
import { UpdateModal } from '../components/UpdateModal';
```

- [ ] **Step 2: Call useUpdater inside the component and add modal state**

At the top of `SettingsPage` component body (after existing state), add:

```typescript
const { hasUpdate, updateInfo, checking, downloading, progress, error,
        autoUpdate, setAutoUpdate, checkForUpdate, startInstall, dismissUpdate } = useUpdater();
const [showModal, setShowModal] = useState(false);
const [lastCheckStatus, setLastCheckStatus] = useState<'idle'|'ok'|'error'>('idle');

const handleCheck = async () => {
  await checkForUpdate();
  setLastCheckStatus('ok');
};
```

- [ ] **Step 3: Add Updates card JSX after the Appearance card**

Add this new `<section>` after the closing `</section>` of the Appearance card, and before the closing `</div>` of `space-y-6`:

```tsx
{/* Updates Card */}
<section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl">
  <div className="px-6 py-4 border-b th-border flex items-center gap-3 th-bg-surface-h">
    <RefreshCw className="w-5 h-5 text-indigo-400" />
    <h2 className="text-sm font-bold tracking-tighter th-text-2 uppercase">{t('Updates')}</h2>
  </div>

  <div className="divide-y th-divide">
    {/* Auto-update toggle */}
    <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
      <div>
        <p className="text-base font-medium th-text-2 mb-1">{t('Auto-update')}</p>
        <p className="text-sm th-text-muted">{t('Automatically check for updates on startup.')}</p>
      </div>
      <Toggle checked={autoUpdate} onChange={() => setAutoUpdate(!autoUpdate)} />
    </div>

    {/* Current version */}
    <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
      <p className="text-base font-medium th-text-2">{t('Current version')}</p>
      <span className="text-sm font-mono th-text-muted">v1.0.0</span>
    </div>

    {/* Check for updates row */}
    <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
      <div>
        <p className="text-base font-medium th-text-2 mb-1">{t('Check for Updates')}</p>
        <p className="text-sm th-text-muted">
          {checking
            ? t('Checking...')
            : hasUpdate && updateInfo
            ? <span className="text-amber-400 font-medium">v{updateInfo.version} {t('available')}</span>
            : lastCheckStatus === 'ok'
            ? t('Up to date')
            : null}
        </p>
      </div>
      <div className="flex items-center gap-3">
        {hasUpdate && updateInfo && (
          <button
            onClick={() => setShowModal(true)}
            className="px-3 py-1.5 text-sm font-medium bg-indigo-600 hover:bg-indigo-700 text-white rounded-lg transition-colors"
          >
            {t('Install & Restart')}
          </button>
        )}
        <button
          onClick={handleCheck}
          disabled={checking}
          className="px-3 py-1.5 text-sm th-text-3 th-bg-input-alt th-border-subtle border rounded-lg th-hover-surface transition-colors disabled:opacity-50 flex items-center gap-2"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${checking ? 'animate-spin' : ''}`} />
          {checking ? t('Checking...') : t('Check for Updates')}
        </button>
      </div>
    </div>
  </div>
</section>

{/* Update install modal */}
{showModal && updateInfo && (
  <UpdateModal
    open={showModal}
    updateInfo={updateInfo}
    downloading={downloading}
    progress={progress}
    error={error}
    onClose={() => { if (!downloading) { setShowModal(false); dismissUpdate(); } }}
    onInstall={startInstall}
  />
)}
```

- [ ] **Step 4: Verify TypeScript compiles**

```bash
export PATH="/opt/homebrew/bin:$PATH" && npm run build
```

---

### Task 9: Update App.tsx — auto-check on startup + pass hasUpdate to Sidebar

**Files:**
- Modify: `src/App.tsx`

- [ ] **Step 1: Import useUpdater**

Add to imports:

```typescript
import { useUpdater } from './updater';
```

- [ ] **Step 2: Call useUpdater in App component and auto-check**

After the existing `useState` declarations, add:

```typescript
const { hasUpdate, checkForUpdate, autoUpdate } = useUpdater();

useEffect(() => {
  if (autoUpdate) {
    checkForUpdate();
  }
}, []); // eslint-disable-line react-hooks/exhaustive-deps
```

- [ ] **Step 3: Pass hasUpdate to Sidebar**

Change the `<Sidebar ...>` call to include `hasUpdate`:

```tsx
<Sidebar
  activePage={activePage}
  onNavigate={setActivePage}
  jsonEnabled={jsonEnabled}
  qrEnabled={qrEnabled}
  pwdEnabled={pwdEnabled}
  sqlInEnabled={sqlInEnabled}
  mdEnabled={mdEnabled}
  fileSearchEnabled={fileSearchEnabled}
  hasUpdate={hasUpdate}
/>
```

---

### Task 10: Update Sidebar.tsx — dot indicator

**Files:**
- Modify: `src/components/Sidebar.tsx`

- [ ] **Step 1: Add hasUpdate to SidebarProps interface**

```typescript
interface SidebarProps {
  // ... existing props ...
  hasUpdate: boolean;
}
```

- [ ] **Step 2: Destructure hasUpdate from props**

```typescript
export function Sidebar({ activePage, onNavigate, jsonEnabled, qrEnabled, pwdEnabled, sqlInEnabled, mdEnabled, fileSearchEnabled, hasUpdate }: SidebarProps) {
```

- [ ] **Step 3: Update the NavItem for Settings in bottomItems to show the dot**

The `bottomItems` array renders using `NavItem`. Replace the `NavItem` component's button content to support a dot indicator for Settings. Modify the `NavItem` component inside `Sidebar` to accept an optional `showDot` prop, or handle it inline for the settings item.

Simplest approach: render the Settings bottom item manually with the dot, instead of mapping through `bottomItems`. Replace the bottom items render section:

```tsx
{/* Bottom items */}
<div className="mt-auto p-3 space-y-1">
  {!collapsed && (
    <div className="text-[10px] font-bold th-text-muted uppercase tracking-widest px-3 mb-2">{t('System')}</div>
  )}
  {/* Settings with optional dot */}
  <button
    onClick={() => onNavigate('settings')}
    title={collapsed ? t('Settings') : undefined}
    className={`w-full flex items-center ${collapsed ? 'justify-center' : ''} gap-3 px-3 py-2 rounded-md font-medium transition-colors relative ${
      activePage === 'settings'
        ? 'bg-indigo-600/10 text-indigo-400 border border-indigo-500/20 shadow-sm'
        : 'th-text-3 th-hover-surface border border-transparent'
    }`}
  >
    <span className="relative flex-shrink-0">
      <Settings className="w-[18px] h-[18px]" strokeWidth={2} />
      {hasUpdate && (
        <span className="absolute -top-1 -right-1 w-2 h-2 bg-amber-400 rounded-full" />
      )}
    </span>
    {!collapsed && <span className="text-[13px] truncate">{t('Settings')}</span>}
  </button>
  {/* User */}
  <NavItem item={{ id: 'user', label: t('User'), icon: User }} />
</div>
```

- [ ] **Step 4: Verify TypeScript compiles**

```bash
export PATH="/opt/homebrew/bin:$PATH" && npm run build
```

Expected: clean build (no new errors).

---

## Chunk 4: CI + Key Setup

### Task 11: One-time key generation (USER ACTION REQUIRED)

> **This task requires manual action from the developer. It cannot be automated.**

- [ ] **Step 1: Generate the signing key pair**

Run this command in your terminal:

```bash
npx tauri signer generate -w ~/.tauri/mtool.key
```

This outputs:
- A public key string (starts with `dW5...`)
- A private key file at `~/.tauri/mtool.key`

- [ ] **Step 2: Copy the public key into tauri.conf.json**

Replace `PLACEHOLDER_REPLACE_WITH_REAL_PUBKEY` in `src-tauri/tauri.conf.json` with the actual public key output from the previous command.

- [ ] **Step 3: Add private key to GitHub Secrets**

1. Go to `https://github.com/mayuanfei/mtool/settings/secrets/actions`
2. Click **New repository secret**
3. Name: `TAURI_SIGNING_PRIVATE_KEY`
4. Value: contents of `~/.tauri/mtool.key`
5. If the key has a password, also add `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

---

### Task 12: Create gen-latest-json.js script

**Files:**
- Create: `.github/scripts/gen-latest-json.js`

- [ ] **Step 1: Create the script**

```javascript
#!/usr/bin/env node
/**
 * Generates latest.json for Tauri v2 updater.
 * Usage: node gen-latest-json.js <version> <repo> <platform>
 *   version: e.g. "1.1.0" (without "v" prefix)
 *   repo:    e.g. "mayuanfei/mtool"
 *   platform: "darwin" or "windows"
 */
const fs = require('fs');
const path = require('path');

const [,, version, repo, platform] = process.argv;
if (!version || !repo || !platform) {
  console.error('Usage: gen-latest-json.js <version> <repo> <platform>');
  process.exit(1);
}

const tag = `v${version}`;
const baseUrl = `https://github.com/${repo}/releases/download/${tag}`;

// Find .sig files
function findSig(dir) {
  if (!fs.existsSync(dir)) return null;
  const files = fs.readdirSync(dir);
  const sig = files.find(f => f.endsWith('.sig'));
  if (!sig) return null;
  return { name: sig.replace('.sig', ''), sig: fs.readFileSync(path.join(dir, sig), 'utf8').trim() };
}

// Extract changelog section for this version from CHANGELOG.md
function extractChangelog(version) {
  const changelogPath = path.join(__dirname, '../../CHANGELOG.md');
  if (!fs.existsSync(changelogPath)) return '';
  const content = fs.readFileSync(changelogPath, 'utf8');
  // Match [Unreleased] or [version] section
  const patterns = [
    new RegExp(`## \\[${version}\\][^\\n]*\\n([\\s\\S]*?)(?=\\n## \\[|$)`),
    /## \[Unreleased\][^\n]*\n([\s\S]*?)(?=\n## \[|$)/,
  ];
  for (const pattern of patterns) {
    const match = content.match(pattern);
    if (match) return match[1].trim();
  }
  return '';
}

// Build platforms object based on current platform
const platforms = {};

if (platform === 'darwin') {
  const dmgSigDir = `src-tauri/target/universal-apple-darwin/release/bundle/macos`;
  const info = findSig(dmgSigDir);
  if (info) {
    platforms['darwin-universal'] = {
      url: `${baseUrl}/${info.name}`,
      signature: info.sig,
    };
  }
  // Also check aarch64 and x86_64 fallbacks
  ['aarch64', 'x86_64'].forEach(arch => {
    const dir = `src-tauri/target/${arch}-apple-darwin/release/bundle/macos`;
    const fi = findSig(dir);
    if (fi) {
      platforms[`darwin-${arch}`] = { url: `${baseUrl}/${fi.name}`, signature: fi.sig };
    }
  });
}

if (platform === 'windows') {
  const nsisDir = `src-tauri/target/release/bundle/nsis`;
  const info = findSig(nsisDir);
  if (info) {
    platforms['windows-x86_64'] = {
      url: `${baseUrl}/${info.name}`,
      signature: info.sig,
    };
  }
}

const existing = fs.existsSync('latest.json')
  ? JSON.parse(fs.readFileSync('latest.json', 'utf8'))
  : {};

const output = {
  version,
  notes: extractChangelog(version),
  pub_date: new Date().toISOString(),
  platforms: { ...(existing.platforms || {}), ...platforms },
};

fs.writeFileSync('latest.json', JSON.stringify(output, null, 2));
console.log(`Generated latest.json for v${version} (${platform})`);
console.log('Platforms:', Object.keys(output.platforms).join(', '));
```

> **Note:** The script merges both `darwin` and `windows` platforms into a single `latest.json`. The macOS job runs first and creates the file; the Windows job runs in parallel. The `softprops/action-gh-release` action handles concurrent uploads correctly.

---

### Task 13: Update release.yml

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: Rewrite release.yml with signing + latest.json generation**

```yaml
name: Build and Release Tauri App

on:
  push:
    tags:
      - 'v*'

jobs:
  build:
    permissions:
      contents: write

    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: macos-latest
            args: '--target universal-apple-darwin'
            gen_platform: darwin

          - platform: windows-latest
            args: ''
            gen_platform: windows

    runs-on: ${{ matrix.platform }}

    env:
      TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}

    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm

      - uses: dtolnay/rust-toolchain@stable

      - name: Add macOS universal targets
        if: matrix.platform == 'macos-latest'
        run: rustup target add aarch64-apple-darwin x86_64-apple-darwin

      - uses: swatinem/rust-cache@v2

      - run: npm install

      - run: npm run tauri build -- ${{ matrix.args }}

      - name: Generate latest.json
        run: |
          VERSION="${{ github.ref_name }}"
          VERSION="${VERSION#v}"
          node .github/scripts/gen-latest-json.js "$VERSION" "mayuanfei/mtool" "${{ matrix.gen_platform }}"
        shell: bash

      - uses: softprops/action-gh-release@v2
        with:
          files: |
            latest.json
            src-tauri/target/**/bundle/**/*.dmg
            src-tauri/target/**/bundle/**/*.msi
            src-tauri/target/**/bundle/**/*.exe
            src-tauri/target/**/bundle/**/*.tar.gz
            src-tauri/target/**/bundle/**/*.sig
```

> **Why upload `.tar.gz` and `.sig`?** The Tauri v2 updater downloads the `.tar.gz` (macOS) or `.nsis.zip` (Windows) file referenced in `latest.json`. These files must be present in the GitHub Release.

- [ ] **Step 2: Verify YAML is valid**

```bash
cat .github/workflows/release.yml
```

Expected: file content visible, no YAML syntax issues.

---

### Task 14: Final build verification

- [ ] **Step 1: Run full build**

```bash
export PATH="/opt/homebrew/bin:$PATH" && npm run build
```

Expected: clean build, no TypeScript errors.

- [ ] **Step 2: Commit all changes**

```bash
git add \
  src-tauri/Cargo.toml \
  src-tauri/Cargo.lock \
  src-tauri/src/lib.rs \
  src-tauri/tauri.conf.json \
  src/updater.ts \
  src/i18n.tsx \
  src/components/UpdateModal.tsx \
  src/pages/Settings.tsx \
  src/App.tsx \
  src/components/Sidebar.tsx \
  .github/workflows/release.yml \
  .github/scripts/gen-latest-json.js \
  package.json \
  package-lock.json \
  docs/superpowers/specs/2026-05-08-auto-update-design.md \
  docs/superpowers/plans/2026-05-08-auto-update.md
git commit -m "feat: add auto-update with tauri-plugin-updater and Settings UI"
```

---

## Summary of Files Changed

| File | Action |
|------|--------|
| `src-tauri/Cargo.toml` | Add tauri-plugin-updater, tauri-plugin-process |
| `src-tauri/src/lib.rs` | Register both plugins |
| `src-tauri/tauri.conf.json` | Add plugins.updater config |
| `package.json` | Add @tauri-apps/plugin-updater, @tauri-apps/plugin-process |
| `src/updater.ts` | New — useUpdater hook |
| `src/i18n.tsx` | Add update translation keys |
| `src/components/UpdateModal.tsx` | New — install dialog |
| `src/pages/Settings.tsx` | Add Updates card |
| `src/App.tsx` | Auto-check on startup, pass hasUpdate to Sidebar |
| `src/components/Sidebar.tsx` | Add hasUpdate prop + dot indicator |
| `.github/workflows/release.yml` | Add signing env + latest.json upload |
| `.github/scripts/gen-latest-json.js` | New — generate latest.json |
