import { Copy, MinusSquare, Trash2, AlignLeft, Check } from 'lucide-react';
import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

const BRACE_COLORS = [
  'text-indigo-400',
  'text-emerald-400',
  'text-amber-400',
  'text-rose-400',
  'text-cyan-400',
  'text-violet-400',
];

function escapeHtml(ch: string): string {
  if (ch === '&') return '&amp;';
  if (ch === '<') return '&lt;';
  if (ch === '>') return '&gt;';
  return ch;
}

function highlightBraces(json: string): string {
  let depth = 0;
  let result = '';

  for (const char of json) {
    if (char === '{' || char === '[') {
      result += `<span class="${BRACE_COLORS[depth % BRACE_COLORS.length]}">${char}</span>`;
      depth++;
    } else if (char === '}' || char === ']') {
      depth--;
      result += `<span class="${BRACE_COLORS[depth % BRACE_COLORS.length]}">${char}</span>`;
    } else {
      result += escapeHtml(char);
    }
  }

  return result;
}

export function JsonFormatter() {
  const [rawInput, setRawInput] = useState('');
  const [formattedHtml, setFormattedHtml] = useState('');
  const [plainOutput, setPlainOutput] = useState('');
  const [isError, setIsError] = useState(false);
  const [isCopied, setIsCopied] = useState(false);

  const updateOutput = useCallback((text: string, isMinified: boolean) => {
    setPlainOutput(text);
    if (isMinified) {
      setFormattedHtml(text);
    } else {
      setFormattedHtml(highlightBraces(text));
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
      <div className="flex justify-between items-center mb-6 border-b border-slate-800 pb-4">
        <div className="flex items-center gap-3">
          <h2 className="text-white font-semibold text-lg flex items-center gap-2">
             <span className="text-indigo-400">{'{ }'}</span> JSON Formatter
          </h2>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={handleClear}
            className="px-3 py-1.5 text-xs bg-slate-800 hover:bg-slate-700 text-slate-300 rounded font-medium border border-slate-700 transition-colors flex items-center gap-1.5 focus:outline-none"
          >
            <Trash2 className="w-3.5 h-3.5" /> Clear
          </button>
          <button
            onClick={handleMinify}
            className="px-3 py-1.5 text-xs bg-slate-800 hover:bg-slate-700 text-slate-300 rounded font-medium border border-slate-700 transition-colors flex items-center gap-1.5 focus:outline-none"
          >
            <MinusSquare className="w-3.5 h-3.5" /> Minify
          </button>
          <button
            onClick={handleFormat}
            className="px-3 py-1.5 text-xs bg-indigo-600 hover:bg-indigo-500 text-white rounded font-medium transition-colors shadow-lg shadow-indigo-600/10 flex items-center gap-1.5 focus:outline-none ml-2"
          >
            <AlignLeft className="w-3.5 h-3.5" /> Format JSON
          </button>
        </div>
      </div>

      <div className="flex-1 flex gap-4 min-h-0">

        {/* Left: Raw Input */}
        <div className="flex-1 bg-slate-900 border border-slate-800 rounded-xl flex flex-col overflow-hidden shadow-2xl">
          <div className="px-4 py-2 bg-slate-800/50 border-b border-slate-800 flex justify-between items-center">
            <span className="text-[11px] font-bold text-slate-400 uppercase tracking-tighter flex items-center gap-2">
              Raw Input
            </span>
            <div className="flex gap-3 text-[10px] text-slate-500 font-mono">
              <span>UTF-8</span>
              <span>CRLF</span>
            </div>
          </div>
          <textarea
            value={rawInput}
            onChange={(e) => setRawInput(e.target.value)}
            className="flex-1 w-full bg-transparent p-4 text-indigo-300/80 text-sm font-mono focus:outline-none resize-none"
            placeholder="Paste raw JSON here..."
          />
        </div>

        {/* Right: Parsed Output */}
        <div className="flex-1 bg-slate-900 border border-slate-800 rounded-xl flex flex-col overflow-hidden shadow-2xl">
          <div className="px-4 py-2 bg-slate-800/50 border-b border-slate-800 flex justify-between items-center">
            <span className="text-[11px] font-bold text-slate-400 uppercase tracking-tighter">
              Formatted Output
            </span>
            <button
              onClick={handleCopy}
              className={`text-[10px] font-bold transition-colors uppercase flex items-center gap-1 ${isCopied ? 'text-emerald-400' : 'text-indigo-400 hover:text-indigo-300'}`}
            >
              {isCopied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />} {isCopied ? 'Copied!' : 'Copy'}
            </button>
          </div>

          <div className="flex-1 p-4 overflow-y-auto">
            {formattedHtml ? (
              <pre
                tabIndex={0}
                onKeyDown={handleOutputKeyDown}
                className={`text-sm font-mono whitespace-pre-wrap focus:outline-none leading-relaxed select-text ${isError ? 'text-red-400' : 'text-slate-300'}`}
                dangerouslySetInnerHTML={{ __html: formattedHtml }}
              />
            ) : (
              <pre className="text-sm font-mono text-slate-600 whitespace-pre-wrap leading-relaxed italic">
                Formatted output will appear here...
              </pre>
            )}
          </div>
        </div>

      </div>

      <footer className="h-8 border-t border-slate-800 mt-4 px-4 bg-slate-900/50 flex items-center justify-between text-[10px] text-slate-500 rounded-b-xl shadow-inner">
        <div className="flex flex-row items-center gap-4">
          <span className="flex items-center gap-1.5">
             <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse"></span> System Ready
          </span>
          <span>Lines: {lineCount}</span>
          <span>Length: {charCount} chars</span>
        </div>
        <div className="flex items-center gap-2 italic">
          MTOOL Desktop Tools {new Date().getFullYear()}
        </div>
      </footer>
    </div>
  );
}
