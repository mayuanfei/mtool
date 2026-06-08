import { useState, useEffect, useMemo, useRef, useCallback } from 'react';
import { FileUp, FileArchive, Folder, File as FileIcon, FileJson, FileCode2, ChevronRight, ChevronDown, ChevronUp, Package, Search, X } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { useI18n } from '../i18n';
import hljs from 'highlight.js';

interface TreeNode {
  name: string;
  path: string;
  isDir: boolean;
  children: Record<string, TreeNode>;
}

function buildTree(paths: string[]): TreeNode {
  const root: TreeNode = { name: '', path: '', isDir: true, children: {} };
  
  for (const p of paths) {
    if (!p) continue;
    const parts = p.split('/').filter(Boolean);
    let current = root;
    let currentPath = '';
    
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      const isDir = i < parts.length - 1 || p.endsWith('/');
      currentPath = currentPath ? `${currentPath}/${part}` : part;
      
      if (!current.children[part]) {
        current.children[part] = {
          name: part,
          path: isDir ? currentPath + '/' : currentPath,
          isDir,
          children: {}
        };
      }
      current = current.children[part];
    }
  }
  return root;
}

function getLanguage(fileName: string): string {
  const ext = fileName.split('.').pop()?.toLowerCase() || '';
  if (ext === 'class' || ext === 'java') return 'java';
  if (ext === 'json') return 'json';
  if (ext === 'yaml' || ext === 'yml') return 'yaml';
  if (ext === 'xml') return 'xml';
  if (ext === 'md') return 'markdown';
  if (['js', 'jsx', 'ts', 'tsx'].includes(ext)) return 'javascript';
  if (['properties', 'ini', 'conf'].includes(ext)) return 'properties';
  if (ext === 'sql') return 'sql';
  if (['sh', 'bash'].includes(ext)) return 'bash';
  return 'plaintext';
}

function getIconForFile(name: string) {
  const ext = name.split('.').pop()?.toLowerCase() || '';
  if (ext === 'class' || ext === 'java') return <FileCode2 className="w-4 h-4 text-emerald-500" />;
  if (ext === 'json' || ext === 'yaml' || ext === 'yml') return <FileJson className="w-4 h-4 text-amber-500" />;
  return <FileIcon className="w-4 h-4 text-slate-400" />;
}

function getSearchTokens(query: string): string[] {
  return query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
}

function matchesEntryPath(path: string, query: string): boolean {
  const normalizedPath = path.toLowerCase();
  const tokens = getSearchTokens(query);
  return tokens.every(token => normalizedPath.includes(token));
}

function HighlightedText({ text, query }: { text: string; query: string }) {
  const normalizedText = text.toLowerCase();
  const token = getSearchTokens(query).find(term => normalizedText.includes(term));

  if (!token) return <span>{text}</span>;

  const start = normalizedText.indexOf(token);
  const end = start + token.length;

  return (
    <span>
      {text.slice(0, start)}
      <mark className="rounded-sm bg-indigo-500/20 px-0.5 text-indigo-700 dark:bg-indigo-500/30 dark:text-indigo-300">
        {text.slice(start, end)}
      </mark>
      {text.slice(end)}
    </span>
  );
}

