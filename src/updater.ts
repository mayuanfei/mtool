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
  installed: boolean;
  autoUpdate: boolean;
  setAutoUpdate: (v: boolean) => void;
  checkForUpdate: () => Promise<void>;
  startInstall: () => Promise<void>;
  doRelaunch: () => Promise<void>;
  dismissUpdate: () => void;
}

export function useUpdater(): UseUpdaterReturn {
  const [hasUpdate, setHasUpdate] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [installed, setInstalled] = useState(false);
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
      setInstalled(true);
    } catch (e) {
      setError(String(e));
      setDownloading(false);
    }
  }, [pendingUpdate]);

  const doRelaunch = useCallback(async () => {
    await relaunch();
  }, []);

  const dismissUpdate = useCallback(() => {
    setHasUpdate(false);
    setUpdateInfo(null);
    setPendingUpdate(null);
    setInstalled(false);
  }, []);

  return {
    hasUpdate, updateInfo, checking, downloading, progress, error, installed,
    autoUpdate, setAutoUpdate, checkForUpdate, startInstall, doRelaunch, dismissUpdate,
  };
}
