import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  FileText,
  Folder,
  FolderOpen,
  RefreshCw,
  RotateCcw,
  Search,
  Clock,
  X,
} from "lucide-react";
import { useI18n } from "../i18n";

// ---------------------------------------------------------------------------
// 类型定义
// ---------------------------------------------------------------------------

interface FileEntry {
  name: string;
  path: string;
  size: number;
  created: number;
  modified: number;
  is_dir: boolean;
  ext: string;
}

interface IndexStatus {
  total: number;
  is_indexing: boolean;
  last_built_at?: number;
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

function formatSize(bytes: number): string {
  if (bytes === 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatDate(ts: number): string {
  if (!ts) return "—";
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function tokenizeQuery(s: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let inQuote = false;
  for (const ch of s) {
    if (ch === '"' || ch === "'") {
      inQuote = !inQuote;
      current += ch;
    } else if ((ch === " " || ch === "\t") && !inQuote) {
      if (current) {
        tokens.push(current);
        current = "";
      }
    } else {
      current += ch;
    }
  }
  if (current) tokens.push(current);
  return tokens;
}

function stripQuotes(s: string): string {
  const t = s.trim();
  if (
    t.length >= 2 &&
    ((t.startsWith('"') && t.endsWith('"')) ||
      (t.startsWith("'") && t.endsWith("'")))
  ) {
    return t.slice(1, -1);
  }
  return t;
}

function HighlightedName({ name, query }: { name: string; query: string }) {
  const raw = tokenizeQuery(query)
    .find(
      (t) =>
        !t.startsWith("size:") &&
        !t.startsWith("content:") &&
        !t.includes("*") &&
        !t.includes("?") &&
        t.length > 0
    );
  const term = raw ? stripQuotes(raw).toLowerCase() : undefined;

  if (!term) return <span>{name}</span>;
  const idx = name.toLowerCase().indexOf(term);
  if (idx === -1) return <span>{name}</span>;

  return (
    <span>
      {name.slice(0, idx)}
      <mark className="bg-indigo-500/20 text-indigo-700 dark:bg-indigo-500/30 dark:text-indigo-300 rounded-sm px-0.5">
        {name.slice(idx, idx + term.length)}
      </mark>
      {name.slice(idx + term.length)}
    </span>
  );
}

// ---------------------------------------------------------------------------
// 主组件
// ---------------------------------------------------------------------------

export function FileSearch() {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<FileEntry[]>([]);
  const [status, setStatus] = useState<IndexStatus>({
    total: 0,
    is_indexing: false,
  });
  const [isSearching, setIsSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchElapsed, setSearchElapsed] = useState<number | null>(null);
  const [recentSearches, setRecentSearches] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem("mtool_filesearch_recent") || "[]");
    } catch {
      return [];
    }
  });
  const [showRecent, setShowRecent] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const searchIdRef = useRef(0);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const recentRef = useRef<HTMLDivElement>(null);

  // 初始化：获取索引状态，监听进度事件
  useEffect(() => {
    invoke<IndexStatus>("get_index_status").then(setStatus).catch(() => {});

    let cancelled = false;
    let unlistenProgress: (() => void) | null = null;
    let unlistenComplete: (() => void) | null = null;

    listen<number>("index_progress", (event) => {
      setStatus((prev) => ({ ...prev, is_indexing: true, total: event.payload }));
    }).then((fn) => {
      if (cancelled) fn();
      else unlistenProgress = fn;
    });

    listen<number>("index_complete", () => {
      invoke<IndexStatus>("get_index_status").then(setStatus).catch(() => {});
    }).then((fn) => {
      if (cancelled) fn();
      else unlistenComplete = fn;
    });

    return () => {
      cancelled = true;
      unlistenProgress?.();
      unlistenComplete?.();
    };
  }, []);

