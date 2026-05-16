import { Copy, Check, Database, Trash2, ChevronDown, ChevronUp } from 'lucide-react';
import { useState, useMemo, useEffect } from 'react';
import { useI18n } from '../i18n';

type QuoteStyle = 'single' | 'double' | 'none';

export function SqlInBuilder() {
  const { t } = useI18n();
  const [input, setInput] = useState('');
  const [quoteStyle, setQuoteStyle] = useState<QuoteStyle>(() => {
    return (localStorage.getItem('mtool_sqlin_quote') as QuoteStyle) || 'single';
  });

  useEffect(() => {
    localStorage.setItem('mtool_sqlin_quote', quoteStyle);
  }, [quoteStyle]);
  const [isCopied, setIsCopied] = useState(false);
  const [showDuplicates, setShowDuplicates] = useState(false);

  const { output, inputCount, outputCount } = useMemo(() => {
    if (!input.trim()) return { output: '', inputCount: 0, outputCount: 0 };

    const lines = input
      .split(/[\n\r]+/)
      .map(line => line.trim())
      .filter(line => line.length > 0);

    // Deduplicate while preserving order
    const unique = [...new Set(lines)];

    if (unique.length === 0) return { output: '', inputCount: lines.length, outputCount: 0 };

    let formatted: string;
    switch (quoteStyle) {
      case 'single':
        formatted = unique.map(v => `'${v.replace(/'/g, "''")}'`).join(', ');
        break;
      case 'double':
        formatted = unique.map(v => `"${v.replace(/"/g, '""')}"`).join(', ');
        break;
      case 'none':
        formatted = unique.join(', ');
        break;
    }

    return { 
      output: `(${formatted})`, 
      inputCount: lines.length, 
      outputCount: unique.length 
    };
  }, [input, quoteStyle]);

  const duplicateCount = inputCount - outputCount;

  // Compute which values are duplicated and how many times
  const duplicateDetails = useMemo(() => {
    const lines = input
      .split(/[\n\r]+/)
      .map(l => l.trim())
      .filter(l => l.length > 0);
    const countMap = new Map<string, number>();
    for (const line of lines) {
      countMap.set(line, (countMap.get(line) || 0) + 1);
    }
    return [...countMap.entries()]
      .filter(([, count]) => count > 1)
      .map(([value, count]) => ({ value, count }));
  }, [input]);

  const handleCopy = async () => {
    if (!output) return;
    try {
      await navigator.clipboard.writeText(output);
      setIsCopied(true);
      setTimeout(() => setIsCopied(false), 2000);
    } catch (e) {
      console.error(e);
    }
  };

  const handleClear = () => {
    setInput('');
  };

  return (
    <div className="w-full h-full flex flex-col min-h-0">
      <div className="flex justify-between items-center mb-6 border-b th-border pb-4 shrink-0">
        <h2 className="th-text font-semibold text-lg flex items-center gap-2">
          <Database className="w-5 h-5 text-indigo-400" />
          {t('SQL IN Builder')}
        </h2>
      </div>

      <div className="flex-1 flex flex-col min-h-0 overflow-y-auto pr-2 pb-6 space-y-6">

        {/* Quote Style Card */}
        <section className="th-bg-card border th-border rounded-xl p-6 shadow-xl shrink-0">
          <span className="text-sm font-bold th-text-2 block mb-3">{t('Quote Style')}</span>
          <div className="flex gap-3">
            {([
              { value: 'single' as QuoteStyle, label: t("Single Quote") + " ( ' )" },
              { value: 'double' as QuoteStyle, label: t("Double Quote") + ' ( " )' },
              { value: 'none' as QuoteStyle, label: t("No Quote") },
            ]).map(opt => (
              <button
                key={opt.value}
                onClick={() => setQuoteStyle(opt.value)}
                className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors border focus:outline-none ${
                  quoteStyle === opt.value
                    ? 'bg-indigo-600/15 text-indigo-400 border-indigo-500/30 shadow-sm'
                    : 'th-bg-input-alt th-text-3 th-border-subtle th-hover-surface'
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </section>

        {/* Input / Output Grid */}
        <section className="flex-1 grid grid-cols-1 md:grid-cols-2 gap-6 min-h-[24rem]">

          {/* Input Panel */}
          <div className="th-bg-card border th-border rounded-xl p-6 shadow-xl flex flex-col min-h-0">
            <div className="flex items-center justify-between mb-3 shrink-0">
              <div className="flex items-center gap-2">
                <Database className="w-4 h-4 text-indigo-400" />
                <span className="text-[11px] font-bold th-text-3 uppercase tracking-widest">{t('Input Values')}</span>
              </div>
              <button
                onClick={handleClear}
                className="p-1.5 th-text-muted hover:th-text th-hover-surface rounded transition-colors focus:outline-none"
                title={t('Clear')}
              >
                <Trash2 className="w-4 h-4" />
              </button>
            </div>
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder={t('Paste one value per line...')}
              className="flex-1 w-full th-bg-input border th-border rounded-lg p-4 text-sm font-mono th-text-2 resize-none focus:outline-none focus:border-indigo-500 transition-colors placeholder:text-slate-400 dark:placeholder:text-slate-500 min-h-[12rem]"
              spellCheck={false}
            />
            <div className="mt-3 text-xs th-text-muted font-mono shrink-0 flex items-center justify-between">
              <div>
                {inputCount} {t('items')}
                {duplicateCount > 0 && (
                  <button
                    onClick={() => setShowDuplicates(!showDuplicates)}
                    className="text-amber-500 ml-2 hover:text-amber-400 transition-colors cursor-pointer inline-flex items-center gap-1 focus:outline-none"
                  >
                    ({duplicateCount} {t('duplicates removed')})
                    {showDuplicates
                      ? <ChevronUp className="w-3 h-3" />
                      : <ChevronDown className="w-3 h-3" />
                    }
                  </button>
                )}
              </div>
            </div>
            {showDuplicates && duplicateDetails.length > 0 && (
              <div className="mt-2 th-bg-input border border-amber-500/20 rounded-lg p-3 space-y-1.5 max-h-32 overflow-y-auto shrink-0">
                {duplicateDetails.map((d, i) => (
                  <div key={i} className="flex items-center justify-between text-xs font-mono">
                    <span className="text-amber-400 truncate mr-3">{d.value}</span>
                    <span className="th-text-muted shrink-0">×{d.count}</span>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Output Panel */}
          <div className="th-bg-card border th-border rounded-xl p-6 shadow-xl flex flex-col min-h-0">
            <div className="flex items-center justify-between mb-3 shrink-0">
              <div className="flex items-center gap-2">
                <Database className="w-4 h-4 text-emerald-400" />
                <span className="text-[11px] font-bold th-text-3 uppercase tracking-widest">{t('SQL Output')}</span>
              </div>
              <button
                onClick={handleCopy}
                disabled={!output}
                className="px-3 py-1.5 bg-indigo-600 hover:bg-indigo-500 disabled:bg-[var(--bg-surface)] disabled:text-[var(--text-muted)] disabled:border disabled:border-[var(--border-subtle)] disabled:shadow-none text-white rounded-lg text-xs font-medium transition-colors flex items-center gap-1.5 focus:outline-none shadow-lg shadow-indigo-600/20"
              >
                {isCopied ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
                {isCopied ? t('Copied!') : t('Copy')}
              </button>
            </div>
            <div className="flex-1 w-full th-bg-input border th-border rounded-lg p-4 text-sm font-mono text-emerald-700 dark:text-emerald-400 overflow-y-auto break-all whitespace-pre-wrap min-h-[12rem]">
              {output || <span className="th-text-faint">{t('SQL IN clause will appear here...')}</span>}
            </div>
            <div className="mt-3 text-xs th-text-muted font-mono shrink-0">
              {outputCount} {t('unique values')}
            </div>
          </div>

        </section>

        {/* Preview */}
        {output && (
          <section className="th-bg-card border th-border rounded-xl p-6 shadow-xl shrink-0">
            <span className="text-[11px] font-bold th-text-3 uppercase tracking-widest block mb-3">{t('SQL Preview')}</span>
            <pre className="text-sm font-mono th-text-2 overflow-x-auto whitespace-pre-wrap break-all th-bg-input border th-border rounded-lg p-4">
              <span className="text-blue-400">WHERE</span> <span className="th-text-2">column_name</span> <span className="text-blue-400">IN</span> <span className="text-emerald-700 dark:text-emerald-400">{output}</span>
            </pre>
          </section>
        )}

      </div>
    </div>
  );
}