function highlightSearchInHtml(html: string, query: string, activeIndex: number): { html: string; count: number } {
  const needle = query.trim().toLowerCase();
  if (!html || !needle || typeof DOMParser === 'undefined') {
    return { html, count: 0 };
  }

  const parser = new DOMParser();
  const doc = parser.parseFromString(`<div>${html}</div>`, 'text/html');
  const root = doc.body.firstElementChild;
  if (!root) return { html, count: 0 };

  const walker = doc.createTreeWalker(root, 4);
  const textNodes: Text[] = [];
  let currentNode = walker.nextNode();

  while (currentNode) {
    textNodes.push(currentNode as Text);
    currentNode = walker.nextNode();
  }

  let count = 0;

  for (const node of textNodes) {
    const text = node.nodeValue || '';
    const lower = text.toLowerCase();
    if (!lower.includes(needle)) continue;

    const fragment = doc.createDocumentFragment();
    let cursor = 0;
    let matchIndex = lower.indexOf(needle, cursor);

    while (matchIndex !== -1) {
      if (matchIndex > cursor) {
        fragment.append(doc.createTextNode(text.slice(cursor, matchIndex)));
      }

      const mark = doc.createElement('mark');
      mark.dataset.jarContentMatch = String(count);
      mark.className = count === activeIndex
        ? 'rounded-sm bg-orange-500 px-0.5 text-white ring-1 ring-orange-300 dark:bg-orange-400 dark:text-slate-950'
        : 'rounded-sm bg-amber-300/70 px-0.5 text-slate-950 dark:bg-amber-300/75 dark:text-slate-950';
      mark.textContent = text.slice(matchIndex, matchIndex + needle.length);
      fragment.append(mark);

      count += 1;
      cursor = matchIndex + needle.length;
      matchIndex = lower.indexOf(needle, cursor);
    }

    if (cursor < text.length) {
      fragment.append(doc.createTextNode(text.slice(cursor)));
    }

    node.parentNode?.replaceChild(fragment, node);
  }

  return { html: root.innerHTML, count };
}

// hljs.highlight 会转义 HTML 特殊字符，但 plaintext 分支和异常兜底分支
// 会直接将原始字符串注入 dangerouslySetInnerHTML，需手动转义防止 XSS
function escapeHtml(str: string): string {
  return str
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}