  // 搜索（带 300ms 防抖）
  const doSearch = useCallback((q: string) => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!q.trim()) {
      setResults([]);
      setSearchElapsed(null);
      return;
    }
    const id = ++searchIdRef.current;
    debounceRef.current = setTimeout(async () => {
      setIsSearching(true);
      setError(null);
      const t0 = performance.now();
      try {
        const res = await invoke<FileEntry[]>("search_files", {
          query: q,
          limit: 200,
        });
        // 丢弃已过期的请求结果（用户已输入新内容）
        if (searchIdRef.current !== id) return;
        setResults(res);
        setSearchElapsed(performance.now() - t0);
        // 保存到最近搜索（去重，最多 5 条）
        setRecentSearches((prev) => {
          const next = [q, ...prev.filter((s) => s !== q)].slice(0, 5);
          localStorage.setItem("mtool_filesearch_recent", JSON.stringify(next));
          return next;
        });
      } catch (e) {
        if (searchIdRef.current !== id) return;
        const errStr = String(e);
        setError(errStr === 'ERR_CONTENT_REQUIRES_FILENAME'
          ? t('Content search requires a filename or glob pattern, e.g. *.yml content:xxx')
          : errStr);
        setResults([]);
      } finally {
        if (searchIdRef.current === id) setIsSearching(false);
      }
    }, 300);
  }, []);

  useEffect(() => {
    doSearch(query);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [query, doSearch]);

  // 点击外部关闭最近搜索下拉
  useEffect(() => {
    if (!showRecent) return;
    const handle = (e: MouseEvent) => {
      if (
        searchInputRef.current &&
        !searchInputRef.current.contains(e.target as Node) &&
        recentRef.current &&
        !recentRef.current.contains(e.target as Node)
      ) {
        setShowRecent(false);
      }
    };
    document.addEventListener("mousedown", handle);
    return () => document.removeEventListener("mousedown", handle);
  }, [showRecent]);

  // 重建索引
  const handleRebuild = async () => {
    setError(null);
    setStatus((prev) => ({ ...prev, is_indexing: true, total: 0 }));
    try {
      await invoke<number>("build_index");
      const s = await invoke<IndexStatus>("get_index_status");
      setStatus(s);
    } catch (e) {
      setError(String(e));
      setStatus((prev) => ({ ...prev, is_indexing: false }));
    }
  };

  // 直接用系统默认程序打开文件 / 打开目录
  const handleOpen = async (entry: FileEntry) => {
    try {
      if (entry.is_dir) {
        await invoke("reveal_in_explorer", { path: entry.path });
      } else {
        await invoke("open_file", { path: entry.path });
      }
    } catch (_) {}
  };

  // 在文件管理器中定位（显示父目录并高亮该文件）
  const handleReveal = async (path: string) => {
    try {
      await invoke("reveal_in_explorer", { path });
    } catch (_) {}
  };

  const isIndexing = status.is_indexing;

  // 标题栏状态文案
  const selectRecent = (q: string) => {
    setQuery(q);
    setShowRecent(false);
    searchInputRef.current?.focus();
  };

  const removeRecent = (q: string) => {
    setRecentSearches((prev) => {
      const next = prev.filter((s) => s !== q);
      localStorage.setItem("mtool_filesearch_recent", JSON.stringify(next));
      return next;
    });
  };

  const renderIndexingBadge = () => {
    if (!isIndexing) {
      return (
        <>
          <span className="w-1.5 h-1.5 rounded-full bg-emerald-500" />
          <span>
            <span className="th-text-2 font-medium">
              {status.total.toLocaleString()}
            </span>
            &nbsp;{t('files indexed')}
          </span>
          {status.last_built_at && (
            <span className="th-text-3">
              · {t('last built')} {formatDate(status.last_built_at)}
            </span>
          )}
        </>
      );
    }
    return (
      <>
        <RefreshCw className="w-3 h-3 animate-spin text-indigo-400" />
        <span className="text-indigo-600 dark:text-indigo-300">
          {t('Indexing...')}&nbsp;
          <span className="font-mono tabular-nums text-indigo-400">
            {status.total.toLocaleString()}
          </span>
          &nbsp;{t('files indexed')}
        </span>
      </>
    );
  };

  return (
    <div className="w-full h-full flex flex-col">

      {/* 标题栏 */}
      <div className="flex justify-between items-center mb-6 border-b th-border pb-4">
        <h2 className="th-text font-semibold text-lg flex items-center gap-2">
          <Search className="w-5 h-5 text-indigo-400" />
          {t('File Search')}
        </h2>

        <div className="flex items-center gap-3">
          {/* 索引状态 */}
          <div className="flex items-center gap-2 text-xs th-text-3">
            {renderIndexingBadge()}
          </div>

          <button
            onClick={handleRebuild}
            disabled={isIndexing}
            className="px-3 py-1.5 text-xs th-bg-surface th-hover-surface th-text-2
              rounded font-medium border th-border-subtle transition-colors
              flex items-center gap-1.5 focus:outline-none
              disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <RotateCcw className="w-3 h-3" />
            {t('Re-index')}
          </button>
        </div>
      </div>

      {/* 搜索框 */}
      <div className="mb-4">
        <div className="relative">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 th-text-muted pointer-events-none" />
          <input
            ref={searchInputRef}
            type="text"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              if (showRecent) setShowRecent(false);
            }}
            onFocus={() => {
              if (!query.trim() && recentSearches.length > 0) {
                setShowRecent(true);
              }
            }}
            placeholder={t('Search file names...   e.g. *.yml   report draft   size:>100MB   content:"keyword"')}
            className="w-full pl-9 pr-10 py-2.5 th-bg-card border th-border-subtle
              rounded-lg text-sm th-text-2
              placeholder:text-slate-400 dark:placeholder:text-slate-500
              focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-indigo-500
              transition-colors font-mono"
          />
          {isSearching && (
            <RefreshCw className="absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-indigo-400 animate-spin" />
          )}

          {/* 最近搜索下拉 */}
          {showRecent && recentSearches.length > 0 && (
            <div
              ref={recentRef}
              className="absolute left-0 right-0 top-full mt-1 th-bg-card border th-border-subtle
                rounded-lg shadow-2xl z-50 overflow-hidden"
            >
              <div className="px-3 py-1.5 text-[10px] font-semibold th-text-muted uppercase tracking-wider">
                {t('Recent Searches')}
              </div>
              {recentSearches.map((s) => (
                <div
                  key={s}
                  onClick={() => selectRecent(s)}
                  className="flex items-center gap-2 px-3 py-2 text-sm th-text-2
                    hover:bg-indigo-500/10 hover:text-indigo-600 dark:hover:bg-indigo-900/30 dark:hover:text-indigo-300 cursor-pointer
                    transition-colors group"
                >
                  <Clock className="w-3.5 h-3.5 th-text-faint group-hover:text-indigo-400 shrink-0" />
                  <span className="flex-1 truncate font-mono text-xs">{s}</span>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      removeRecent(s);
                    }}
                    className="p-0.5 rounded th-text-ghost hover:th-text-3 th-hover-surface
                      opacity-0 group-hover:opacity-100 transition-all shrink-0"
                    title={t('Remove this record')}
                  >
                    <X className="w-3 h-3" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* 语法快捷按钮 */}
        <div className="flex gap-2 mt-2 flex-wrap">
          {[
            { label: "*.md", tip: t('glob match') },
            { label: "*.yml", tip: t('glob match') },
            { label: "size:>100MB", tip: t('size filter') },
            { label: 'content:"关键词"', tip: t('file content search') },
          ].map((item) => (
            <button
              key={item.label}
              onClick={() =>
                setQuery((q) => (q ? `${q} ${item.label}` : item.label))
              }
              title={item.tip}
              className="text-xs px-2 py-0.5 rounded th-bg-surface th-text-3
                hover:bg-indigo-500/10 hover:text-indigo-600 dark:hover:bg-indigo-900/50 dark:hover:text-indigo-300 font-mono
                border th-border-subtle transition-colors"
            >
              {item.label}
            </button>
          ))}
        </div>
      </div>

      {/* 错误提示 */}
      {error && (
        <div className="mb-3 px-3 py-2 bg-rose-900/30 border border-rose-700/50 rounded-lg text-sm text-rose-400">
          {error}
        </div>
      )}

      {/* 搜索结果区 */}
      <div className="flex-1 th-bg-card border th-border rounded-xl flex flex-col overflow-hidden shadow-2xl min-h-0">

        {/* 结果列表头 */}
        <div className="px-4 py-2 th-bg-surface-h border-b th-border flex items-center justify-between">
          <span className="text-[11px] font-bold th-text-3 uppercase tracking-tighter">
            {t('Search Results')}
          </span>
          {results.length > 0 && query.trim() && (
            <span className="text-[10px] th-text-muted font-mono">
              {results.length} {t('entries')}{results.length >= 200 && ` ${t('(first 200 shown)')}`}
              {searchElapsed !== null && (
                <span className="ml-2 th-text-muted">
                  · {searchElapsed < 1000
                    ? `${Math.round(searchElapsed)} ms`
                    : `${(searchElapsed / 1000).toFixed(2)} s`}
                </span>
              )}
            </span>
          )}
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          {/* 空态：无查询 */}
          {!query.trim() && !isIndexing && status.total === 0 && (
            <div className="flex flex-col items-center justify-center h-full th-text-faint">
              <Search className="w-10 h-10 mb-3 opacity-30" />
              <p className="text-sm">{t('No index data')}</p>
              <p className="text-xs mt-1">{t('Click "Re-index" to scan all system files')}</p>
            </div>
          )}

          {/* 空态：建索引中 */}
          {!query.trim() && isIndexing && (
            <div className="flex flex-col items-center justify-center h-full th-text-faint">
              <RefreshCw className="w-10 h-10 mb-3 opacity-30 animate-spin" />
              <p className="text-sm text-indigo-400">{t('Building index in background...')}</p>
              <p className="text-xs mt-1 font-mono tabular-nums">
                {status.total.toLocaleString()} {t('indexed so far, searchable when done')}
              </p>
            </div>
          )}

          {/* 空态：有索引但无查询 */}
          {!query.trim() && status.total > 0 && !isIndexing && (
            <div className="flex flex-col items-center justify-center h-full th-text-faint">
              <Search className="w-10 h-10 mb-3 opacity-30" />
              <p className="text-sm">{t('Enter keyword to search files')}</p>
              <p className="text-xs mt-1">
                {status.total.toLocaleString()} {t('files indexed')}
              </p>
            </div>
          )}

          {/* 无结果 */}
          {query.trim() && !isSearching && results.length === 0 && !error && (
            <div className="flex flex-col items-center justify-center h-full th-text-faint">
              <Search className="w-10 h-10 mb-3 opacity-30" />
              <p className="text-sm">{t('No matching files found')}</p>
            </div>
          )}

          {/* 结果列表 */}
          {results.length > 0 && (
            <div className="space-y-1.5">
              {results.map((entry) => (
                <div
                  key={entry.path}
                  onClick={() => handleOpen(entry)}
                  className="group th-bg-surface-h border th-border-muted rounded-lg px-4 py-3
                    th-hover-surface hover:border-indigo-500/50 cursor-pointer transition-all"
                >
                  {/* 文件名行 */}
                  <div className="flex items-center gap-2 mb-1">
                    {entry.is_dir ? (
                      <Folder className="w-4 h-4 text-indigo-400 shrink-0" />
                    ) : (
                      <FileText className="w-4 h-4 th-text-muted shrink-0 group-hover:th-text-3" />
                    )}
                    <span className="font-medium th-text-2 text-sm truncate">
                      <HighlightedName name={entry.name} query={query} />
                    </span>
                    {entry.ext && (
                      <span className="text-[10px] th-text-3 font-mono shrink-0 uppercase
                        th-bg-surface border th-border-subtle rounded px-1.5 py-0.5">
                        .{entry.ext}
                      </span>
                    )}
                    {/* 定位到文件夹按钮（仅文件时显示） */}
                    {!entry.is_dir && (
                      <button
                        onClick={(e) => { e.stopPropagation(); handleReveal(entry.path); }}
                        title={t('Reveal in file manager')}
                        className="ml-auto shrink-0 p-1 rounded th-text-faint hover:th-text-2
                          th-hover-surface opacity-0 group-hover:opacity-100 transition-all"
                      >
                        <FolderOpen className="w-3.5 h-3.5" />
                      </button>
                    )}
                  </div>

                  {/* 路径 */}
                  <div className="text-[11px] th-text-muted truncate pl-6 mb-2 font-mono">
                    {entry.path}
                  </div>

                  {/* 元数据行 */}
                  <div className="flex flex-wrap gap-x-4 gap-y-0.5 pl-6">
                    {!entry.is_dir && (
                      <span className="text-[11px] th-text-faint">
                        {t('Size')}:{" "}
                        <span className="th-text-3">{formatSize(entry.size)}</span>
                      </span>
                    )}
                    <span className="text-[11px] th-text-faint">
                      {t('Created')}:{" "}
                      <span className="th-text-3 font-mono">{formatDate(entry.created)}</span>
                    </span>
                    <span className="text-[11px] th-text-faint">
                      {t('Date Modified')}:{" "}
                      <span className="th-text-3 font-mono">{formatDate(entry.modified)}</span>
                    </span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* 底部状态栏 */}
      <footer className="h-8 border-t th-border mt-4 px-4 th-bg-card
        flex items-center justify-between text-[10px] th-text-muted rounded-b-xl shadow-inner" style={{ opacity: 0.8 }}>
        <div className="flex items-center gap-4">
          <span className="flex items-center gap-1.5">
            <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" />
            {t('System Ready')}
          </span>
          {query.trim() && (
            <span>{t('Results')}: {results.length}</span>
          )}
        </div>
        <div className="italic">
          MTOOL Desktop Tools {new Date().getFullYear()}
        </div>
      </footer>
    </div>
  );
}
