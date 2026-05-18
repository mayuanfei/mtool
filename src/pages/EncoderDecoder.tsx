import { useState } from 'react';
import { ArrowRightLeft, Copy, Trash2, ArrowDown, Check } from 'lucide-react';
import { useI18n } from '../i18n';

type Mode = 'base64' | 'url' | 'unicode' | 'html' | 'xml';

export function EncoderDecoder() {
  const { t } = useI18n();
  const [mode, setMode] = useState<Mode>('base64');
  const [input, setInput] = useState('');
  const [output, setOutput] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const handleEncode = () => {
    setError(null);
    try {
      let res = '';
      if (mode === 'base64') {
        res = btoa(unescape(encodeURIComponent(input)));
      } else if (mode === 'url') {
        res = encodeURIComponent(input);
      } else if (mode === 'unicode') {
        res = input.replace(/[^\0-~]/g, (ch) => "\\u" + ("0000" + ch.charCodeAt(0).toString(16)).slice(-4));
      } else if (mode === 'html') {
        res = input
          .replace(/&/g, '&amp;')
          .replace(/</g, '&lt;')
          .replace(/>/g, '&gt;')
          .replace(/"/g, '&quot;')
          .replace(/'/g, '&#39;');
      } else if (mode === 'xml') {
        res = input
          .replace(/&/g, '&amp;')
          .replace(/</g, '&lt;')
          .replace(/>/g, '&gt;')
          .replace(/"/g, '&quot;')
          .replace(/'/g, '&apos;');
      }
      setOutput(res);
    } catch (err: any) {
      setError(err.message || String(err));
    }
  };

  const handleDecode = () => {
    setError(null);
    try {
      let res = '';
      if (mode === 'base64') {
        res = decodeURIComponent(escape(atob(input)));
      } else if (mode === 'url') {
        res = decodeURIComponent(input);
      } else if (mode === 'unicode') {
        res = input.replace(/\\u([0-9a-fA-F]{4})/g, (_, grp) => String.fromCharCode(parseInt(grp, 16)));
      } else if (mode === 'html') {
        const txt = document.createElement('textarea');
        txt.innerHTML = input;
        res = txt.value;
      } else if (mode === 'xml') {
        res = input
          .replace(/&apos;/g, "'")
          .replace(/&quot;/g, '"')
          .replace(/&gt;/g, '>')
          .replace(/&lt;/g, '<')
          .replace(/&amp;/g, '&');
      }
      setOutput(res);
    } catch (err: any) {
      setError(err.message || String(err));
    }
  };

  const copyOutput = async () => {
    if (!output) return;
    try {
      await navigator.clipboard.writeText(output);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy', err);
    }
  };

  const modes: { value: Mode; label: string }[] = [
    { value: 'base64', label: t('Base64') },
    { value: 'url', label: t('URL') },
    { value: 'unicode', label: t('Unicode') },
    { value: 'html', label: t('HTML Entity') },
    { value: 'xml', label: t('XML Entity') },
  ];

  return (
    <div className="w-full h-full flex flex-col min-h-0">
      <div className="flex justify-between items-center mb-6 border-b th-border pb-4 shrink-0">
        <h2 className="th-text font-semibold text-lg flex items-center gap-2">
          <ArrowRightLeft className="w-5 h-5 text-indigo-400" />
          {t('Encoder / Decoder')}
        </h2>
      </div>

      <div className="flex-1 flex flex-col gap-4 min-h-0">
        
        {/* Mode Selector */}
        <div className="flex flex-wrap gap-2 flex-shrink-0 bg-indigo-500/5 p-1 rounded-lg border th-border">
          {modes.map((m) => (
            <button
              key={m.value}
              onClick={() => { setMode(m.value); setInput(''); setOutput(''); setError(null); }}
              className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
                mode === m.value 
                  ? 'bg-indigo-600 text-white shadow-sm' 
                  : 'th-text-2 hover:bg-indigo-500/10'
              }`}
            >
              {m.label}
            </button>
          ))}
        </div>

        <div className="flex-1 flex gap-4 min-h-0">
          
          {/* Input Panel */}
          <div className="flex-1 flex flex-col min-h-0 border th-border rounded-xl overflow-hidden shadow-sm th-bg-card">
            <div className="px-4 py-3 border-b th-border th-bg-surface-h flex items-center justify-between">
              <span className="font-semibold text-sm th-text-2 uppercase tracking-tight">{t('Input')}</span>
              <button 
                onClick={() => { setInput(''); setOutput(''); setError(null); }}
                className="p-1.5 th-text-muted hover:text-rose-400 hover:bg-rose-500/10 rounded transition-colors"
                title={t('Clear Input')}
              >
                <Trash2 className="w-4 h-4" />
              </button>
            </div>
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              className="flex-1 w-full p-4 bg-transparent border-none focus:ring-0 th-text placeholder:text-gray-500/40 resize-none font-mono text-sm leading-relaxed"
              placeholder="..."
              spellCheck={false}
            />
          </div>

          {/* Action Center */}
          <div className="flex flex-col justify-center gap-4 shrink-0 px-2">
            <button
              onClick={handleEncode}
              className="flex flex-col items-center justify-center gap-1 w-24 py-3 bg-indigo-600 hover:bg-indigo-700 text-white rounded-xl shadow-lg transition-all active:scale-95"
            >
              <span className="font-bold text-sm">{t('Encode')}</span>
              <ArrowDown className="w-4 h-4 -rotate-90" />
            </button>
            <button
              onClick={handleDecode}
              className="flex flex-col items-center justify-center gap-1 w-24 py-3 th-bg-surface th-hover-surface border th-border th-text-2 rounded-xl shadow-sm transition-all active:scale-95"
            >
              <ArrowDown className="w-4 h-4 -rotate-90" />
              <span className="font-bold text-sm">{t('Decode')}</span>
            </button>
          </div>

          {/* Output Panel */}
          <div className="flex-1 flex flex-col min-h-0 border th-border rounded-xl overflow-hidden shadow-sm th-bg-card">
            <div className="px-4 py-3 border-b th-border th-bg-surface-h flex items-center justify-between">
              <span className="font-semibold text-sm th-text-2 uppercase tracking-tight">{t('Output')}</span>
              <button 
                onClick={copyOutput}
                className={`p-1.5 rounded transition-colors ${copied ? 'text-emerald-400 bg-emerald-500/10' : 'th-text-muted hover:text-emerald-400 hover:bg-emerald-500/10'}`}
                title={copied ? t('Copied!') : t('Copy Output')}
              >
                {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
              </button>
            </div>
            <div className="flex-1 relative">
              <textarea
                value={output}
                readOnly
                className="absolute inset-0 w-full h-full p-4 bg-transparent border-none focus:ring-0 th-text placeholder:text-gray-500/40 resize-none font-mono text-sm leading-relaxed"
                placeholder="..."
                spellCheck={false}
              />
              {error && (
                <div className="absolute bottom-4 left-4 right-4 p-3 bg-rose-500/10 border border-rose-500/20 text-rose-400 text-sm rounded-lg backdrop-blur-sm shadow-sm break-words">
                  {error}
                </div>
              )}
            </div>
          </div>
          
        </div>
      </div>
    </div>
  );
}
