# Auto-Update Feature Design

**Date:** 2026-05-08  
**Status:** Approved  
**Approach:** tauri-plugin-updater + GitHub Releases + signed artifacts

---

## Goal

Add auto-update support to mtool so that:
- The app checks for updates on startup (when auto-update is enabled)
- Users can manually check for updates in Settings
- The update dialog displays the changelog for the new version
- Updates are downloaded in the background and installed with user confirmation

---

## Architecture

### Components Modified

| File | Change |
|------|--------|
| `src-tauri/Cargo.toml` | Add `tauri-plugin-updater` |
| `src-tauri/src/lib.rs` | Register updater plugin |
| `src-tauri/tauri.conf.json` | Add `plugins.updater` (endpoints + pubkey) |
| `src/hooks/useUpdater.ts` | New hook — encapsulates all update logic |
| `src/pages/Settings.tsx` | Add Updates card |
| `src/App.tsx` | Trigger auto-check on startup |
| `src/components/Sidebar.tsx` | Show orange dot when update available |
| `src/i18n.tsx` | Add update-related translation keys |
| `.github/workflows/release.yml` | Add signing env vars + upload latest.json |
| `.github/scripts/gen-latest-json.js` | New script — generates latest.json from build artifacts + CHANGELOG.md |

---

## Data Flow

```
App startup
  └─ useUpdater.init()
       ├─ if autoUpdate=true → checkForUpdate()
       │     ├─ fetch latest.json from GitHub Releases
       │     ├─ compare version (semver)
       │     └─ if newer → set hasUpdate=true, store UpdateInfo
       └─ expose: { hasUpdate, updateInfo, checking, downloading, progress,
                    checkForUpdate, startInstall, autoUpdate, setAutoUpdate }

Settings page
  └─ renders Updates card using useUpdater state
       ├─ Toggle: autoUpdate
       ├─ Display: current version
       ├─ Button: Check for Updates → checkForUpdate()
       └─ When hasUpdate: show version badge + Install button → startInstall()

Sidebar
  └─ receives hasUpdate prop
       └─ renders orange dot on Settings icon when true

Install flow
  └─ startInstall()
       ├─ open UpdateModal
       ├─ download() → progress events → progressbar
       └─ on complete → installAndRestart()
```

---

## Settings UI

New **Updates** card added after the Appearance card:

```
┌─────────────────────────────────────────────────────┐
│  🔄  Updates                                         │
├─────────────────────────────────────────────────────┤
│  Auto-update                    [Toggle]             │
│  Automatically check on startup                      │
├─────────────────────────────────────────────────────┤
│  Current version                v1.0.0               │
├─────────────────────────────────────────────────────┤
│  Check for Updates              [Button]             │
│  Status: Up to date / Checking... / v1.1.0 available │
└─────────────────────────────────────────────────────┘
```

When update available, the bottom row expands to show changelog preview and Install button.

---

## Update Install Modal

Shown when user clicks Install:

```
┌──────────────────────────────────┐
│  Update to v1.1.0                │
│                                  │
│  What's new:                     │
│  (markdown changelog content)    │
│                                  │
│  [████████░░] 80%                │
│                                  │
│  [Cancel]     [Install & Restart]│
└──────────────────────────────────┘
```

---

## Update Behavior

| Scenario | Behavior |
|----------|----------|
| Startup, auto-update ON | Silent background check; dot appears in sidebar if update found |
| Startup, auto-update OFF | No check |
| Manual check | Spinner → result shown in Settings card |
| Update found | Show version + changelog; wait for user to click Install |
| Installing | Modal with progress bar; cannot be dismissed mid-download |
| Install complete | Prompt to restart |

Updates **never** install silently — user confirmation is always required.

---

## latest.json Format (Tauri v2)

```json
{
  "version": "1.1.0",
  "notes": "### What's new\n- Fix: ...\n- Feature: ...",
  "pub_date": "2026-05-08T00:00:00Z",
  "platforms": {
    "darwin-universal": {
      "url": "https://github.com/mayuanfei/mtool/releases/download/v1.1.0/mtool_1.1.0_universal.app.tar.gz",
      "signature": "..."
    },
    "windows-x86_64": {
      "url": "https://github.com/mayuanfei/mtool/releases/download/v1.1.0/mtool_1.1.0_x64-setup.nsis.zip",
      "signature": "..."
    }
  }
}
```

---

## CI Changes

```yaml
env:
  TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
```

After build, macOS job runs `.github/scripts/gen-latest-json.js` which:
1. Reads `.sig` files from bundle output directories
2. Extracts the `[Unreleased]` or matching version section from `CHANGELOG.md`
3. Writes `latest.json` with correct platforms, signatures, and notes
4. Uploads `latest.json` alongside other release artifacts

---

## One-Time Key Setup (User Action Required)

```bash
# Run locally once to generate signing key pair
npx tauri signer generate -w ~/.tauri/mtool.key

# Output includes public key → paste into tauri.conf.json plugins.updater.pubkey
# Private key file → paste content into GitHub Secrets as TAURI_SIGNING_PRIVATE_KEY
```

---

## tauri.conf.json Addition

```json
{
  "plugins": {
    "updater": {
      "pubkey": "<PASTE_PUBLIC_KEY_HERE>",
      "endpoints": [
        "https://github.com/mayuanfei/mtool/releases/latest/download/latest.json"
      ],
      "windows": {
        "installMode": "passive"
      }
    }
  }
}
```

---

## Persistence

- `autoUpdate` preference stored in `localStorage` as `mtool_auto_update` (default: `true`)
