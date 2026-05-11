import { Copy, MinusSquare, Trash2, AlignLeft, Check } from 'lucide-react';
import { useState, useCallback, useEffect, useRef, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../i18n';

const BRACE_COLORS = [
  'text-indigo-400',
  'text-emerald-400',
  'text-amber-400',
  'text-rose-400',
  'text-cyan-400',
  'text-violet-400',
];

function escapeHtml(str: string): string {
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// Full JSON syntax highlighter: keys=blue, strings=green, numbers=red,
// bools=violet, null=slate, braces=rainbow by depth.
function syntaxHighlight(json: string): string {
  let depth = 0;

  const escaped = json
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');

  return escaped.replace(
    /("(?:\\u[a-fA-F0-9]{4}|\\[^u]|[^\\"])*"(?:\s*:)?|\b(?:true|false|null)\b|(?<!\d)-?\d+(?:\.\d*)?(?:[eE][+-]?\d+)?|[{}\[\]])/g,
    (match) => {
      if (match[0] === '"') {
        return /:$/.test(match)
          ? `<span class="json-key">${match}</span>`
          : `<span class="json-string">${match}</span>`;
      }
      if (match === 'true' || match === 'false') {
        return `<span class="json-bool">${match}</span>`;
      }
      if (match === 'null') {
        return `<span class="json-null">${match}</span>`;
      }
      if (match === '{' || match === '[') {
        const cls = BRACE_COLORS[depth % BRACE_COLORS.length];
        depth++;
        return `<span class="${cls}">${match}</span>`;
      }
      if (match === '}' || match === ']') {
        depth = Math.max(0, depth - 1);
        return `<span class="${BRACE_COLORS[depth % BRACE_COLORS.length]}">${match}</span>`;
      }
      return `<span class="json-number">${match}</span>`;
    }
  );
}

function buildCollapsibleHtml(text: string, highlightedHtml: string): string {
  const plainLines = text.split('\n');
  const htmlLines = highlightedHtml.split('\n');
  
  const blockEnds = new Map<number, number>();
  const stack: number[] = [];
  for (let i = 0; i < plainLines.length; i++) {
    const trimmed = plainLines[i].trim();
    if (trimmed.endsWith('{') || trimmed.endsWith('[')) {
      stack.push(i);
    } else if (trimmed.startsWith('}') || trimmed.startsWith(']')) {
      if (stack.length > 0) {
        const start = stack.pop()!;
        blockEnds.set(start, i);
      }
    }
  }

  function render(start: number, end: number): string {
    const result = [];
    let i = start;
    while (i <= end) {
      if (blockEnds.has(i)) {
        const blockEnd = blockEnds.get(i)!;
        const startHtml = htmlLines[i];
        const match = startHtml.match(/^( *)/);
        const spaces = match ? match[1] : '';
        const rest = startHtml.slice(spaces.length);
        const suffix = plainLines[blockEnd].trim();
        
        const summary = `<summary class="json-summary" data-suffix="${escapeHtml(suffix)}">${spaces}<span class="json-toggle"></span>${rest}</summary>`;
        
        const inner = render(i + 1, blockEnd - 1);
        const endLineHtml = htmlLines[blockEnd];
        
        const detailsContent = [summary];
        if (inner) detailsContent.push(inner);
        detailsContent.push(endLineHtml);
        
        result.push(`<details class="json-collapse" open>${detailsContent.join('\n')}</details>`);
        
        i = blockEnd + 1;
      } else {
        result.push(htmlLines[i]);
        i++;
      }
    }
    return result.join('\n');
  }

  return render(0, plainLines.length - 1);
}

const MAX_LINE_NUMBER_ROWS = 2000;

export function JsonFormatter() {
  const { t } = useI18n();
  const [rawInput, setRawInput] = useState('');
  const [formattedHtml, setFormattedHtml] = useState('');
  
  const lineNumbersRef = useRef<HTMLDivElement>(null);

  const handleScroll = useCallback((e: React.UIEvent<HTMLTextAreaElement>) => {
    if (lineNumbersRef.current) {
      lineNumbersRef.current.scrollTop = e.currentTarget.scrollTop;
    }
  }, []);

  const inputLinesCount = useMemo(() => {
    const count = rawInput.split('\n').length;
    if (count > MAX_LINE_NUMBER_ROWS) return null;
    return Array.from({ length: Math.max(1, count) }, (_, i) => i + 1);
  }, [rawInput]);

  const [plainOutput, setPlainOutput] = useState('');
  const [isError, setIsError] = useState(false);
  const [isCopied, setIsCopied] = useState(false);

  const updateOutput = useCallback((text: string, isMinified: boolean) => {
    setPlainOutput(text);
    if (isMinified) {
      setFormattedHtml(
        text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      );
    } else {
      const highlighted = syntaxHighlight(text);
      setFormattedHtml(buildCollapsibleHtml(text, highlighted));
    }
    setIsError(false);
  }, []);

  const showError = useCallback((err: string) => {
    setPlainOutput(err);
    setFormattedHtml(`<span class="text-red-400">${escapeHtml(err)}</span>`);
    setIsError(true);
  }, []);

  useEffect(() => {
    if (!rawInput.trim()) {
      setFormattedHtml('');
      setPlainOutput('');
      setIsError(false);
      return;
    }
    const timer = setTimeout(async () => {
      try {
        const result = await invoke<string>('format_json', { input: rawInput });
        updateOutput(result, false);
      } catch (e) {
        showError(String(e));
      }
    }, 300);
    return () => clearTimeout(timer);
  }, [rawInput, updateOutput, showError]);

  const handleFormat = useCallback(async () => {
    if (!rawInput.trim()) return;
    try {
      const result = await invoke<string>('format_json', { input: rawInput });
      updateOutput(result, false);
    } catch (e) {
      showError(String(e));
    }
  }, [rawInput, updateOutput, showError]);

  const handleMinify = useCallback(async () => {
    if (!rawInput.trim()) return;
    try {
      const result = await invoke<string>('minify_json', { input: rawInput });
      updateOutput(result, true);
    } catch (e) {
      showError(String(e));
    }
  }, [rawInput, updateOutput, showError]);

  const handleClear = useCallback(() => {
    setRawInput('');
    setFormattedHtml('');
    setPlainOutput('');
    setIsError(false);
  }, []);

  const handleCopy = useCallback(async () => {
    if (!plainOutput) return;
    try {
      await navigator.clipboard.writeText(plainOutput);
      setIsCopied(true);
      setTimeout(() => setIsCopied(false), 2000);
    } catch {
      // fallback for environments without clipboard API
    }
  }, [plainOutput]);

  const handleOutputKeyDown = useCallback((e: React.KeyboardEvent<HTMLPreElement>) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
      e.preventDefault();
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(e.currentTarget);
      selection?.removeAllRanges();
      selection?.addRange(range);
    }
  }, []);

  const lineCount = plainOutput ? plainOutput.split('\n').length : 0;
  const charCount = plainOutput.length;

  return (
    <div className="w-full h-full flex flex-col">
      <div className="flex justify-between items-center mb-6 border-b th-border pb-4">
        <div className="flex items-center gap-3">
          <h2 className="th-text font-semibold text-lg flex items-center gap-2">
             <span className="text-indigo-400">{'{ }'}</span> {t('JSON Formatter')}
          </h2>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={handleClear}
            className="px-3 py-1.5 text-xs th-bg-surface th-hover-surface th-text-2 rounded font-medium border th-border-subtle transition-colors flex items-center gap-1.5 focus:outline-none"
          >
            <Trash2 className="w-3.5 h-3.5" /> {t('Clear')}
          </button>
          <button
            onClick={handleMinify}
            className="px-3 py-1.5 text-xs th-bg-surface th-hover-surface th-text-2 rounded font-medium border th-border-subtle transition-colors flex items-center gap-1.5 focus:outline-none"
          >
            <MinusSquare className="w-3.5 h-3.5" /> {t('Minify')}
          </button>
          <button
            onClick={handleFormat}
            className="px-3 py-1.5 text-xs bg-indigo-600 hover:bg-indigo-500 text-white rounded font-medium transition-colors shadow-lg shadow-indigo-600/10 flex items-center gap-1.5 focus:outline-none ml-2"
          >
            <AlignLeft className="w-3.5 h-3.5" /> {t('Format JSON')}
          </button>
        </div>
      </div>

      <div className="flex-1 flex gap-4 min-h-0">

        {/* Left: Raw Input */}
        <div className="flex-1 th-bg-card border th-border rounded-xl flex flex-col overflow-hidden shadow-2xl">
          <div className="px-4 py-2 th-bg-surface-h border-b th-border flex justify-between items-center z-10">
            <span className="text-[11px] font-bold th-text-3 uppercase tracking-tighter flex items-center gap-2">
              {t('Raw Input')}
            </span>
            <div className="flex gap-3 text-[10px] th-text-muted font-mono">
              <span>UTF-8</span>
              <span>CRLF</span>
            </div>
          </div>
          <div className="flex-1 flex overflow-hidden relative">
            <div 
              ref={lineNumbersRef}
              className="w-12 flex-shrink-0 py-4 pr-3 border-r th-border th-bg-surface-h th-text-faint text-sm leading-relaxed font-mono text-right overflow-hidden select-none"
            >
              {inputLinesCount ? (
                inputLinesCount.map(num => (
                  <div key={num}>{num}</div>
                ))
              ) : (
                <div className="h-full flex items-center justify-center text-xs opacity-50">•••</div>
              )}
            </div>
            <textarea
              value={rawInput}
              onChange={(e) => {
                if (e.target.value.length > 5 * 1024 * 1024) {
                  showError(t('Input exceeds 5MB limit. Please provide a smaller JSON.'));
                  return;
                }
                setRawInput(e.target.value);
              }}
              onScroll={handleScroll}
              className="flex-1 w-full bg-transparent py-4 px-4 th-text-2 text-sm leading-relaxed font-mono focus:outline-none resize-none placeholder:text-slate-400 dark:placeholder:text-slate-500 whitespace-pre overflow-auto"
              placeholder={t('Paste raw JSON here...')}
              wrap="off"
              spellCheck={false}
            />
          </div>
        </div>

        {/* Right: Parsed Output */}
        <div className="flex-1 th-bg-card border th-border rounded-xl flex flex-col overflow-hidden shadow-2xl">
          <div className="px-4 py-2 th-bg-surface-h border-b th-border flex justify-between items-center">
            <span className="text-[11px] font-bold th-text-3 uppercase tracking-tighter">
              {t('Formatted Output')}
            </span>
            <button
              onClick={handleCopy}
              className={`text-[10px] font-bold transition-colors uppercase flex items-center gap-1 ${isCopied ? 'text-emerald-400' : 'text-indigo-400 hover:text-indigo-300'}`}
            >
              {isCopied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />} {isCopied ? t('Copied!') : t('Copy')}
            </button>
          </div>

          <div className="flex-1 p-4 overflow-y-auto">
            {formattedHtml ? (
              <pre
                tabIndex={0}
                onKeyDown={handleOutputKeyDown}
                className={`text-sm font-mono whitespace-pre-wrap focus:outline-none leading-relaxed select-text ${isError ? 'text-red-400' : 'th-text-2'}`}
                dangerouslySetInnerHTML={{ __html: formattedHtml }}
              />
            ) : (
              <pre className="text-sm font-mono th-text-faint whitespace-pre-wrap leading-relaxed italic">
                {t('Formatted output will appear here...')}
              </pre>
            )}
          </div>
        </div>

      </div>

      <footer className="h-8 border-t th-border mt-4 px-4 th-bg-card flex items-center justify-between text-[10px] th-text-muted rounded-b-xl shadow-inner" style={{ opacity: 0.8 }}>
        <div className="flex flex-row items-center gap-4">
          <span className="flex items-center gap-1.5">
             <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse"></span> {t('System Ready')}
          </span>
          <span>{t('Lines')}: {lineCount}</span>
          <span>{t('Length')}: {charCount} {t('chars')}</span>
        </div>
        <div className="flex items-center gap-2 italic">
          MTOOL Desktop Tools {new Date().getFullYear()}
        </div>
      </footer>
    </div>
  );
}
