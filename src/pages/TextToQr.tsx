import { Copy, Download, Clipboard, Trash2, QrCode, Check } from 'lucide-react';
import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../i18n';
import { useTheme } from '../theme';

export function TextToQr() {
  const { t } = useI18n();
  const { theme } = useTheme();
  const [payload, setPayload] = useState('');
  const [redundancy, setRedundancy] = useState('Q');
  const [resolution, setResolution] = useState(512);
  const [selectedColor, setSelectedColor] = useState('#6366f1');
  const [qrBase64, setQrBase64] = useState('');
  const [isGenerating, setIsGenerating] = useState(false);
  const [isCopied, setIsCopied] = useState(false);
  const genIdRef = useRef(0);

  const bgColor = getComputedStyle(document.documentElement)
    .getPropertyValue('--bg-card').trim() || (theme === 'dark' ? '#0f172a' : '#ffffff');

  useEffect(() => {
    if (!payload.trim()) {
      setQrBase64('');
      return;
    }
    const id = ++genIdRef.current;
    const timer = setTimeout(async () => {
      setIsGenerating(true);
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
      } catch (err) {
        if (genIdRef.current !== id) return;
        console.error('Failed to generate QR:', err);
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
    }
  };

  const handleDownload = async () => {
    if (!qrBase64) return;
    try {
      await invoke('download_qr', { base64Str: qrBase64 });
    } catch (e) {
      console.error('Failed to download image', e);
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
              className="w-full th-bg-input-alt border th-border-muted rounded-md p-4 th-text-2 text-sm focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 resize-none h-32 mb-3 shadow-inner"
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
                        ? 'th-bg-surface text-indigo-400 border border-indigo-500/30 shadow-sm' 
                        : 'th-text-muted hover:th-text-2 border border-transparent'
                    }`}
                  >
                    {level} <span className="opacity-50 text-[10px]">({
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
                    className="w-full h-1.5 th-bg-surface rounded-lg appearance-none cursor-pointer accent-indigo-500"
                 />
                 <div className="flex justify-between text-[10px] th-text-muted mt-3 font-mono">
                    <span className={resolution === 256 ? "text-indigo-400" : ""}>256px</span>
                    <span className={resolution === 512 ? "text-indigo-400" : ""}>512px</span>
                    <span className={resolution === 1024 ? "text-indigo-400" : ""}>1024px</span>
                    <span className={resolution === 2048 ? "text-indigo-400" : ""}>2048px</span>
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
                   <button onClick={() => setSelectedColor('#6366f1')} className={`w-8 h-8 rounded-full bg-[#6366f1] cursor-pointer transition-all ${selectedColor === '#6366f1' ? 'ring-2 ring-offset-2 ring-indigo-500' : 'hover:ring-2 ring-offset-2'}`} style={{ ['--tw-ring-offset-color' as any]: 'var(--bg-card)' }}></button>
                   <button onClick={() => setSelectedColor('#34d399')} className={`w-8 h-8 rounded-full bg-[#34d399] cursor-pointer transition-all ${selectedColor === '#34d399' ? 'ring-2 ring-offset-2 ring-emerald-400' : 'hover:ring-2 ring-offset-2'}`} style={{ ['--tw-ring-offset-color' as any]: 'var(--bg-card)' }}></button>
                   <button onClick={() => setSelectedColor('#000000')} className={`w-8 h-8 rounded-full bg-[#000000] border th-border-subtle cursor-pointer transition-all ${selectedColor === '#000000' ? 'ring-2 ring-offset-2' : 'hover:ring-2 ring-offset-2'}`} style={{ ['--tw-ring-offset-color' as any]: 'var(--bg-card)' }}></button>
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
                   {qrBase64 ? (
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
            <button onClick={handleCopyImage} className={`flex-1 flex items-center justify-center gap-2 py-2.5 rounded-md transition-colors border text-sm font-medium shadow-sm focus:outline-none disabled:opacity-50 ${isCopied ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20' : 'th-bg-surface th-text-2 th-border-subtle'}`} disabled={!qrBase64}>
              {isCopied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />} {isCopied ? t('Copied!') : t('Copy Image')}
            </button>
            <button onClick={handleDownload} className="flex-1 flex items-center justify-center gap-2 py-2.5 bg-indigo-600 text-white rounded-md hover:bg-indigo-500 transition-colors text-sm font-medium shadow-lg shadow-indigo-600/20 focus:outline-none disabled:opacity-50" disabled={!qrBase64}>
              <Download className="w-4 h-4" /> {t('Download')}
            </button>
          </div>
        </div>

      </div>
    </div>
  );
}
