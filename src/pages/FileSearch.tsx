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

function HighlightedName({ name, query }: { name: string; query: string }) {
  const term = query
    .split(/\s+/)
    .find(
      (t) =>
        !t.startsWith("size:") &&
        !t.startsWith("content:") &&
        !t.includes("*") &&
        !t.includes("?") &&
        t.length > 0
    )
    ?.toLowerCase();

  if (!term) return <span>{name}</span>;
  const idx = name.toLowerCase().indexOf(term);
  if (idx === -1) return <span>{name}</span>;

  return (
    <span>
      {name.slice(0, idx)}
      <mark className="bg-indigo-500/30 text-indigo-300 rounded-sm px-0.5">
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

    const unlistenProgress = listen<number>("index_progress", (event) => {
      setStatus((prev) => ({ ...prev, is_indexing: true, total: event.payload }));
    });

    const unlistenComplete = listen<number>("index_complete", () => {
      invoke<IndexStatus>("get_index_status").then(setStatus).catch(() => {});
    });

    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
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
        setError(String(e));
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
            已索引&nbsp;
            <span className="text-slate-300 font-medium">
              {status.total.toLocaleString()}
            </span>
            &nbsp;个文件
          </span>
          {status.last_built_at && (
            <span className="text-slate-600">
              · 上次建立 {formatDate(status.last_built_at)}
            </span>
          )}
        </>
      );
    }
    return (
      <>
        <RefreshCw className="w-3 h-3 animate-spin text-indigo-400" />
        <span className="text-indigo-300">
          正在建立索引...&nbsp;
          <span className="font-mono tabular-nums text-indigo-400">
            {status.total.toLocaleString()}
          </span>
          &nbsp;个文件
        </span>
      </>
    );
  };

  return (
    <div className="w-full h-full flex flex-col">

      {/* 标题栏 */}
      <div className="flex justify-between items-center mb-6 border-b border-slate-800 pb-4">
        <h2 className="text-white font-semibold text-lg flex items-center gap-2">
          <Search className="w-5 h-5 text-indigo-400" />
          {t('File Search')}
        </h2>

        <div className="flex items-center gap-3">
          {/* 索引状态 */}
          <div className="flex items-center gap-2 text-xs text-slate-400">
            {renderIndexingBadge()}
          </div>

          <button
            onClick={handleRebuild}
            disabled={isIndexing}
            className="px-3 py-1.5 text-xs bg-slate-800 hover:bg-slate-700 text-slate-300
              rounded font-medium border border-slate-700 transition-colors
              flex items-center gap-1.5 focus:outline-none
              disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <RotateCcw className="w-3 h-3" />
            重建索引
          </button>
        </div>
      </div>

      {/* 搜索框 */}
      <div className="mb-4">
        <div className="relative">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-slate-500 pointer-events-none" />
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
            placeholder='搜索文件名...  例: *.yml   report draft   size:>100MB   content:"关键词"'
            className="w-full pl-9 pr-10 py-2.5 bg-slate-900 border border-slate-700
              rounded-lg text-sm text-slate-200 placeholder-slate-600
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
              className="absolute left-0 right-0 top-full mt-1 bg-slate-900 border border-slate-700
                rounded-lg shadow-2xl z-50 overflow-hidden"
            >
              <div className="px-3 py-1.5 text-[10px] font-semibold text-slate-500 uppercase tracking-wider">
                最近搜索
              </div>
              {recentSearches.map((s) => (
                <div
                  key={s}
                  onClick={() => selectRecent(s)}
                  className="flex items-center gap-2 px-3 py-2 text-sm text-slate-300
                    hover:bg-indigo-900/30 hover:text-indigo-300 cursor-pointer
                    transition-colors group"
                >
                  <Clock className="w-3.5 h-3.5 text-slate-600 group-hover:text-indigo-400 shrink-0" />
                  <span className="flex-1 truncate font-mono text-xs">{s}</span>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      removeRecent(s);
                    }}
                    className="p-0.5 rounded text-slate-700 hover:text-slate-400 hover:bg-slate-700
                      opacity-0 group-hover:opacity-100 transition-all shrink-0"
                    title="删除此记录"
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
            { label: "*.md", tip: "glob 匹配" },
            { label: "*.yml", tip: "glob 匹配" },
            { label: "size:>100MB", tip: "大小过滤" },
            { label: 'content:"关键词"', tip: "文件内容搜索" },
          ].map((item) => (
            <button
              key={item.label}
              onClick={() =>
                setQuery((q) => (q ? `${q} ${item.label}` : item.label))
              }
              title={item.tip}
              className="text-xs px-2 py-0.5 rounded bg-slate-800 text-slate-400
                hover:bg-indigo-900/50 hover:text-indigo-300 font-mono
                border border-slate-700 transition-colors"
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
      <div className="flex-1 bg-slate-900 border border-slate-800 rounded-xl flex flex-col overflow-hidden shadow-2xl min-h-0">

        {/* 结果列表头 */}
        <div className="px-4 py-2 bg-slate-800/50 border-b border-slate-800 flex items-center justify-between">
          <span className="text-[11px] font-bold text-slate-400 uppercase tracking-tighter">
            搜索结果
          </span>
          {results.length > 0 && query.trim() && (
            <span className="text-[10px] text-slate-500 font-mono">
              {results.length} 条{results.length >= 200 && "（仅显示前 200 条）"}
              {searchElapsed !== null && (
                <span className="ml-2 text-slate-600">
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
            <div className="flex flex-col items-center justify-center h-full text-slate-600">
              <Search className="w-10 h-10 mb-3 opacity-30" />
              <p className="text-sm">暂无索引数据</p>
              <p className="text-xs mt-1">点击「重建索引」开始扫描全系统文件</p>
            </div>
          )}

          {/* 空态：建索引中 */}
          {!query.trim() && isIndexing && (
            <div className="flex flex-col items-center justify-center h-full text-slate-600">
              <RefreshCw className="w-10 h-10 mb-3 opacity-30 animate-spin" />
              <p className="text-sm text-indigo-400">正在后台建立索引...</p>
              <p className="text-xs mt-1 font-mono tabular-nums">
                已建立 {status.total.toLocaleString()} 条，完成后可搜索
              </p>
            </div>
          )}

          {/* 空态：有索引但无查询 */}
          {!query.trim() && status.total > 0 && !isIndexing && (
            <div className="flex flex-col items-center justify-center h-full text-slate-600">
              <Search className="w-10 h-10 mb-3 opacity-30" />
              <p className="text-sm">输入关键词搜索文件</p>
              <p className="text-xs mt-1">
                已索引 {status.total.toLocaleString()} 个文件
              </p>
            </div>
          )}

          {/* 无结果 */}
          {query.trim() && !isSearching && results.length === 0 && !error && (
            <div className="flex flex-col items-center justify-center h-full text-slate-600">
              <Search className="w-10 h-10 mb-3 opacity-30" />
              <p className="text-sm">未找到匹配的文件</p>
            </div>
          )}

          {/* 结果列表 */}
          {results.length > 0 && (
            <div className="space-y-1.5">
              {results.map((entry, idx) => (
                <div
                  key={idx}
                  onClick={() => handleOpen(entry)}
                  className="group bg-slate-800/40 border border-slate-700/50 rounded-lg px-4 py-3
                    hover:bg-slate-800 hover:border-indigo-500/50 cursor-pointer transition-all"
                >
                  {/* 文件名行 */}
                  <div className="flex items-center gap-2 mb-1">
                    {entry.is_dir ? (
                      <Folder className="w-4 h-4 text-indigo-400 shrink-0" />
                    ) : (
                      <FileText className="w-4 h-4 text-slate-500 shrink-0 group-hover:text-slate-400" />
                    )}
                    <span className="font-medium text-slate-200 text-sm truncate">
                      <HighlightedName name={entry.name} query={query} />
                    </span>
                    {entry.ext && (
                      <span className="text-[10px] text-slate-600 font-mono shrink-0 uppercase
                        bg-slate-900 border border-slate-700 rounded px-1.5 py-0.5">
                        .{entry.ext}
                      </span>
                    )}
                    {/* 定位到文件夹按钮（仅文件时显示） */}
                    {!entry.is_dir && (
                      <button
                        onClick={(e) => { e.stopPropagation(); handleReveal(entry.path); }}
                        title="在文件管理器中定位"
                        className="ml-auto shrink-0 p-1 rounded text-slate-600 hover:text-slate-300
                          hover:bg-slate-700 opacity-0 group-hover:opacity-100 transition-all"
                      >
                        <FolderOpen className="w-3.5 h-3.5" />
                      </button>
                    )}
                  </div>

                  {/* 路径 */}
                  <div className="text-[11px] text-slate-500 truncate pl-6 mb-2 font-mono">
                    {entry.path}
                  </div>

                  {/* 元数据行 */}
                  <div className="flex flex-wrap gap-x-4 gap-y-0.5 pl-6">
                    {!entry.is_dir && (
                      <span className="text-[11px] text-slate-600">
                        大小:{" "}
                        <span className="text-slate-400">{formatSize(entry.size)}</span>
                      </span>
                    )}
                    <span className="text-[11px] text-slate-600">
                      创建:{" "}
                      <span className="text-slate-400 font-mono">{formatDate(entry.created)}</span>
                    </span>
                    <span className="text-[11px] text-slate-600">
                      修改:{" "}
                      <span className="text-slate-400 font-mono">{formatDate(entry.modified)}</span>
                    </span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* 底部状态栏 */}
      <footer className="h-8 border-t border-slate-800 mt-4 px-4 bg-slate-900/50
        flex items-center justify-between text-[10px] text-slate-500 rounded-b-xl shadow-inner">
        <div className="flex items-center gap-4">
          <span className="flex items-center gap-1.5">
            <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" />
            System Ready
          </span>
          {query.trim() && (
            <span>Results: {results.length}</span>
          )}
        </div>
        <div className="italic">
          MTOOL Desktop Tools {new Date().getFullYear()}
        </div>
      </footer>
    </div>
  );
}
