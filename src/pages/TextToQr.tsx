import { Copy, Download, Clipboard, Trash2, QrCode, Check, XCircle } from 'lucide-react';
import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../i18n';
import { useTheme } from '../theme';

function translateError(err: string, t: (k: any) => string): string {
  if (!err) return '';
  if (err.includes('data too long')) return t('Payload too long (max 2048 chars)');
  if (err.includes('Payload is empty')) return t('Payload is empty');
  return err;
}

export function TextToQr() {
  const { t } = useI18n();
  const { theme } = useTheme();
  const [payload, setPayload] = useState('');
  const [redundancy, setRedundancy] = useState(() => {
    return localStorage.getItem('mtool_qr_redundancy') || 'Q';
  });
  const [resolution, setResolution] = useState(() => {
    const saved = localStorage.getItem('mtool_qr_resolution');
    const val = saved ? parseInt(saved, 10) : 512;
    return isNaN(val) ? 512 : val;
  });
  const [selectedColor, setSelectedColor] = useState(() => {
    return localStorage.getItem('mtool_qr_color') || '#6366f1';
  });

  useEffect(() => {
    localStorage.setItem('mtool_qr_redundancy', redundancy);
  }, [redundancy]);
  useEffect(() => {
    localStorage.setItem('mtool_qr_resolution', resolution.toString());
  }, [resolution]);
  useEffect(() => {
    localStorage.setItem('mtool_qr_color', selectedColor);
  }, [selectedColor]);
  const [qrBase64, setQrBase64] = useState('');
  const [genError, setGenError] = useState('');
  const [isGenerating, setIsGenerating] = useState(false);
  const [isCopied, setIsCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  const [downloadError, setDownloadError] = useState(false);
  const genIdRef = useRef(0);

  const bgColor = getComputedStyle(document.documentElement)
    .getPropertyValue('--bg-card').trim() || (theme === 'dark' ? '#0f172a' : '#ffffff');

  useEffect(() => {
    if (!payload.trim()) {
      setQrBase64('');
      setGenError('');
      return;
    }
    const id = ++genIdRef.current;
    const timer = setTimeout(async () => {
      setIsGenerating(true);
      setGenError('');
      try {
        const result = await invoke<string>('generate_qr', {
          payload,
          redundancy,
          resolution,
          color: selectedColor,
          bgColor
        });
        if (genIdRef.current !== id) return;
        setQrBase64(result);
        setGenError('');
      } catch (err) {
        if (genIdRef.current !== id) return;
        console.error('Failed to generate QR:', err);
        setQrBase64('');
        setGenError(String(err));
      } finally {
        if (genIdRef.current === id) setIsGenerating(false);
      }
    }, 300);
    return () => clearTimeout(timer);
  }, [payload, redundancy, resolution, selectedColor, bgColor, theme]);

  const handleCopyImage = async () => {
    if (!qrBase64) return;
    try {
      await invoke('copy_qr_to_clipboard', { base64Str: qrBase64 });
      setIsCopied(true);
      setTimeout(() => setIsCopied(false), 2000);
    } catch (e) {
      console.error('Failed to copy image', e);
      setCopyError(true);
      setTimeout(() => setCopyError(false), 2000);
    }
  };

  const handleDownload = async () => {
    if (!qrBase64) return;
    try {
      await invoke('download_qr', { base64Str: qrBase64 });
    } catch (e) {
      console.error('Failed to download image', e);
      setDownloadError(true);
      setTimeout(() => setDownloadError(false), 2000);
    }
  };

  return (
    <div className="w-full h-full flex flex-col">
      <div className="mb-6 border-b th-border pb-4">
        <h2 className="th-text font-semibold text-lg flex items-center gap-2">
          <span className="text-indigo-400 px-1"><QrCode className="w-5 h-5" /></span> {t('Text to QR')}
        </h2>
      </div>

      <div className="flex-1 flex flex-col lg:flex-row gap-6 min-h-0">
        
        {/* Left Column: Config */}
        <div className="flex-[3] flex flex-col gap-6 overflow-y-auto pr-2">
          
          {/* Raw Payload */}
          <div className="th-bg-card border th-border rounded-xl p-5 shadow-2xl">
            <div className="flex justify-between items-center mb-3">
              <h3 className="text-[11px] font-bold th-text-3 uppercase tracking-tighter flex items-center gap-2">
                {t('Raw Payload')}
              </h3>
              <span className="text-xs th-text-muted font-mono th-bg-input px-2 py-0.5 rounded border th-border">{payload.length} / 2048 {t('chars')}</span>
            </div>
            
            <textarea
              value={payload}
              onChange={(e) => setPayload(e.target.value)}
              maxLength={2048}
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              className="w-full th-bg-input-alt border th-border-muted rounded-md p-4 th-text-2 text-sm focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 resize-none h-32 mb-3 shadow-inner placeholder:text-slate-400 dark:placeholder:text-slate-500"
              placeholder={t('Enter URL, text, or JSON payload...')}
            />
            
            <div className="flex justify-end gap-2">
               <button className="p-2 th-text-muted hover:th-text th-hover-surface rounded transition-colors border border-transparent" title="Paste from clipboard" onClick={async () => { try { const text = await invoke<string>('read_text_from_clipboard'); setPayload(text); } catch(e){ console.error(e); } }}>
                  <Clipboard className="w-4 h-4" />
               </button>
               <button className="p-2 th-text-muted hover:text-red-400 th-hover-surface rounded transition-colors border border-transparent" title="Clear payload" onClick={() => setPayload('')}>
                  <Trash2 className="w-4 h-4" />
               </button>
            </div>
          </div>

          <div className="flex gap-6">
            {/* Redundancy */}
            <div className="flex-1 th-bg-card border th-border rounded-xl p-5 shadow-2xl">
              <h3 className="text-[11px] font-bold th-text-3 uppercase tracking-tighter flex items-center gap-2 mb-4">
                {t('Redundancy Level')}
              </h3>
              <div className="grid grid-cols-4 gap-2 th-bg-input-alt p-1.5 rounded-lg border th-border-muted">
                {['L', 'M', 'Q', 'H'].map((level) => (
                  <button
                    key={level}
                    onClick={() => setRedundancy(level)}
                    className={`text-xs py-2 rounded font-medium transition-colors ${
                      redundancy === level 
                        ? 'bg-indigo-50 dark:bg-slate-700 text-indigo-600 dark:text-indigo-400 border border-indigo-400 dark:border-indigo-500/40 shadow-sm' 
                        : 'th-text-muted hover:th-text-2 border border-transparent'
                    }`}
                  >
                    {level} <span className={`text-[10px] ${redundancy === level ? 'opacity-70' : 'th-text-muted'}`}>({
                      level === 'L' ? '7%' : level === 'M' ? '15%' : level === 'Q' ? '25%' : '30%'
                    })</span>
                  </button>
                ))}
              </div>
            </div>

            {/* Matrix Resolution */}
            <div className="flex-1 th-bg-card border th-border rounded-xl p-5 shadow-2xl">
              <h3 className="text-[11px] font-bold th-text-3 uppercase tracking-tighter flex items-center gap-2 mb-6">
                {t('Matrix Resolution')}
              </h3>
              
              <div className="px-2">
                 <input 
                    type="range" min="0" max="3" step="1"
                    value={[256, 512, 1024, 2048].indexOf(resolution) !== -1 ? [256, 512, 1024, 2048].indexOf(resolution) : 1}
                    onChange={(e) => setResolution([256, 512, 1024, 2048][parseInt(e.target.value)])}
                    className="qr-slider w-full cursor-pointer"
                 />
                 <div className="flex justify-between text-[10px] th-text-muted mt-3 font-mono">
                     <span className={resolution === 256 ? "text-indigo-600 dark:text-indigo-400 font-semibold" : ""}>256px</span>
                     <span className={resolution === 512 ? "text-indigo-600 dark:text-indigo-400 font-semibold" : ""}>512px</span>
                     <span className={resolution === 1024 ? "text-indigo-600 dark:text-indigo-400 font-semibold" : ""}>1024px</span>
                     <span className={resolution === 2048 ? "text-indigo-600 dark:text-indigo-400 font-semibold" : ""}>2048px</span>
                 </div>
              </div>
            </div>
          </div>
          
          {/* Chromatic Injection */}
          <div className="th-bg-card border th-border rounded-xl p-5 shadow-2xl">
             <h3 className="text-[11px] font-bold th-text-3 uppercase tracking-tighter flex items-center gap-2 mb-4">
                {t('Chromatic Injection')}
             </h3>
             <div className="flex items-center gap-4">
                <div className="flex gap-2">
                   <button onClick={() => setSelectedColor('#6366f1')} className={`w-8 h-8 rounded-full bg-[#6366f1] cursor-pointer transition-all ${selectedColor === '#6366f1' ? 'ring-2 ring-offset-2 ring-indigo-500' : 'hover:ring-2 ring-offset-2'}`} style={{ '--tw-ring-offset-color': 'var(--bg-card)' } as React.CSSProperties}></button>
                   <button onClick={() => setSelectedColor('#34d399')} className={`w-8 h-8 rounded-full bg-[#34d399] cursor-pointer transition-all ${selectedColor === '#34d399' ? 'ring-2 ring-offset-2 ring-emerald-400' : 'hover:ring-2 ring-offset-2'}`} style={{ '--tw-ring-offset-color': 'var(--bg-card)' } as React.CSSProperties}></button>
                   <button onClick={() => setSelectedColor('#f97316')} className={`w-8 h-8 rounded-full bg-[#f97316] cursor-pointer transition-all ${selectedColor === '#f97316' ? 'ring-2 ring-offset-2 ring-orange-400' : 'hover:ring-2 ring-offset-2'}`} style={{ '--tw-ring-offset-color': 'var(--bg-card)' } as React.CSSProperties}></button>
                </div>
                <div className="h-8 w-px" style={{ backgroundColor: 'var(--border-default)' }}></div>
                <div className="flex-1 flex items-center th-bg-input-alt border th-border-muted rounded-md px-3 py-1.5 focus-within:border-indigo-500 transition-colors shadow-inner">
                   <span className="th-text-muted text-xs font-mono mr-2">{t('HEX')}</span>
                   <input type="text" value={selectedColor} onChange={(e) => setSelectedColor(e.target.value)} className="bg-transparent border-none th-text-2 text-sm font-mono focus:outline-none w-full" />
                </div>
             </div>
          </div>

        </div>

        {/* Right Column: Preview */}
        <div className="flex-[2] flex flex-col gap-4">
          <div className="flex-1 th-bg-card border th-border rounded-xl flex flex-col justify-center items-center relative overflow-hidden shadow-2xl">
             
             {/* Corner brackets */}
             <div className="absolute top-4 left-4 w-6 h-6 border-t border-l th-border-muted"></div>
             <div className="absolute top-4 right-4 w-6 h-6 border-t border-r th-border-muted"></div>
             <div className="absolute bottom-4 left-4 w-6 h-6 border-b border-l th-border-muted"></div>
             <div className="absolute bottom-4 right-4 w-6 h-6 border-b border-r th-border-muted"></div>
             
             <div className="absolute top-6 left-0 right-0 flex justify-center">
                <span className="text-[10px] font-bold text-indigo-400 uppercase flex items-center gap-2 px-2 tracking-widest" style={{ backgroundColor: 'var(--bg-card)' }}>
                   <span className="w-1.5 h-1.5 rounded-full bg-indigo-500 animate-pulse"></span> {t('Preview')}
                </span>
             </div>

             <div className="relative mt-8 group">
                <div className="absolute -inset-4 bg-indigo-500/20 blur-2xl rounded-full opacity-0 group-hover:opacity-100 transition-opacity duration-1000"></div>
                <div className="relative w-64 h-64 th-bg-input border th-border rounded-xl p-4 flex items-center justify-center shadow-xl">
                   {genError ? (
                      <div className="text-red-400 text-xs font-mono text-center px-2 overflow-auto max-h-full w-full">
                        <XCircle className="w-8 h-8 mx-auto mb-2 text-red-500/80" />
                        {t('Failed to generate QR Code')}:<br/>
                        <span className="text-[11px] opacity-80 mt-1 inline-block break-words select-text w-full">{translateError(genError, t)}</span>
                      </div>
                   ) : qrBase64 ? (
                      <img 
                        src={`data:image/png;base64,${qrBase64}`} 
                        alt="QR Code" 
                        className={`w-full h-full object-contain ${isGenerating ? 'opacity-50' : 'opacity-100'} transition-opacity`}
                      />
                   ) : (
                      <div className="th-text-faint text-xs font-mono text-center">{t('Enter payload to generate')}</div>
                   )}
                </div>
             </div>
             
             <div className="mt-8 text-center space-y-1 text-xs th-text-muted font-mono">
                <p>{t('Format: PNG')}</p>
                <p>{t('Dimensions')}: {resolution}x{resolution}px</p>
             </div>
          </div>
          
          <div className="flex gap-4">
            <button onClick={handleCopyImage} className={`flex-1 flex items-center justify-center gap-2 py-2.5 rounded-md transition-colors border text-sm font-medium shadow-sm focus:outline-none disabled:opacity-50 ${copyError ? 'bg-red-500/10 text-red-400 border-red-500/20' : isCopied ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20' : 'th-bg-surface th-text-2 th-border-subtle'}`} disabled={!qrBase64}>
              {copyError ? <XCircle className="w-4 h-4" /> : isCopied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
              {copyError ? t('Failed') : isCopied ? t('Copied!') : t('Copy Image')}
            </button>
            <button onClick={handleDownload} className={`flex-1 flex items-center justify-center gap-2 py-2.5 rounded-md transition-colors text-sm font-medium shadow-lg focus:outline-none disabled:opacity-50 ${downloadError ? 'bg-red-500/20 text-red-400 shadow-red-500/10' : 'bg-indigo-600 text-white hover:bg-indigo-500 shadow-indigo-600/20'}`} disabled={!qrBase64}>
              {downloadError ? <XCircle className="w-4 h-4" /> : <Download className="w-4 h-4" />}
              {downloadError ? t('Failed') : t('Download')}
            </button>
          </div>
        </div>

      </div>
    </div>
  );
}
