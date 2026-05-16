import { useState, useRef, useCallback, useEffect, useMemo } from 'react';
import { FileUp, ClipboardPaste, ChevronUp, ChevronDown, RotateCcw, ArrowLeftRight, XCircle } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { useI18n } from '../i18n';

/* ------------------------------------------------------------------ */
/*  Text file extensions that are allowed                              */
/* ------------------------------------------------------------------ */
const TEXT_EXTS = new Set([
  'txt', 'md', 'markdown', 'yaml', 'yml', 'json', 'jsonc', 'json5',
  'xml', 'html', 'htm', 'css', 'scss', 'less', 'js', 'jsx', 'ts',
  'tsx', 'csv', 'tsv', 'log', 'ini', 'cfg', 'conf', 'toml', 'env',
  'sh', 'bash', 'zsh', 'bat', 'cmd', 'ps1', 'py', 'rb', 'java',
  'c', 'cpp', 'h', 'hpp', 'go', 'rs', 'swift', 'kt', 'sql', 'graphql',
  'properties', 'gitignore', 'dockerignore', 'editorconfig', 'eslintrc',
  'prettierrc', 'babelrc', 'npmrc', 'lock', 'vue', 'svelte',
]);

function isTextFile(name: string): boolean {
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  if (TEXT_EXTS.has(ext)) return true;
  // Files without extension or dot-files (e.g. .gitignore) are treated as text
  if (!name.includes('.') || name.startsWith('.')) return true;
  return false;
}

/* ------------------------------------------------------------------ */
/*  Myers diff algorithm – operates on arrays of strings (lines)       */
/* ------------------------------------------------------------------ */
type DiffOp = 'equal' | 'insert' | 'delete';
interface DiffLine { op: DiffOp; oldIdx: number; newIdx: number; text: string }

function myersDiff(a: string[], b: string[]): DiffLine[] {
  const n = a.length;
  const m = b.length;
  if (n === 0 && m === 0) return [];
  if (n === 0) return b.map((text, i) => ({ op: 'insert' as DiffOp, oldIdx: -1, newIdx: i, text }));
  if (m === 0) return a.map((text, i) => ({ op: 'delete' as DiffOp, oldIdx: i, newIdx: -1, text }));

  const max = n + m;
  const vSize = 2 * max + 1;
  const v = new Int32Array(vSize);
  const trace: Int32Array[] = [];

  const vIdx = (k: number) => ((k % vSize) + vSize) % vSize;

  // v[1] = 0 is the standard starting point
  v[vIdx(1)] = 0;

  outer:
  for (let d = 0; d <= max; d++) {
    trace.push(new Int32Array(v));
    for (let k = -d; k <= d; k += 2) {
      let x: number;
      if (k === -d || (k !== d && v[vIdx(k - 1)] < v[vIdx(k + 1)])) {
        x = v[vIdx(k + 1)];
      } else {
        x = v[vIdx(k - 1)] + 1;
      }
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) { x++; y++; }
      v[vIdx(k)] = x;
      if (x >= n && y >= m) break outer;
    }
  }

  // Backtrack
  const result: DiffLine[] = [];
  let x = n, y = m;
  for (let d = trace.length - 1; d > 0; d--) {
    // trace[d] = v at START of iteration d = v AFTER iteration d-1
    const vPrev = trace[d];
    const k = x - y;
    let prevK: number;
    if (k === -d || (k !== d && vPrev[vIdx(k - 1)] < vPrev[vIdx(k + 1)])) {
      prevK = k + 1;
    } else {
      prevK = k - 1;
    }
    const prevX = vPrev[vIdx(prevK)];
    const prevY = prevX - prevK;
    // Diagonal (equal lines)
    while (x > prevX && y > prevY) {
      x--; y--;
      result.push({ op: 'equal', oldIdx: x, newIdx: y, text: a[x] });
    }
    if (x === prevX) {
      y--;
      result.push({ op: 'insert', oldIdx: -1, newIdx: y, text: b[y] });
    } else {
      x--;
      result.push({ op: 'delete', oldIdx: x, newIdx: -1, text: a[x] });
    }
  }
  // Remaining diagonal at d=0
  while (x > 0 && y > 0) {
    x--; y--;
    result.push({ op: 'equal', oldIdx: x, newIdx: y, text: a[x] });
  }
  while (x > 0) {
    x--;
    result.push({ op: 'delete', oldIdx: x, newIdx: -1, text: a[x] });
  }
  while (y > 0) {
    y--;
    result.push({ op: 'insert', oldIdx: -1, newIdx: y, text: b[y] });
  }
  result.reverse();
  return result;
}