export function JarViewer() {
  const { t } = useI18n();
  const [filePath, setFilePath] = useState('');
  const [isJar, setIsJar] = useState(false);
  const [tree, setTree] = useState<TreeNode | null>(null);
  const [entryPaths, setEntryPaths] = useState<string[]>([]);
  const [selectedEntry, setSelectedEntry] = useState<string | null>(null);
  const [content, setContent] = useState('');
  const [loading, setLoading] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [searchQuery, setSearchQuery] = useState('');
  const [contentSearchOpen, setContentSearchOpen] = useState(false);
  const [contentSearchQuery, setContentSearchQuery] = useState('');
  const [contentSearchIndex, setContentSearchIndex] = useState(0);
  
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    try {
      const saved = localStorage.getItem('mtool_jarviewer_width');
      const w = saved ? parseInt(saved, 10) : 256;
      return isNaN(w) ? 256 : Math.max(150, Math.min(800, w));
    } catch {
      return 256;
    }
  });
  const sidebarWidthRef = useRef(sidebarWidth);
  const loadIdRef = useRef(0);
  const contentSearchInputRef = useRef<HTMLInputElement>(null);
  const contentScrollRef = useRef<HTMLDivElement>(null);

  const handleMouseDownResizer = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = sidebarWidthRef.current;
    
    const onMouseMove = (e: MouseEvent) => {
      const newWidth = Math.max(150, Math.min(800, startWidth + (e.clientX - startX)));
      sidebarWidthRef.current = newWidth;
      setSidebarWidth(newWidth);
    };
    
    const onMouseUp = () => {
      document.removeEventListener('mousemove', onMouseMove);
      document.removeEventListener('mouseup', onMouseUp);
      document.body.style.cursor = '';
      try {
        localStorage.setItem('mtool_jarviewer_width', sidebarWidthRef.current.toString());
      } catch {}
    };
    
    document.addEventListener('mousemove', onMouseMove);
    document.addEventListener('mouseup', onMouseUp);
    document.body.style.cursor = 'col-resize';
  };

  const handleOpenFile = async () => {
    try {
      const result = await invoke<[string, string]>('open_jar_or_class');
      const [path, ext] = result;
      await loadFile(path, ext);
    } catch {
      // User cancelled or error
    }
  };

  const loadFile = async (path: string, ext: string) => {
    setFilePath(path);
    const isArchive = ['jar', 'zip'].includes(ext.toLowerCase());
    setIsJar(isArchive);
    setSelectedEntry(null);
    setContent('');
    setExpandedDirs(new Set());
    setEntryPaths([]);
    setSearchQuery('');
    setContentSearchOpen(false);
    setContentSearchQuery('');
    setContentSearchIndex(0);
    
    if (isArchive) {
      try {
        setLoading(true);
        const entries = await invoke<string[]>('list_jar_entries', { path });
        const root = buildTree(entries);
        setEntryPaths(entries);
        setTree(root);
      } catch (err) {
        setTree(null);
        setContent(`Error: ${err}`);
      } finally {
        setLoading(false);
      }
    } else {
      const fileName = path.split(/[/\\]/).pop() || path;
      const root = buildTree([fileName]);
      setEntryPaths([fileName]);
      setTree(root);
      // Automatically load the single file
      await loadEntryContent(path, '', false);
    }
  };

  const loadFileRef = useRef(loadFile);
  useEffect(() => {
    loadFileRef.current = loadFile;
  });

  useEffect(() => {
    const setup = async () => {
      const unlisten = await getCurrentWebview().onDragDropEvent(async (event) => {
        if (event.payload.type === 'drop') {
          setDragOver(false);
          const paths = event.payload.paths;
          if (!paths || paths.length === 0) return;
          const path = paths[0];
          const ext = path.split('.').pop()?.toLowerCase() || '';
          await loadFileRef.current(path, ext);
        } else if (event.payload.type === 'enter' || event.payload.type === 'over') {
          setDragOver(true);
        } else if (event.payload.type === 'leave') {
          setDragOver(false);
        }
      });
      return unlisten;
    };
    const cleanup = setup();
    return () => { cleanup.then(fn => fn()); };
  }, []);

  const loadEntryContent = useCallback(async (basePath: string, entryPath: string, isJarEntry: boolean) => {
    setSelectedEntry(entryPath || basePath);
    setLoading(true);
    setContent('');
    setContentSearchIndex(0);
    const currentId = ++loadIdRef.current;
    try {
      let result = '';
      if (isJarEntry) {
        result = await invoke<string>('read_jar_entry', { jarPath: basePath, entryName: entryPath });
      } else {
        result = await invoke<string>('read_local_class', { path: basePath });
      }
      if (loadIdRef.current !== currentId) return;
      setContent(result);
    } catch (err) {
      if (loadIdRef.current !== currentId) return;
      setContent(`Error reading file: ${err}`);
    } finally {
      if (loadIdRef.current === currentId) {
        setLoading(false);
      }
    }
  }, []);

  const toggleDir = useCallback((path: string) => {
    setExpandedDirs(prev => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const { highlightedContent, lineCount } = useMemo(() => {
    if (!content) return { highlightedContent: '', lineCount: 0 };

    const lineCount = content.split('\n').length;
    const lang = getLanguage(selectedEntry || filePath);
    try {
      if (lang === 'plaintext') {
        return { highlightedContent: escapeHtml(content), lineCount };
      }
      return { highlightedContent: hljs.highlight(content, { language: lang, ignoreIllegals: true }).value, lineCount };
    } catch {
      return { highlightedContent: escapeHtml(content), lineCount };
    }
  }, [content, selectedEntry, filePath]);

  const { html: searchableContent, count: contentSearchMatchCount } = useMemo(() => {
    if (!contentSearchOpen || !contentSearchQuery.trim()) {
      return { html: highlightedContent, count: 0 };
    }
    return highlightSearchInHtml(highlightedContent, contentSearchQuery, contentSearchIndex);
  }, [highlightedContent, contentSearchOpen, contentSearchQuery, contentSearchIndex]);

  const goToContentSearchMatch = useCallback((direction: 1 | -1) => {
    if (contentSearchMatchCount === 0) return;
    setContentSearchIndex(prev => (
      prev + direction + contentSearchMatchCount
    ) % contentSearchMatchCount);
  }, [contentSearchMatchCount]);

  useEffect(() => {
    setContentSearchIndex(0);
  }, [contentSearchQuery, selectedEntry, content]);

  useEffect(() => {
    if (contentSearchMatchCount === 0) {
      if (contentSearchIndex !== 0) setContentSearchIndex(0);
      return;
    }
    if (contentSearchIndex >= contentSearchMatchCount) {
      setContentSearchIndex(0);
    }
  }, [contentSearchIndex, contentSearchMatchCount]);

  useEffect(() => {
    if (!contentSearchOpen) return;
    const frameId = window.requestAnimationFrame(() => {
      contentSearchInputRef.current?.focus();
      contentSearchInputRef.current?.select();
    });
    return () => window.cancelAnimationFrame(frameId);
  }, [contentSearchOpen]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f') {
        if (!filePath || (isJar && !selectedEntry)) return;
        event.preventDefault();
        setContentSearchOpen(true);
      } else if (contentSearchOpen && event.key === 'Escape') {
        event.preventDefault();
        setContentSearchOpen(false);
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [contentSearchOpen, filePath, isJar, selectedEntry]);

  useEffect(() => {
    if (!contentSearchOpen || !contentSearchQuery.trim() || contentSearchMatchCount === 0) return;
    const frameId = window.requestAnimationFrame(() => {
      const currentMatch = contentScrollRef.current?.querySelector<HTMLElement>(
        `mark[data-jar-content-match="${contentSearchIndex}"]`
      );
      currentMatch?.scrollIntoView({ block: 'center', inline: 'nearest' });
    });
    return () => window.cancelAnimationFrame(frameId);
  }, [contentSearchIndex, contentSearchMatchCount, contentSearchOpen, contentSearchQuery, searchableContent]);

  const trimmedSearchQuery = searchQuery.trim();
  const matchingEntryPaths = useMemo(() => {
    if (!trimmedSearchQuery) return entryPaths;
    return entryPaths.filter(path => matchesEntryPath(path, trimmedSearchQuery));
  }, [entryPaths, trimmedSearchQuery]);

  const displayedTree = useMemo(() => {
    if (!tree) return null;
    if (!trimmedSearchQuery) return tree;
    return buildTree(matchingEntryPaths);
  }, [tree, matchingEntryPaths, trimmedSearchQuery]);

  const matchedEntryCount = useMemo(() => {
    return matchingEntryPaths.filter(path => path && !path.endsWith('/')).length;
  }, [matchingEntryPaths]);

  // Memoize tree rendering to prevent lag on large jars during sidebar resize
  const renderedTree = useMemo(() => {
    if (!displayedTree) return null;
    const isFiltering = trimmedSearchQuery.length > 0;

    const renderNode = (node: TreeNode, depth = 0): React.ReactNode => {
      const entries = Object.values(node.children).sort((a, b) => {
        if (a.isDir && !b.isDir) return -1;
        if (!a.isDir && b.isDir) return 1;
        return a.name.localeCompare(b.name);
      });

      return entries.map(child => {
        const isExpanded = isFiltering || expandedDirs.has(child.path);
        const isSelected = selectedEntry === child.path;

        if (child.isDir) {
          return (
            <div key={child.path}>
              <div 
                className={`flex items-center gap-1.5 px-2 py-1 cursor-pointer select-none text-xs th-text-2 th-hover-surface transition-colors`}
                style={{ paddingLeft: `${depth * 12 + 8}px` }}
                onClick={() => toggleDir(child.path)}
              >
                <div className="w-4 h-4 flex items-center justify-center text-slate-400">
                  {isExpanded ? <ChevronDown className="w-3.5 h-3.5" /> : <ChevronRight className="w-3.5 h-3.5" />}
                </div>
                <Folder className="w-4 h-4 text-blue-400 flex-shrink-0" fill="currentColor" fillOpacity={0.2} />
                <span className="truncate" title={child.path}>
                  <HighlightedText text={child.name} query={trimmedSearchQuery} />
                </span>
              </div>
              {isExpanded && renderNode(child, depth + 1)}
            </div>
          );
        }

        return (
          <div 
            key={child.path}
            className={`flex items-center gap-2 px-2 py-1 cursor-pointer select-none text-xs transition-colors ${
              isSelected ? 'bg-indigo-500/15 text-indigo-400 font-medium' : 'th-text-2 th-hover-surface'
            }`}
            style={{ paddingLeft: `${depth * 12 + 28}px` }}
            onClick={() => loadEntryContent(filePath, child.path, isJar)}
          >
            {getIconForFile(child.name)}
            <span className="truncate" title={child.path}>
              <HighlightedText text={child.name} query={trimmedSearchQuery} />
            </span>
          </div>
        );
      });
    };

    return renderNode(displayedTree);
  }, [displayedTree, trimmedSearchQuery, expandedDirs, selectedEntry, filePath, isJar, toggleDir, loadEntryContent]);

  if (!filePath) {
    return (
      <div 
        className={`flex flex-col h-full w-full items-center justify-center border-2 border-dashed ${dragOver ? 'border-indigo-500 bg-indigo-500/5' : 'th-border th-bg-surface-h'} transition-all m-0 rounded-none`}
      >
        <Package className={`w-16 h-16 mb-4 ${dragOver ? 'text-indigo-400' : 'th-text-muted'}`} />
        <h2 className="text-lg font-bold th-text mb-2">{t('Jar Viewer & Decompiler')}</h2>
        <p className="text-sm th-text-muted mb-6 text-center max-w-sm">
          {t('Drag and drop a .jar, .class, or any text file here, or click the button below to browse.')}
        </p>
        <button
          onClick={handleOpenFile}
          className="flex items-center gap-2 px-4 py-2 bg-indigo-600 hover:bg-indigo-700 text-white rounded-md text-sm font-medium transition-colors"
        >
          <FileUp className="w-4 h-4" />
          {t('Open File')}
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full w-full overflow-hidden">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-4 py-2 border-b th-border th-bg-surface flex-shrink-0">
        <div className="flex items-center gap-2 overflow-hidden flex-1">
          <div className="p-1.5 rounded-md bg-indigo-500/10 text-indigo-500 flex-shrink-0">
            {isJar ? <FileArchive className="w-4 h-4" /> : <FileCode2 className="w-4 h-4" />}
          </div>
          <h1 className="text-sm font-bold th-text truncate" title={filePath}>
            {filePath.split(/[/\\]/).pop()}
          </h1>
          <span className="text-xs th-text-muted truncate min-w-0" title={filePath}>
            {filePath}
          </span>
        </div>

        <button
          onClick={handleOpenFile}
          className="ml-4 px-3 py-1.5 text-xs rounded th-text-3 th-hover-surface transition-colors border th-border flex items-center gap-1.5"
        >
          <FileUp className="w-3.5 h-3.5" />
          {t('Open Another')}
        </button>
      </div>

      <div className="flex flex-1 min-h-0">
        {/* Left Tree Panel */}
        <div 
          className="border-r th-border flex flex-col flex-shrink-0 th-bg-surface-h"
          style={{ width: `${sidebarWidth}px` }}
        >
          <div className="px-3 py-2 border-b th-border bg-black/5">
            <div className="flex items-center justify-between gap-2 text-xs font-semibold th-text-muted uppercase tracking-wider">
              <span className="truncate">{isJar ? t('Archive Contents') : t('File')}</span>
              {isJar && trimmedSearchQuery && (
                <span className="shrink-0 normal-case tracking-normal th-text-3">
                  {t('{count} matches', { count: matchedEntryCount })}
                </span>
              )}
            </div>
            {isJar && (
              <div className="relative mt-2">
                <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 th-text-muted pointer-events-none" />
                <input
                  type="text"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  placeholder={t('Search archive entries...')}
                  className="h-8 w-full rounded-md border th-border-subtle th-bg-input py-1 pl-8 pr-8 text-xs th-text-2 placeholder:text-slate-400 dark:placeholder:text-slate-500 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                />
                {searchQuery && (
                  <button
                    type="button"
                    onClick={() => setSearchQuery('')}
                    className="absolute right-1.5 top-1/2 flex h-5 w-5 -translate-y-1/2 items-center justify-center rounded th-text-muted th-hover-surface hover:text-indigo-400 transition-colors"
                    title={t('Clear')}
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>
            )}
          </div>
          <div className="flex-1 overflow-y-auto py-1">
            {trimmedSearchQuery && matchingEntryPaths.length === 0 ? (
              <div className="flex h-full flex-col items-center justify-center px-3 text-center text-xs th-text-faint">
                <Search className="mb-2 h-8 w-8 opacity-30" />
                {t('No matching archive entries')}
              </div>
            ) : (
              renderedTree
            )}
          </div>
        </div>

        {/* Resizer Handle */}
        <div 
          className="w-1 cursor-col-resize hover:bg-indigo-500/50 active:bg-indigo-500 transition-colors z-10"
          onMouseDown={handleMouseDownResizer}
        />

        {/* Right Content Panel */}
        <div className="flex-1 flex flex-col min-w-0 th-bg-main">
          {(selectedEntry || !isJar) && (
            <div className="px-4 py-2 border-b th-border text-xs th-text-muted flex items-center gap-2 th-bg-surface">
              <span className="truncate min-w-0" title={selectedEntry ? selectedEntry : filePath.split(/[/\\]/).pop()}>
                {selectedEntry ? selectedEntry : filePath.split(/[/\\]/).pop()}
              </span>
              <div className="ml-auto flex shrink-0 items-center gap-2">
                {loading && <span className="text-indigo-400 animate-pulse">{t('Loading...')}</span>}
                <button
                  type="button"
                  onClick={() => setContentSearchOpen(true)}
                  className="flex h-7 w-7 items-center justify-center rounded th-text-muted th-hover-surface hover:text-indigo-400 transition-colors"
                  title={t('Find in file')}
                >
                  <Search className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
          )}

          {contentSearchOpen && (
            <div className="flex items-center gap-2 border-b th-border th-bg-surface px-3 py-2">
              <Search className="h-4 w-4 shrink-0 th-text-muted" />
              <input
                ref={contentSearchInputRef}
                type="text"
                value={contentSearchQuery}
                onChange={(e) => setContentSearchQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    goToContentSearchMatch(e.shiftKey ? -1 : 1);
                  } else if (e.key === 'Escape') {
                    e.preventDefault();
                    setContentSearchOpen(false);
                  }
                }}
                placeholder={t('Find in current file...')}
                className="h-8 min-w-0 flex-1 max-w-md rounded-md border th-border-subtle th-bg-input px-3 py-1 text-xs th-text-2 placeholder:text-slate-400 dark:placeholder:text-slate-500 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              />
              <span className="w-16 shrink-0 text-center text-xs tabular-nums th-text-muted">
                {contentSearchQuery.trim()
                  ? (contentSearchMatchCount > 0 ? `${Math.min(contentSearchIndex + 1, contentSearchMatchCount)}/${contentSearchMatchCount}` : t('No matches'))
                  : ''}
              </span>
              <button
                type="button"
                onClick={() => goToContentSearchMatch(-1)}
                disabled={contentSearchMatchCount === 0}
                className="flex h-7 w-7 items-center justify-center rounded th-text-muted th-hover-surface hover:text-indigo-400 disabled:opacity-40 disabled:hover:text-inherit transition-colors"
                title={t('Previous match')}
              >
                <ChevronUp className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                onClick={() => goToContentSearchMatch(1)}
                disabled={contentSearchMatchCount === 0}
                className="flex h-7 w-7 items-center justify-center rounded th-text-muted th-hover-surface hover:text-indigo-400 disabled:opacity-40 disabled:hover:text-inherit transition-colors"
                title={t('Next match')}
              >
                <ChevronDown className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                onClick={() => setContentSearchOpen(false)}
                className="flex h-7 w-7 items-center justify-center rounded th-text-muted th-hover-surface hover:text-indigo-400 transition-colors"
                title={t('Close search')}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          )}
          
          <div ref={contentScrollRef} className="flex-1 overflow-auto bg-white dark:bg-[#0d1117]">
            {content ? (
              <div className="flex text-[13px] leading-relaxed font-mono w-fit min-w-full">
                <div 
                  className="sticky left-0 select-none text-right px-3 py-4 border-r th-border text-slate-400 dark:text-slate-600 bg-slate-50 dark:bg-[#0d1117] min-w-[3rem] whitespace-pre z-10"
                >
                  {Array.from({ length: lineCount }, (_, i) => i + 1).join('\n')}
                </div>
                <pre className="m-0 flex-1 p-4 text-slate-900 dark:text-slate-200 overflow-visible">
                  <code dangerouslySetInnerHTML={{ __html: searchableContent }} className="hljs" style={{ background: 'transparent', padding: 0 }} />
                </pre>
              </div>
            ) : (
              <div className="h-full flex items-center justify-center text-sm th-text-muted">
                {loading ? t('Decompiling/Reading...') : (isJar ? t('Select a file to view its contents') : '')}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
