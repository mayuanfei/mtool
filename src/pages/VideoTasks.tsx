import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  BookmarkPlus,
  Check,
  ExternalLink,
  Gauge,
  Link,
  Pencil,
  PictureInPicture2,
  Play,
  ShieldCheck,
  Square,
  Trash2,
  Video,
  Volume2,
  VolumeX,
  X,
} from 'lucide-react';
import { useI18n } from '../i18n';

interface Bookmark {
  id: string;
  name: string;
  url: string;
  createdAt: number;
}

interface WindowStatus {
  open: boolean;
  url: string | null;
}

const BOOKMARKS_KEY = 'mtool_video_task_bookmarks';
const SPEED_KEY = 'mtool_video_task_speed';
const MUTED_KEY = 'mtool_video_task_muted';
const COMPACT_KEY = 'mtool_video_task_compact';

function loadBookmarks(): Bookmark[] {
  try {
    const saved = localStorage.getItem(BOOKMARKS_KEY);
    if (!saved) return [];
    const value = JSON.parse(saved) as unknown;
    return Array.isArray(value) ? value.filter((item): item is Bookmark => (
      typeof item === 'object' && item !== null &&
      typeof (item as Bookmark).id === 'string' &&
      typeof (item as Bookmark).name === 'string' &&
      typeof (item as Bookmark).url === 'string'
    )) : [];
  } catch {
    return [];
  }
}

function normalizeUrl(value: string): string {
  const withProtocol = /^https?:\/\//i.test(value.trim()) ? value.trim() : `https://${value.trim()}`;
  const parsed = new URL(withProtocol);
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') throw new Error('invalid protocol');
  return parsed.toString();
}