/* ------------------------------------------------------------------ */
/*  Build side-by-side rows from diff result                           */
/* ------------------------------------------------------------------ */
interface DiffRow {
  leftNum: number | null;
  leftText: string;
  leftType: 'equal' | 'delete' | 'empty';
  rightNum: number | null;
  rightText: string;
  rightType: 'equal' | 'insert' | 'empty';
  isDiff: boolean;
  wdiff?: { oldSpans: { text: string; highlighted: boolean }[]; newSpans: { text: string; highlighted: boolean }[] } | null;
}

function buildRows(diff: DiffLine[]): DiffRow[] {
  const rows: DiffRow[] = [];
  let i = 0;
  while (i < diff.length) {
    const d = diff[i];
    if (d.op === 'equal') {
      rows.push({
        leftNum: d.oldIdx + 1, leftText: d.text, leftType: 'equal',
        rightNum: d.newIdx + 1, rightText: d.text, rightType: 'equal',
        isDiff: false,
      });
      i++;
    } else if (d.op === 'delete') {
      // Collect consecutive deletes
      const deletes: DiffLine[] = [];
      while (i < diff.length && diff[i].op === 'delete') { deletes.push(diff[i]); i++; }
      // Collect consecutive inserts
      const inserts: DiffLine[] = [];
      while (i < diff.length && diff[i].op === 'insert') { inserts.push(diff[i]); i++; }
      const max = Math.max(deletes.length, inserts.length);
      for (let j = 0; j < max; j++) {
        const del = j < deletes.length ? deletes[j] : null;
        const ins = j < inserts.length ? inserts[j] : null;
        rows.push({
          leftNum: del ? del.oldIdx + 1 : null,
          leftText: del ? del.text : '',
          leftType: del ? 'delete' : 'empty',
          rightNum: ins ? ins.newIdx + 1 : null,
          rightText: ins ? ins.text : '',
          rightType: ins ? 'insert' : 'empty',
          isDiff: true,
        });
      }
    } else if (d.op === 'insert') {
      const inserts: DiffLine[] = [];
      while (i < diff.length && diff[i].op === 'insert') { inserts.push(diff[i]); i++; }
      for (const ins of inserts) {
        rows.push({
          leftNum: null, leftText: '', leftType: 'empty',
          rightNum: ins.newIdx + 1, rightText: ins.text, rightType: 'insert',
          isDiff: true,
        });
      }
    }
  }
  return rows;
}

/* ------------------------------------------------------------------ */
/*  Inline word-level highlighting for modified lines                   */
/* ------------------------------------------------------------------ */
interface WordSpan { text: string; highlighted: boolean }

function wordDiff(oldLine: string, newLine: string): { oldSpans: WordSpan[]; newSpans: WordSpan[] } {
  // Tokenize by word boundaries: split on non-alphanumeric chars so that
  // identifiers like "zaxxer" and "zaxxeir" are compared as individual tokens.
  const tokenize = (s: string): string[] => {
    const tokens: string[] = [];
    let current = '';
    for (const ch of s) {
      // Letters, digits, CJK characters stay together
      if (/[\w\u4e00-\u9fff]/.test(ch)) {
        current += ch;
      } else {
        if (current) { tokens.push(current); current = ''; }
        tokens.push(ch);
      }
    }
    if (current) tokens.push(current);
    return tokens;
  };

  const aTokens = tokenize(oldLine);
  const bTokens = tokenize(newLine);
  const diff = myersDiff(aTokens, bTokens);

  const oldSpans: WordSpan[] = [];
  const newSpans: WordSpan[] = [];
  for (const d of diff) {
    if (d.op === 'equal') {
      oldSpans.push({ text: d.text, highlighted: false });
      newSpans.push({ text: d.text, highlighted: false });
    } else if (d.op === 'delete') {
      oldSpans.push({ text: d.text, highlighted: true });
    } else {
      newSpans.push({ text: d.text, highlighted: true });
    }
  }
  return { oldSpans, newSpans };
}

