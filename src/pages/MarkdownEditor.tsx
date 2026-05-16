import { useState, useCallback, useEffect, useRef, useMemo } from 'react';
import { Trash2, FolderOpen, Save, Copy, Check, Eye, Edit3, FileText, Download, Upload } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { Marked } from 'marked';
import { markedHighlight } from 'marked-highlight';
import hljs from 'highlight.js';
import DOMPurify from 'dompurify';
import { useI18n } from '../i18n';

export function MarkdownEditor({ setMdDirty }: { setMdDirty?: (dirty: boolean) => void }) {
  const { t } = useI18n();
  const [content, setContent] = useState('');
  const [filePath, setFilePath] = useState('');
  const [isDirty, setIsDirty] = useState(false);

  useEffect(() => {
    setMdDirty?.(isDirty);
    return () => setMdDirty?.(false);
  }, [isDirty, setMdDirty]);
  const [originalContent, setOriginalContent] = useState('');
  const [isCopied, setIsCopied] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);
  const [viewMode, setViewMode] = useState<'split' | 'edit' | 'preview'>(() => {
    return (localStorage.getItem('mtool_md_viewmode') as 'split' | 'edit' | 'preview') || 'preview';
  });

  useEffect(() => {
    localStorage.setItem('mtool_md_viewmode', viewMode);
  }, [viewMode]);
  const previewRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<HTMLTextAreaElement>(null);
  // Tracks which pane is driving the scroll to prevent infinite loops
  const scrollSourceRef = useRef<'editor' | 'preview' | null>(null);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
    };
  }, []);

  // Configure marked with GFM + highlight.js
  const marked = useMemo(() => {
    const m = new Marked(
      markedHighlight({
        emptyLangClass: 'hljs',
        langPrefix: 'hljs language-',
        highlight(code, lang) {
          const language = hljs.getLanguage(lang) ? lang : 'plaintext';
          return hljs.highlight(code, { language }).value;
        }
      })
    );
    m.setOptions({
      gfm: true,
      breaks: true,
    });
    return m;
  }, []);

  // Render markdown to HTML
  const renderedHtml = useMemo(() => {
    if (!content.trim()) return '';
    try {
      return DOMPurify.sanitize(marked.parse(content) as string);
    } catch {
      return `<p class="text-red-400">${t('Render error')}</p>`;
    }
  }, [content, marked, t]);

  // Track dirty state
  const handleContentChange = useCallback((value: string) => {
    setContent(value);
    setIsDirty(value !== originalContent);
  }, [originalContent]);

  // Open file
  const handleOpen = useCallback(async () => {
    try {
      const result = await invoke<[string, string]>('open_md_file');
      setFilePath(result[0]);
      setContent(result[1]);
      setOriginalContent(result[1]);
      setIsDirty(false);
    } catch {
      // user cancelled or error
    }
  }, []);

  // Save file
  const handleSave = useCallback(async () => {
    try {
      const savedPath = await invoke<string>('save_md_file', { path: filePath, content });
      setFilePath(savedPath);
      setOriginalContent(content);
      setIsDirty(false);
    } catch {
      // user cancelled or error
    }
  }, [filePath, content]);

  // Save As
  const handleSaveAs = useCallback(async () => {
    try {
      const savedPath = await invoke<string>('save_md_file_as', { content });
      setFilePath(savedPath);
      setOriginalContent(content);
      setIsDirty(false);
    } catch {
      // user cancelled or error
    }
  }, [content]);

  // Clear
  const handleClear = useCallback(() => {
    setContent('');
    setFilePath('');
    setOriginalContent('');
    setIsDirty(false);
  }, []);

  // Drag-drop visual feedback
  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
  }, []);

  // Copy Markdown source
  const handleCopy = useCallback(async () => {
    if (!content) return;
    try {
      await navigator.clipboard.writeText(content);
      setIsCopied(true);
      setTimeout(() => setIsCopied(false), 2000);
    } catch {
      // fallback
    }
  }, [content]);

  // Drag-drop file open
  useEffect(() => {
    const setup = async () => {
      const unlisten = await getCurrentWebview().onDragDropEvent(async (event) => {
        if (event.payload.type === 'drop') {
          const paths = event.payload.paths;
          if (!paths || paths.length === 0) return;
          const droppedPath = paths[0];
          const ext = droppedPath.split('.').pop()?.toLowerCase();
          if (!ext || !['md', 'markdown', 'txt'].includes(ext)) return;
          try {
            const result = await invoke<[string, string]>('open_md_file_by_path', { path: droppedPath });
            setFilePath(result[0]);
            setContent(result[1]);
            setOriginalContent(result[1]);
            setIsDirty(false);
          } catch {
            // read error
          }
        }
      });
      return unlisten;
    };
    const cleanup = setup();
    return () => { cleanup.then(fn => fn()); };
  }, []);

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 's') {
        e.preventDefault();
        if (e.shiftKey) {
          handleSaveAs();
        } else {
          handleSave();
        }
      }
      if ((e.ctrlKey || e.metaKey) && e.key === 'o') {
        e.preventDefault();
        handleOpen();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [handleSave, handleSaveAs, handleOpen]);

  // Bidirectional scroll sync — editor drives preview
  const handleEditorScroll = useCallback(() => {
    if (viewMode !== 'split' || !editorRef.current || !previewRef.current) return;
    if (scrollSourceRef.current === 'preview') return; // ignore echo
    scrollSourceRef.current = 'editor';
    const editor = editorRef.current;
    const ratio = editor.scrollTop / (editor.scrollHeight - editor.clientHeight || 1);
    previewRef.current.scrollTop = ratio * (previewRef.current.scrollHeight - previewRef.current.clientHeight || 1);
    // Reset source after scroll settles
    if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
    scrollTimerRef.current = setTimeout(() => { scrollSourceRef.current = null; }, 50);
  }, [viewMode]);

  // Bidirectional scroll sync — preview drives editor
  const handlePreviewScroll = useCallback(() => {
    if (viewMode !== 'split' || !editorRef.current || !previewRef.current) return;
    if (scrollSourceRef.current === 'editor') return; // ignore echo
    scrollSourceRef.current = 'preview';
    const preview = previewRef.current;
    const ratio = preview.scrollTop / (preview.scrollHeight - preview.clientHeight || 1);
    editorRef.current.scrollTop = ratio * (editorRef.current.scrollHeight - editorRef.current.clientHeight || 1);
    // Reset source after scroll settles
    if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
    scrollTimerRef.current = setTimeout(() => { scrollSourceRef.current = null; }, 50);
  }, [viewMode]);

  // Handle tab key in editor for indentation
  const handleEditorKeyDown = useCallback((e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Tab') {
      e.preventDefault();
      const textarea = e.currentTarget;
      const start = textarea.selectionStart;
      const end = textarea.selectionEnd;
      const value = textarea.value;
      const newValue = value.substring(0, start) + '  ' + value.substring(end);
      handleContentChange(newValue);
      // Restore cursor position
      requestAnimationFrame(() => {
        textarea.selectionStart = textarea.selectionEnd = start + 2;
      });
    }
  }, [handleContentChange]);

  // Intercept link clicks in preview — open in system browser instead of navigating the webview.
  // Only allow http/https to prevent file://, custom-app://, relative paths, etc.
  const handlePreviewClick = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const anchor = (e.target as HTMLElement).closest('a');
    if (!anchor) return;
    const href = anchor.getAttribute('href');
    if (!href) return;
    if (!href.startsWith('http://') && !href.startsWith('https://')) return;
    e.preventDefault();
    openUrl(href).catch(console.error);
  }, []);

  const lineCount = content ? content.split('\n').length : 0;
  const charCount = content.length;
  const wordCount = content.trim() ? content.trim().split(/\s+/).length : 0;
  const fileName = filePath ? filePath.split('/').pop() || filePath.split('\\').pop() || 'Untitled' : t('Untitled');

  return (
    <div
      className="w-full h-full flex flex-col relative"
      onDragOver={handleDragOver}
      onDragEnter={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {/* Header */}
      <div className="flex justify-between items-center mb-6 border-b th-border pb-4">
        <div className="flex items-center gap-3">
          <h2 className="th-text font-semibold text-lg flex items-center gap-2">
            <span className="text-indigo-400"><FileText className="w-5 h-5 inline" /></span> {t('Markdown Editor')}
          </h2>
          {filePath && (
            <span className="text-xs th-text-muted font-mono max-w-[300px] truncate" title={filePath}>
              {fileName}{isDirty && ' •'}
            </span>
          )}
          {!filePath && isDirty && (
            <span className="text-xs th-text-muted font-mono">
              {t('Untitled')}{' •'}
            </span>
          )}
        </div>

        <div className="flex items-center gap-2">
          {/* View mode toggle */}
          <div className="flex items-center th-bg-surface rounded border th-border-subtle mr-2">
            <button
              onClick={() => setViewMode('edit')}
              className={`px-2 py-1.5 text-xs font-medium transition-colors flex items-center gap-1 rounded-l ${
                viewMode === 'edit' ? 'bg-indigo-600 text-white' : 'th-text-3 hover:th-text-2'
              }`}
              title={t('Edit Mode')}
            >
              <Edit3 className="w-3.5 h-3.5" />
            </button>
            <button
              onClick={() => setViewMode('split')}
              className={`px-2 py-1.5 text-xs font-medium transition-colors flex items-center gap-1 border-x th-border-subtle ${
                viewMode === 'split' ? 'bg-indigo-600 text-white' : 'th-text-3 hover:th-text-2'
              }`}
              title={t('Split View')}
            >
              <Edit3 className="w-3.5 h-3.5" /><Eye className="w-3.5 h-3.5" />
            </button>
            <button
              onClick={() => setViewMode('preview')}
              className={`px-2 py-1.5 text-xs font-medium transition-colors flex items-center gap-1 rounded-r ${
                viewMode === 'preview' ? 'bg-indigo-600 text-white' : 'th-text-3 hover:th-text-2'
              }`}
              title={t('Preview Mode')}
            >
              <Eye className="w-3.5 h-3.5" />
            </button>
          </div>

          <button
            onClick={handleOpen}
            className="px-3 py-1.5 text-xs th-bg-surface th-hover-surface th-text-2 rounded font-medium border th-border-subtle transition-colors flex items-center gap-1.5 focus:outline-none"
          >
            <FolderOpen className="w-3.5 h-3.5" /> {t('Open')}
          </button>
          <button
            onClick={handleSave}
            className="px-3 py-1.5 text-xs th-bg-surface th-hover-surface th-text-2 rounded font-medium border th-border-subtle transition-colors flex items-center gap-1.5 focus:outline-none"
          >
            <Save className="w-3.5 h-3.5" /> {t('Save')}
          </button>
          <button
            onClick={handleSaveAs}
            className="px-3 py-1.5 text-xs th-bg-surface th-hover-surface th-text-2 rounded font-medium border th-border-subtle transition-colors flex items-center gap-1.5 focus:outline-none"
          >
            <Download className="w-3.5 h-3.5" /> {t('Save As')}
          </button>
          <button
            onClick={handleClear}
            className="px-3 py-1.5 text-xs th-bg-surface th-hover-surface th-text-2 rounded font-medium border th-border-subtle transition-colors flex items-center gap-1.5 focus:outline-none"
          >
            <Trash2 className="w-3.5 h-3.5" /> {t('Clear')}
          </button>
        </div>
      </div>

      {/* Main editor area */}
      <div className="flex-1 flex gap-4 min-h-0">

        {/* Editor pane */}
        {(viewMode === 'split' || viewMode === 'edit') && (
          <div className="flex-1 th-bg-card border th-border rounded-xl flex flex-col overflow-hidden shadow-2xl">
            <div className="px-4 py-2 th-bg-surface-h border-b th-border flex justify-between items-center">
              <span className="text-[11px] font-bold th-text-3 uppercase tracking-tighter flex items-center gap-2">
                <Edit3 className="w-3 h-3" /> {t('Editor')}
              </span>
              <div className="flex gap-3 text-[10px] th-text-muted font-mono">
                <span>Markdown</span>
                <span>UTF-8</span>
              </div>
            </div>
            <textarea
              ref={editorRef}
              value={content}
              onChange={(e) => handleContentChange(e.target.value)}
              onScroll={handleEditorScroll}
              onKeyDown={handleEditorKeyDown}
              className="flex-1 w-full bg-transparent p-4 th-text-2 text-sm font-mono focus:outline-none resize-none leading-relaxed"
              placeholder={t('Start writing Markdown...')}
              spellCheck={false}
            />
          </div>
        )}

        {/* Preview pane */}
        {(viewMode === 'split' || viewMode === 'preview') && (
          <div className="flex-1 th-bg-card border th-border rounded-xl flex flex-col overflow-hidden shadow-2xl">
            <div className="px-4 py-2 th-bg-surface-h border-b th-border flex justify-between items-center">
              <span className="text-[11px] font-bold th-text-3 uppercase tracking-tighter flex items-center gap-2">
                <Eye className="w-3 h-3" /> {t('Preview')}
              </span>
              <button
                onClick={handleCopy}
                className={`text-[10px] font-bold transition-colors uppercase flex items-center gap-1 ${isCopied ? 'text-emerald-400' : 'text-indigo-400 hover:text-indigo-300'}`}
              >
                {isCopied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />} {isCopied ? t('Copied!') : t('Copy')}
              </button>
            </div>

            <div
              ref={previewRef}
              onScroll={handlePreviewScroll}
              onClick={handlePreviewClick}
              className="flex-1 p-6 overflow-y-auto"
            >
              {renderedHtml ? (
                <div
                  className="markdown-body"
                  dangerouslySetInnerHTML={{ __html: renderedHtml }}
                />
              ) : (
                <div className="text-sm th-text-faint italic">
                  {t('Preview will appear here...')}
                </div>
              )}
            </div>
          </div>
        )}

      </div>

      {/* Status bar */}
      <footer className="h-8 border-t th-border mt-4 px-4 th-bg-card flex items-center justify-between text-[10px] th-text-muted rounded-b-xl shadow-inner" style={{ opacity: 0.8 }}>
        <div className="flex flex-row items-center gap-4">
          <span className="flex items-center gap-1.5">
            <span className={`w-1.5 h-1.5 rounded-full ${isDirty ? 'bg-amber-400' : 'bg-emerald-500'} animate-pulse`}></span>
            {isDirty ? t('Modified') : t('System Ready')}
          </span>
          <span>{t('Lines')}: {lineCount}</span>
          <span>{t('Words')}: {wordCount}</span>
          <span>Length: {charCount} {t('chars')}</span>
        </div>
        <div className="flex items-center gap-2 italic">
          MTOOL Desktop Tools {new Date().getFullYear()}
        </div>
      </footer>

      {isDragOver && (
        <div className="absolute inset-0 z-50 flex items-center justify-center bg-indigo-950/80 border-2 border-dashed border-indigo-400 rounded-xl pointer-events-none">
          <div className="flex flex-col items-center gap-3 text-indigo-300">
            <Upload className="w-12 h-12" />
            <span className="text-lg font-semibold">{t('Release to open file')}</span>
          </div>
        </div>
      )}
    </div>
  );
}