export function VideoTasks() {
  const { t } = useI18n();
  const [url, setUrl] = useState('');
  const [bookmarkName, setBookmarkName] = useState('');
  const [bookmarks, setBookmarks] = useState<Bookmark[]>(loadBookmarks);
  const [speed, setSpeed] = useState(() => {
    const saved = Number(localStorage.getItem(SPEED_KEY));
    return Number.isFinite(saved) && saved >= 0.25 && saved <= 10 ? saved : 1;
  });
  const [muted, setMuted] = useState(() => localStorage.getItem(MUTED_KEY) === 'true');
  const [compact, setCompact] = useState(() => localStorage.getItem(COMPACT_KEY) === 'true');
  const [windowStatus, setWindowStatus] = useState<WindowStatus>({ open: false, url: null });
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingName, setEditingName] = useState('');

  const showMessage = useCallback((text: string, error = false) => {
    setMessage({ text, error });
    window.setTimeout(() => setMessage(null), 3500);
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      setWindowStatus(await invoke<WindowStatus>('get_video_task_window_status'));
    } catch {
      setWindowStatus({ open: false, url: null });
    }
  }, []);

  useEffect(() => {
    refreshStatus();
    window.addEventListener('focus', refreshStatus);
    return () => window.removeEventListener('focus', refreshStatus);
  }, [refreshStatus]);

  useEffect(() => {
    localStorage.setItem(BOOKMARKS_KEY, JSON.stringify(bookmarks));
  }, [bookmarks]);

  useEffect(() => {
    localStorage.setItem(SPEED_KEY, String(speed));
    invoke('set_video_task_speed', { speed }).catch(() => {});
  }, [speed]);

  useEffect(() => {
    localStorage.setItem(MUTED_KEY, String(muted));
    invoke('set_video_task_muted', { muted }).catch(() => {});
  }, [muted]);

  const currentHost = useMemo(() => {
    if (!windowStatus.url) return null;
    try {
      return new URL(windowStatus.url).host;
    } catch {
      return windowStatus.url;
    }
  }, [windowStatus.url]);

  const openVideo = async (target = url) => {
    let normalized: string;
    try {
      normalized = normalizeUrl(target);
    } catch {
      showMessage(t('Please enter a valid teaching URL.'), true);
      return;
    }

    setBusy(true);
    try {
      const status = await invoke<WindowStatus>('open_video_task_window', {
        url: normalized,
        speed,
        muted,
        compact,
      });
      setUrl(normalized);
      setWindowStatus(status);
      showMessage(t('Teaching site opened in the built-in browser.'));
    } catch (error) {
      showMessage(String(error), true);
    } finally {
      setBusy(false);
    }
  };

  const saveBookmark = () => {
    let normalized: string;
    try {
      normalized = normalizeUrl(url);
    } catch {
      showMessage(t('Please enter a valid teaching URL.'), true);
      return;
    }
    if (!bookmarkName.trim()) {
      showMessage(t('Please name this bookmark.'), true);
      return;
    }

    setBookmarks((items) => {
      const existing = items.find((item) => item.url === normalized);
      if (existing) {
        return items.map((item) => item.id === existing.id ? { ...item, name: bookmarkName.trim() } : item);
      }
      return [{ id: crypto.randomUUID(), name: bookmarkName.trim(), url: normalized, createdAt: Date.now() }, ...items];
    });
    setUrl(normalized);
    setBookmarkName('');
    showMessage(t('Bookmark saved.'));
  };

  const changeCompactMode = async () => {
    if (!windowStatus.open) {
      showMessage(t('Open a teaching site first.'), true);
      return;
    }
    const next = !compact;
    try {
      await invoke('set_video_task_compact', { compact: next });
      setCompact(next);
      localStorage.setItem(COMPACT_KEY, String(next));
      showMessage(next ? t('Focus mini window enabled.') : t('Full-size player restored.'));
    } catch (error) {
      showMessage(String(error), true);
    }
  };

  const closePlayer = async () => {
    try {
      await invoke('close_video_task_window');
      setWindowStatus({ open: false, url: null });
      showMessage(t('Player window closed.'));
    } catch (error) {
      showMessage(String(error), true);
    }
  };

  const restorePlayer = async () => {
    try {
      await invoke('restore_video_task_window');
      showMessage(t('Player window restored.'));
    } catch (error) {
      showMessage(String(error), true);
    }
  };

  const saveRename = (id: string) => {
    if (!editingName.trim()) return;
    setBookmarks((items) => items.map((item) => item.id === id ? { ...item, name: editingName.trim() } : item));
    setEditingId(null);
    setEditingName('');
  };

  const speedOptions = [1, 1.5, 2, 3, 5, 10];

  return (
    <div className="max-w-6xl mx-auto w-full">
      <div className="mb-8 flex flex-col lg:flex-row lg:items-start lg:justify-between gap-6">
        <div>
          <div className="flex items-center gap-3 mb-2">
            <div className="w-11 h-11 rounded-xl bg-indigo-500/15 border border-indigo-500/25 flex items-center justify-center">
              <Video className="w-6 h-6 text-indigo-400" />
            </div>
            <h1 className="text-4xl font-bold tracking-tight th-text">{t('Video Tasks')}</h1>
          </div>
          <p className="th-text-3">{t('Open authenticated teaching sites, save learning links, and control playback globally.')}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className={`px-3 py-2 rounded-lg border text-xs font-semibold flex items-center gap-2 ${windowStatus.open ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-400' : 'th-bg-card th-border th-text-muted'}`}>
            <span className={`w-2 h-2 rounded-full ${windowStatus.open ? 'bg-emerald-400 animate-pulse' : 'bg-slate-500'}`} />
            {windowStatus.open ? `${t('Player running')}${currentHost ? ` · ${currentHost}` : ''}` : t('Player closed')}
          </div>
          {windowStatus.open && (
            <button onClick={restorePlayer} className="px-3 py-2 rounded-lg border border-indigo-500/30 bg-indigo-500/10 text-indigo-300 hover:bg-indigo-500/20 text-xs font-semibold flex items-center gap-2 transition-colors">
              <ExternalLink className="w-3.5 h-3.5" />
              {t('Bring Player Back')}
            </button>
          )}
        </div>
      </div>

      {message && (
        <div className={`mb-5 px-4 py-3 rounded-xl border text-sm ${message.error ? 'bg-rose-500/10 border-rose-500/30 text-rose-300' : 'bg-emerald-500/10 border-emerald-500/30 text-emerald-300'}`}>
          {message.text}
        </div>
      )}

      <div className="grid grid-cols-1 xl:grid-cols-[1.4fr_0.9fr] gap-6 mb-6">
        <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl">
          <div className="px-6 py-4 border-b th-border flex items-center gap-3 th-bg-surface-h">
            <Link className="w-5 h-5 text-indigo-400" />
            <h2 className="text-sm font-bold tracking-tighter th-text-2 uppercase">{t('Teaching Website')}</h2>
          </div>
          <div className="p-6 space-y-5">
            <label className="block">
              <span className="block text-xs font-semibold th-text-3 mb-2">{t('Teaching URL')}</span>
              <div className="flex gap-3">
                <input
                  value={url}
                  onChange={(event) => setUrl(event.target.value)}
                  onKeyDown={(event) => { if (event.key === 'Enter') openVideo(); }}
                  placeholder="https://ysstudy.example.com/..."
                  className="flex-1 min-w-0 th-bg-input border th-border-subtle th-text-2 rounded-lg px-4 py-3 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500/50 focus:border-indigo-500 shadow-inner select-text"
                />
                <button
                  onClick={() => openVideo()}
                  disabled={busy}
                  className="px-5 py-3 bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white rounded-lg text-sm font-semibold flex items-center gap-2 transition-colors shadow-lg shadow-indigo-600/20"
                >
                  <ExternalLink className="w-4 h-4" />
                  {busy ? t('Opening...') : t('Open Browser')}
                </button>
              </div>
            </label>

            <div className="grid grid-cols-1 md:grid-cols-[1fr_auto] gap-3 items-end">
              <label className="block">
                <span className="block text-xs font-semibold th-text-3 mb-2">{t('Bookmark Name')}</span>
                <input
                  value={bookmarkName}
                  onChange={(event) => setBookmarkName(event.target.value)}
                  placeholder={t('e.g. 2026 Security Awareness Training')}
                  className="w-full th-bg-input border th-border-subtle th-text-2 rounded-lg px-4 py-3 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500/50 focus:border-indigo-500 shadow-inner select-text"
                />
              </label>
              <button onClick={saveBookmark} className="px-5 py-3 th-bg-input-alt border th-border-subtle th-text-2 rounded-lg text-sm font-semibold flex items-center justify-center gap-2 th-hover-surface transition-colors">
                <BookmarkPlus className="w-4 h-4 text-indigo-400" />
                {t('Save Bookmark')}
              </button>
            </div>
          </div>
        </section>

        <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl">
          <div className="px-6 py-4 border-b th-border flex items-center gap-3 th-bg-surface-h">
            <Gauge className="w-5 h-5 text-indigo-400" />
            <h2 className="text-sm font-bold tracking-tighter th-text-2 uppercase">{t('Global Playback Speed')}</h2>
          </div>
          <div className="p-6">
            <div className="flex items-end justify-between mb-4">
              <div>
                <div className="text-3xl font-bold th-text tabular-nums">{speed.toFixed(speed % 1 === 0 ? 0 : 2)}×</div>
                <p className="text-xs th-text-muted mt-1">{t('Applied to videos in the task browser')}</p>
              </div>
              <span className="text-[11px] text-amber-400 bg-amber-500/10 border border-amber-500/20 rounded-md px-2 py-1">{t('Maximum 10x')}</span>
            </div>
            <input
              type="range"
              min="0.25"
              max="10"
              step="0.25"
              value={speed}
              onChange={(event) => setSpeed(Number(event.target.value))}
              className="w-full accent-indigo-500 cursor-pointer"
            />
            <div className="grid grid-cols-6 gap-2 mt-4">
              {speedOptions.map((option) => (
                <button
                  key={option}
                  onClick={() => setSpeed(option)}
                  className={`py-2 rounded-lg text-xs font-semibold border transition-colors ${speed === option ? 'bg-indigo-600 border-indigo-500 text-white' : 'th-bg-input-alt th-border-subtle th-text-3 th-hover-surface'}`}
                >
                  {option}×
                </button>
              ))}
            </div>
            <div className="mt-5 pt-5 border-t th-border flex items-center justify-between gap-4">
              <div>
                <div className="text-sm font-semibold th-text-2">{t('Global Mute')}</div>
                <p className="text-xs th-text-muted mt-1">{t('Applied to all videos in the task browser')}</p>
              </div>
              <button
                onClick={() => setMuted((value) => !value)}
                className={`px-4 py-2.5 rounded-lg border text-sm font-semibold flex items-center gap-2 transition-colors ${muted ? 'bg-rose-500/15 border-rose-500/30 text-rose-300' : 'th-bg-input-alt th-border-subtle th-text-2 th-hover-surface'}`}
              >
                {muted ? <VolumeX className="w-4 h-4" /> : <Volume2 className="w-4 h-4" />}
                {muted ? t('Muted') : t('Sound On')}
              </button>
            </div>
          </div>
        </section>
      </div>

      <section className="mb-6 th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl">
        <div className="px-6 py-4 border-b th-border flex items-center gap-3 th-bg-surface-h">
          <ShieldCheck className="w-5 h-5 text-indigo-400" />
          <h2 className="text-sm font-bold tracking-tighter th-text-2 uppercase">{t('Background Playback Compatibility')}</h2>
        </div>
        <div className="px-6 py-5 flex flex-col lg:flex-row lg:items-center justify-between gap-5">
          <div className="flex items-start gap-4 max-w-3xl">
            <div className={`mt-0.5 w-10 h-10 rounded-lg flex items-center justify-center border ${compact ? 'bg-indigo-500/15 border-indigo-500/30' : 'th-bg-surface th-border-subtle'}`}>
              <PictureInPicture2 className={`w-5 h-5 ${compact ? 'text-indigo-400' : 'th-text-3'}`} />
            </div>
            <div>
              <h3 className="text-sm font-semibold th-text-2 mb-1">{t('Focus mini window')}</h3>
              <p className="text-sm th-text-muted leading-relaxed">{t('Keeps a small always-on-top WebView visible and filters common blur/visibility pause handlers. This is more reliable than true OS minimization, which may throttle the webpage.')}</p>
            </div>
          </div>
          <div className="flex items-center gap-3 shrink-0">
            <button onClick={changeCompactMode} className={`px-4 py-2.5 rounded-lg text-sm font-semibold flex items-center gap-2 border transition-colors ${compact ? 'bg-indigo-600 border-indigo-500 text-white' : 'th-bg-input-alt th-border-subtle th-text-2 th-hover-surface'}`}>
              <PictureInPicture2 className="w-4 h-4" />
              {compact ? t('Restore Full Window') : t('Enable Mini Window')}
            </button>
            {windowStatus.open && (
              <button onClick={closePlayer} className="px-4 py-2.5 rounded-lg text-sm font-semibold flex items-center gap-2 border border-rose-500/30 text-rose-300 hover:bg-rose-500/10 transition-colors">
                <Square className="w-4 h-4" />
                {t('Close Player')}
              </button>
            )}
          </div>
        </div>
      </section>

      <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl">
        <div className="px-6 py-4 border-b th-border flex items-center justify-between th-bg-surface-h">
          <div className="flex items-center gap-3">
            <BookmarkPlus className="w-5 h-5 text-indigo-400" />
            <h2 className="text-sm font-bold tracking-tighter th-text-2 uppercase">{t('Saved Learning Links')}</h2>
          </div>
          <span className="text-xs th-text-muted">{bookmarks.length} {t('items')}</span>
        </div>

        {bookmarks.length === 0 ? (
          <div className="py-14 text-center">
            <BookmarkPlus className="w-10 h-10 th-text-ghost mx-auto mb-3" />
            <p className="text-sm th-text-muted">{t('No teaching links saved yet.')}</p>
          </div>
        ) : (
          <div className="divide-y th-divide">
            {bookmarks.map((bookmark) => (
              <div key={bookmark.id} className="px-6 py-4 flex items-center gap-4 th-hover-surface transition-colors">
                <button onClick={() => openVideo(bookmark.url)} className="w-10 h-10 rounded-lg bg-indigo-500/10 border border-indigo-500/20 flex items-center justify-center text-indigo-400 hover:bg-indigo-500/20 transition-colors shrink-0" title={t('Open Browser')}>
                  <Play className="w-4 h-4 fill-current" />
                </button>
                <div className="min-w-0 flex-1">
                  {editingId === bookmark.id ? (
                    <div className="flex items-center gap-2">
                      <input
                        autoFocus
                        value={editingName}
                        onChange={(event) => setEditingName(event.target.value)}
                        onKeyDown={(event) => { if (event.key === 'Enter') saveRename(bookmark.id); }}
                        className="w-full max-w-md th-bg-input border th-border-subtle th-text-2 rounded-md px-3 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500/50 select-text"
                      />
                      <button onClick={() => saveRename(bookmark.id)} className="p-2 text-emerald-400 hover:bg-emerald-500/10 rounded-md"><Check className="w-4 h-4" /></button>
                      <button onClick={() => setEditingId(null)} className="p-2 th-text-muted th-hover-surface rounded-md"><X className="w-4 h-4" /></button>
                    </div>
                  ) : (
                    <>
                      <div className="text-sm font-semibold th-text-2 truncate">{bookmark.name}</div>
                      <div className="text-xs th-text-muted truncate mt-1 select-text">{bookmark.url}</div>
                    </>
                  )}
                </div>
                {editingId !== bookmark.id && (
                  <div className="flex items-center gap-1 shrink-0">
                    <button onClick={() => { setEditingId(bookmark.id); setEditingName(bookmark.name); }} className="p-2 th-text-muted hover:text-indigo-400 th-hover-surface rounded-md transition-colors" title={t('Rename')}>
                      <Pencil className="w-4 h-4" />
                    </button>
                    <button onClick={() => setBookmarks((items) => items.filter((item) => item.id !== bookmark.id))} className="p-2 th-text-muted hover:text-rose-400 hover:bg-rose-500/10 rounded-md transition-colors" title={t('Delete')}>
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