/* ------------------------------------------------------------------ */
/*  FilePanel — one side of the comparison                             */
/* ------------------------------------------------------------------ */
interface FilePanelProps {
  side: 'left' | 'right';
  fileName: string;
  content: string;
  onFileOpen: (name: string, content: string) => void;
}

function FilePanel({ side, fileName, content, onFileOpen }: FilePanelProps) {
  const { t } = useI18n();
  const [dragOver, setDragOver] = useState(false);
  const [pasteError, setPasteError] = useState(false);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(false);
  }, []);

  const handleDrop = useCallback(async (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(false);

    // Try to read dropped text first
    const text = e.dataTransfer.getData('text/plain');
    if (text) {
      onFileOpen(t('Pasted Text'), text);
      return;
    }

    // For drag-and-drop from OS, files are read via the browser File API
    const files = Array.from(e.dataTransfer.files);
    if (files.length > 0) {
      const file = files[0];
      if (!isTextFile(file.name)) return;
      if (file.size > 10 * 1024 * 1024) {
        alert(t('File size exceeds 10MB limit'));
        return;
      }
      const reader = new FileReader();
      reader.onload = () => {
        if (typeof reader.result === 'string') {
          onFileOpen(file.name, reader.result);
        }
      };
      reader.readAsText(file);
    }
  }, [onFileOpen, t]);

  const handleOpenFile = useCallback(async () => {
    try {
      const result = await invoke<[string, string]>('open_text_file');
      const name = result[0].split(/[/\\]/).pop() || result[0];
      onFileOpen(name, result[1]);
    } catch {
      // User cancelled
    }
  }, [onFileOpen]);

  const handlePasteFromClipboard = useCallback(async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (text) {
        if (text.length > 10 * 1024 * 1024) {
          alert(t('Pasted text exceeds 10MB limit'));
          return;
        }
        onFileOpen(t('Pasted Text'), text);
      }
    } catch {
      setPasteError(true);
      setTimeout(() => setPasteError(false), 2000);
    }
  }, [onFileOpen, t]);

  return (
    <div
      className={`flex flex-col flex-1 min-w-0 border th-border rounded-lg overflow-hidden transition-all ${
        dragOver ? 'ring-2 ring-indigo-500 border-indigo-500/50' : ''
      }`}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 th-bg-surface-h border-b th-border gap-2">
        <span className="text-xs font-medium th-text-3 truncate flex-1" title={fileName}>
          {fileName || (side === 'left' ? t('Original File') : t('Modified File'))}
        </span>
        <div className="flex items-center gap-1 flex-shrink-0">
          <button
            onClick={handleOpenFile}
            className="p-1.5 rounded th-text-muted hover:th-text-2 th-hover-surface transition-colors"
            title={t('Open File')}
          >
            <FileUp className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={handlePasteFromClipboard}
            className={`p-1.5 rounded transition-colors ${
              pasteError ? 'bg-red-500/20 text-red-400' : 'th-text-muted hover:th-text-2 th-hover-surface'
            }`}
            title={pasteError ? t('Failed') : t('Paste from Clipboard')}
          >
            {pasteError ? <XCircle className="w-3.5 h-3.5" /> : <ClipboardPaste className="w-3.5 h-3.5" />}
          </button>
        </div>
      </div>

      {/* Content */}
      <textarea
        value={content}
        onChange={(e) => {
          if (e.target.value.length > 10 * 1024 * 1024) {
            alert(t('Text exceeds 10MB limit'));
            return;
          }
          onFileOpen(fileName || t('Pasted Text'), e.target.value);
        }}
        placeholder={t('Paste text here, or drop a file')}
        className="flex-1 p-3 text-xs font-mono th-bg-input th-text-2 resize-none outline-none leading-relaxed"
        spellCheck={false}
      />
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  DiffMinimap — vertical navigation bar on the right edge            */
/* ------------------------------------------------------------------ */
interface DiffMinimapProps {
  rows: DiffRow[];
  diffIndices: number[];
  currentDiffIdx: number;
  onNavigate: (diffIdx: number) => void;
  containerRef: React.RefObject<HTMLDivElement | null>;
}

function DiffMinimap({ rows, diffIndices, currentDiffIdx, onNavigate, containerRef }: DiffMinimapProps) {
  const totalRows = rows.length;
  if (totalRows === 0) return null;

  // Build diff group ranges: [startRow, endRow, groupIndex]
  const groups: { start: number; end: number; idx: number }[] = [];
  let groupIdx = 0;
  for (let i = 0; i < diffIndices.length; i++) {
    const start = diffIndices[i];
    let end = start;
    while (end + 1 < totalRows && rows[end + 1].isDiff) end++;
    groups.push({ start, end, idx: groupIdx });
    groupIdx++;
  }

  const handleClick = (groupIndex: number) => {
    onNavigate(groupIndex);
    const container = containerRef.current;
    if (!container) return;
    const rowIdx = diffIndices[groupIndex];
    const el = container.querySelector(`[data-row="${rowIdx}"]`) as HTMLElement | null;
    if (!el) return;
    const elAbsTop = el.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop;
    const targetScrollTop = Math.max(0, elAbsTop - container.clientHeight / 2 + el.clientHeight / 2);
    container.scrollTo({ top: targetScrollTop, behavior: 'smooth' });
  };

  return (
    <div className="diff-minimap">
      {groups.map((g) => {
        const topPct = (g.start / totalRows) * 100;
        const heightPct = Math.max(((g.end - g.start + 1) / totalRows) * 100, 0.8);
        const isCurrent = g.idx === currentDiffIdx;
        return (
          <div
            key={g.idx}
            className={`diff-minimap-block ${isCurrent ? 'diff-minimap-current' : ''}`}
            style={{ top: `${topPct}%`, height: `${heightPct}%` }}
            onClick={() => handleClick(g.idx)}
            title={`${g.idx + 1} / ${groups.length}`}
          />
        );
      })}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  DiffView — the actual comparison result                            */
/* ------------------------------------------------------------------ */
interface DiffViewProps {
  rows: DiffRow[];
  diffIndices: number[];
  currentDiffIdx: number;
  onNavigate: (idx: number) => void;
  userNavRef: React.RefObject<boolean>;
}

function DiffView({ rows, diffIndices, currentDiffIdx, onNavigate, userNavRef }: DiffViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  // Scroll within the diff-scroll-area only when user explicitly navigates
  useEffect(() => {
    if (!userNavRef.current) return;
    userNavRef.current = false;
    if (currentDiffIdx < 0 || currentDiffIdx >= diffIndices.length) return;
    const container = containerRef.current;
    if (!container) return;
    const rowIdx = diffIndices[currentDiffIdx];
    const el = container.querySelector(`[data-row="${rowIdx}"]`) as HTMLElement | null;
    if (!el) return;
    // Absolute scroll position: convert viewport-relative coords to scroll offset
    const elAbsTop = el.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop;
    const targetScrollTop = Math.max(0, elAbsTop - container.clientHeight / 2 + el.clientHeight / 2);
    container.scrollTo({ top: targetScrollTop, behavior: 'smooth' });
  }, [currentDiffIdx, diffIndices, userNavRef]);

  const gutterWidth = Math.max(String(rows.length).length * 8 + 16, 40);

  return (
    <div className="diff-view-wrapper">
      <div ref={containerRef} className="diff-scroll-area font-mono text-xs leading-5">
        <table className="w-full border-collapse" style={{ tableLayout: 'fixed' }}>
          <colgroup>
            <col style={{ width: `${gutterWidth}px` }} />
            <col style={{ width: `calc(50% - ${gutterWidth}px)` }} />
            <col style={{ width: `${gutterWidth}px` }} />
            <col style={{ width: `calc(50% - ${gutterWidth}px)` }} />
          </colgroup>
          <tbody>
            {rows.map((row, i) => {
              const isCurrentDiff = diffIndices[currentDiffIdx] === i;
              const wdiff = row.wdiff;

              return (
                <tr
                  key={i}
                  data-row={i}
                  className={`${isCurrentDiff ? 'diff-current-row' : ''}`}
                >
                  {/* Left gutter */}
                  <td className={`text-right px-2 select-none border-r th-border diff-gutter ${
                    row.leftType === 'delete' ? 'diff-gutter-del' :
                    row.leftType === 'empty' ? 'diff-gutter-empty' : ''
                  }`}>
                    {row.leftNum ?? ''}
                  </td>
                  {/* Left content */}
                  <td className={`px-2 whitespace-pre overflow-hidden ${
                    row.leftType === 'delete' ? 'diff-line-del' :
                    row.leftType === 'empty' ? 'diff-line-empty' : 'diff-line-eq'
                  }`}>
                    {wdiff
                      ? wdiff.oldSpans.map((s, si) => (
                          <span key={si} className={s.highlighted ? 'diff-word-del' : ''}>
                            {s.text}
                          </span>
                        ))
                      : row.leftText}
                  </td>
                  {/* Right gutter */}
                  <td className={`text-right px-2 select-none border-r border-l th-border diff-gutter ${
                    row.rightType === 'insert' ? 'diff-gutter-ins' :
                    row.rightType === 'empty' ? 'diff-gutter-empty' : ''
                  }`}>
                    {row.rightNum ?? ''}
                  </td>
                  {/* Right content */}
                  <td className={`px-2 whitespace-pre overflow-hidden ${
                    row.rightType === 'insert' ? 'diff-line-ins' :
                    row.rightType === 'empty' ? 'diff-line-empty' : 'diff-line-eq'
                  }`}>
                    {wdiff
                      ? wdiff.newSpans.map((s, si) => (
                          <span key={si} className={s.highlighted ? 'diff-word-ins' : ''}>
                            {s.text}
                          </span>
                        ))
                      : row.rightText}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {/* Right-side minimap navigation */}
      <DiffMinimap
        rows={rows}
        diffIndices={diffIndices}
        currentDiffIdx={currentDiffIdx}
        onNavigate={onNavigate}
        containerRef={containerRef}
      />
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Main FileDiff component                                            */
/* ------------------------------------------------------------------ */
export function FileDiff() {
  const { t } = useI18n();

  const [leftName, setLeftName] = useState('');
  const [leftContent, setLeftContent] = useState('');
  const [rightName, setRightName] = useState('');
  const [rightContent, setRightContent] = useState('');
  const [showInput, setShowInput] = useState(true);
  const [currentDiffIdx, setCurrentDiffIdx] = useState(0);
  const userNavRef = useRef(false);

  const leftContentRef = useRef(leftContent);
  const rightContentRef = useRef(rightContent);
  const leftNameRef = useRef(leftName);
  const rightNameRef = useRef(rightName);

  useEffect(() => { leftContentRef.current = leftContent; }, [leftContent]);
  useEffect(() => { rightContentRef.current = rightContent; }, [rightContent]);
  useEffect(() => { leftNameRef.current = leftName; }, [leftName]);
  useEffect(() => { rightNameRef.current = rightName; }, [rightName]);

  const handleLeftFile = useCallback((name: string, content: string) => {
    setLeftName(name);
    setLeftContent(content);
  }, []);

  const handleRightFile = useCallback((name: string, content: string) => {
    setRightName(name);
    setRightContent(content);
  }, []);

  // Handle Tauri drag-drop events for native file paths
  useEffect(() => {
    // Track which side should receive the next drop
    let dropSide: 'left' | 'right' = 'left';

    const setup = async () => {
      const unlisten = await getCurrentWebview().onDragDropEvent(async (event) => {
        if (event.payload.type === 'drop') {
          const paths = event.payload.paths;
          if (!paths || paths.length === 0) return;

          // If two files are dropped, assign left and right
          if (paths.length >= 2) {
            const path1 = paths[0];
            const name1 = path1.split(/[/\\]/).pop() || path1;
            if (isTextFile(name1)) {
              try {
                const [p1, c1] = await invoke<[string, string]>('read_text_file_by_path', { path: path1 });
                const n1 = p1.split(/[/\\]/).pop() || p1;
                handleLeftFile(n1, c1);
              } catch { /* skip */ }
            }
            const path2 = paths[1];
            const name2 = path2.split(/[/\\]/).pop() || path2;
            if (isTextFile(name2)) {
              try {
                const [p2, c2] = await invoke<[string, string]>('read_text_file_by_path', { path: path2 });
                const n2 = p2.split(/[/\\]/).pop() || p2;
                handleRightFile(n2, c2);
              } catch { /* skip */ }
            }
            return;
          }

          // Single file: assign to whichever side is empty, or alternate
          const droppedPath = paths[0];
          const fileName = droppedPath.split(/[/\\]/).pop() || droppedPath;
          if (!isTextFile(fileName)) return;

          try {
            const [fullPath, content] = await invoke<[string, string]>('read_text_file_by_path', { path: droppedPath });
            const name = fullPath.split(/[/\\]/).pop() || fullPath;

            if (!leftContentRef.current && leftNameRef.current === '') {
              handleLeftFile(name, content);
              dropSide = 'right';
            } else if (!rightContentRef.current && rightNameRef.current === '') {
              handleRightFile(name, content);
              dropSide = 'left';
            } else if (dropSide === 'left') {
              handleLeftFile(name, content);
              dropSide = 'right';
            } else {
              handleRightFile(name, content);
              dropSide = 'left';
            }
          } catch {
            // read error
          }
        }
      });
      return unlisten;
    };
    const cleanup = setup();
    return () => { cleanup.then(fn => fn()); };
  }, [handleLeftFile, handleRightFile]);

  // Compute diff
  const { rows, diffIndices, totalDiffs, isTruncated } = useMemo(() => {
    if (!leftContent && !rightContent) return { rows: [], diffIndices: [], totalDiffs: 0, isTruncated: false };
    const leftLines = leftContent.split('\n');
    const rightLines = rightContent.split('\n');
    const MAX_LINES = 10000;
    
    let diff: DiffLine[];
    let skipWordDiff = false;
    if (leftLines.length > MAX_LINES || rightLines.length > MAX_LINES) {
      diff = [];
      const maxL = Math.max(leftLines.length, rightLines.length);
      for (let i = 0; i < maxL; i++) {
        const l = leftLines[i];
        const r = rightLines[i];
        if (l === r) {
          diff.push({ op: 'equal' as DiffOp, oldIdx: i, newIdx: i, text: l || '' });
        } else {
          if (i < leftLines.length) diff.push({ op: 'delete' as DiffOp, oldIdx: i, newIdx: -1, text: l });
          if (i < rightLines.length) diff.push({ op: 'insert' as DiffOp, oldIdx: -1, newIdx: i, text: r });
        }
      }
      skipWordDiff = true;
    } else {
      diff = myersDiff(leftLines, rightLines);
    }
    
    const rows = buildRows(diff);

    // Collect diff group start indices
    const indices: number[] = [];
    let inDiff = false;
    rows.forEach((row, i) => {
      if (row.isDiff && !inDiff) {
        indices.push(i);
        inDiff = true;
      } else if (!row.isDiff) {
        inDiff = false;
      }
      
      if (!skipWordDiff && row.isDiff && row.leftType === 'delete' && row.rightType === 'insert') {
        if (row.leftText.length < 500 && row.rightText.length < 500) {
          row.wdiff = wordDiff(row.leftText, row.rightText);
        }
      }
    });

    return { rows, diffIndices: indices, totalDiffs: indices.length, isTruncated: skipWordDiff };
  }, [leftContent, rightContent]);

  // Reset current diff index when diff changes
  useEffect(() => {
    setCurrentDiffIdx(0);
  }, [diffIndices.length]);

  const goToDiff = useCallback((direction: 'prev' | 'next') => {
    if (totalDiffs === 0) return;
    userNavRef.current = true;
    setCurrentDiffIdx(prev => {
      if (direction === 'next') return (prev + 1) % totalDiffs;
      return (prev - 1 + totalDiffs) % totalDiffs;
    });
  }, [totalDiffs]);

  const handleSwap = useCallback(() => {
    const tmpName = leftName;
    const tmpContent = leftContent;
    setLeftName(rightName);
    setLeftContent(rightContent);
    setRightName(tmpName);
    setRightContent(tmpContent);
  }, [leftName, leftContent, rightName, rightContent]);

  const handleReset = useCallback(() => {
    setLeftName('');
    setLeftContent('');
    setRightName('');
    setRightContent('');
    setShowInput(true);
    setCurrentDiffIdx(0);
  }, []);

  const hasAnyContent = leftContent.length > 0 || rightContent.length > 0;
  const hasBothContent = leftContent.length > 0 && rightContent.length > 0;

  return (
    <div className="flex flex-col h-full w-full overflow-hidden">
      {/* Toolbar — opaque sticky header */}
      <div className="flex items-center justify-between px-4 py-2 border-b th-border th-bg-surface flex-shrink-0">
        <div className="flex items-center gap-2">
          <h1 className="text-sm font-bold th-text tracking-tight">{t('File Compare')}</h1>
          {hasBothContent && (
            <span className={`text-xs font-medium px-2 py-0.5 rounded-full ${
              totalDiffs === 0
                ? 'bg-emerald-500/15 text-emerald-400'
                : 'bg-amber-500/15 text-amber-400'
            }`}>
              {totalDiffs === 0 ? t('Identical') : `${totalDiffs} ${t('differences')}`}
            </span>
          )}
        </div>

        <div className="flex items-center gap-1">
          {/* Toggle input panels */}
          {hasAnyContent && (
            <button
              onClick={() => setShowInput(prev => !prev)}
              className="px-2 py-1 text-xs rounded th-text-3 th-hover-surface transition-colors border th-border-muted"
            >
              {showInput ? t('Hide Input') : t('Show Input')}
            </button>
          )}

          {/* Diff navigation */}
          {totalDiffs > 0 && (
            <div className="flex items-center gap-0.5 ml-2">
              <button
                onClick={() => goToDiff('prev')}
                className="p-1 rounded th-text-3 th-hover-surface transition-colors"
                title={t('Previous Difference')}
              >
                <ChevronUp className="w-4 h-4" />
              </button>
              <span className="text-xs th-text-muted font-mono min-w-[3em] text-center">
                {currentDiffIdx + 1}/{totalDiffs}
              </span>
              <button
                onClick={() => goToDiff('next')}
                className="p-1 rounded th-text-3 th-hover-surface transition-colors"
                title={t('Next Difference')}
              >
                <ChevronDown className="w-4 h-4" />
              </button>
            </div>
          )}

          {/* Swap */}
          {hasBothContent && (
            <button
              onClick={handleSwap}
              className="p-1.5 rounded th-text-3 th-hover-surface transition-colors ml-1"
              title={t('Swap Files')}
            >
              <ArrowLeftRight className="w-3.5 h-3.5" />
            </button>
          )}

          {/* Reset */}
          {hasAnyContent && (
            <button
              onClick={handleReset}
              className="p-1.5 rounded th-text-3 th-hover-surface transition-colors"
              title={t('Reset')}
            >
              <RotateCcw className="w-3.5 h-3.5" />
            </button>
          )}
        </div>
      </div>

      {showInput && (
        <div className="flex gap-3 p-3 flex-shrink-0" style={{ height: hasAnyContent && hasBothContent ? '200px' : '280px' }}>
          <FilePanel
            side="left"
            fileName={leftName}
            content={leftContent}
            onFileOpen={handleLeftFile}
          />
          <FilePanel
            side="right"
            fileName={rightName}
            content={rightContent}
            onFileOpen={handleRightFile}
          />
        </div>
      )}

      {/* Diff view */}
      {hasBothContent ? (
        <div className="flex-1 min-h-0 border-t th-border flex flex-col relative">
          {isTruncated && (
            <div className="bg-amber-500/10 text-amber-500 text-xs px-4 py-2 border-b border-amber-500/20 text-center z-10 font-medium">
              {t('Files exceed 10000 lines. Diff is approximate.')}
            </div>
          )}
          <DiffView
            rows={rows}
            diffIndices={diffIndices}
            currentDiffIdx={currentDiffIdx}
            onNavigate={setCurrentDiffIdx}
            userNavRef={userNavRef}
          />
        </div>
      ) : (
        !showInput && (
          <div className="flex-1 flex items-center justify-center">
            <p className="text-sm th-text-muted">{t('Load two files to compare')}</p>
          </div>
        )
      )}
    </div>
  );
}
