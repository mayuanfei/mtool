import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { 
  FileUp, 
  Settings2, 
  HelpCircle, 
  CheckCircle2, 
  XCircle, 
  Clock, 
  FolderOpen, 
  ExternalLink,
  ChevronDown,
  ChevronUp,
  Download,
  AlertCircle
} from 'lucide-react';
import { useI18n } from '../i18n';

interface InstallProgress {
  stage: 'not_started' | 'downloading' | 'extracting' | 'success' | 'failed';
  progress: number;
  message: string;
}

interface HistoryItem {
  id: string;
  time: string;
  inputPath: string;
  outputPath: string;
  fromFormat: string;
  toFormat: string;
  status: 'success' | 'failed';
  error?: string;
}

const SUPPORTED_INPUTS = [
  { value: 'auto', label: 'Auto Detect' },
  { value: 'md', label: 'Markdown (.md)' },
  { value: 'docx', label: 'Word (.docx)' },
  { value: 'html', label: 'HTML (.html)' },
  { value: 'latex', label: 'LaTeX (.tex)' },
  { value: 'epub', label: 'EPUB (.epub)' },
  { value: 'pptx', label: 'PowerPoint (.pptx)' },
  { value: 'txt', label: 'Text (.txt)' }
];

const SUPPORTED_OUTPUTS = [
  { value: 'docx', label: 'Word (.docx)' },
  { value: 'md', label: 'Markdown (.md)' },
  { value: 'pdf', label: 'PDF (.pdf)' },
  { value: 'html', label: 'HTML (.html)' },
  { value: 'epub', label: 'EPUB (.epub)' },
  { value: 'latex', label: 'LaTeX (.tex)' },
  { value: 'pptx', label: 'PowerPoint (.pptx)' }
];

export function DocConverter() {
  const { t } = useI18n();

  // Statuses
  const [pandocStatus, setPandocStatus] = useState<'checking' | 'detected' | 'not_installed'>('checking');
  const [pandocVersion, setPandocVersion] = useState('');
  
  // Install progress
  const [installing, setInstalling] = useState(false);
  const [installProgress, setInstallProgress] = useState(0);
  const [installMessage, setInstallMessage] = useState('');
  const [installError, setInstallError] = useState<string | null>(null);

  // Load last configuration from cache
  const [savedConfig] = useState(() => {
    try {
      const saved = localStorage.getItem('mtool_pandoc_last_config');
      return saved ? JSON.parse(saved) : null;
    } catch {
      return null;
    }
  });

  // Form states
  const [sourcePath, setSourcePath] = useState(savedConfig?.sourcePath || '');
  const [targetPath, setTargetPath] = useState(savedConfig?.targetPath || '');
  const [fromFormat, setFromFormat] = useState(savedConfig?.fromFormat || 'auto');
  const [toFormat, setToFormat] = useState(savedConfig?.toFormat || 'docx');
  const [extraArgs, setExtraArgs] = useState(savedConfig?.extraArgs || '');
  const [showAdvanced, setShowAdvanced] = useState(!!savedConfig?.extraArgs);

  // Auto-save form configuration changes
  useEffect(() => {
    try {
      localStorage.setItem('mtool_pandoc_last_config', JSON.stringify({
        sourcePath,
        targetPath,
        fromFormat,
        toFormat,
        extraArgs
      }));
    } catch (e) {
      console.error('Failed to save pandoc config:', e);
    }
  }, [sourcePath, targetPath, fromFormat, toFormat, extraArgs]);

  // Convert states
  const [converting, setConverting] = useState(false);
  const [convertResult, setConvertResult] = useState<{ success: boolean; message: string } | null>(null);

  // History state
  const [history, setHistory] = useState<HistoryItem[]>(() => {
    try {
      const saved = localStorage.getItem('mtool_pandoc_history');
      return saved ? JSON.parse(saved) : [];
    } catch {
      return [];
    }
  });

  const checkStatus = useCallback(async () => {
    setPandocStatus('checking');
    try {
      const res = await invoke<string>('check_pandoc');
      if (res === 'not_installed') {
        setPandocStatus('not_installed');
        // Check if there is an active background download to inherit
        const installStatus = await invoke<InstallProgress>('get_pandoc_install_status');
        if (installStatus && (installStatus.stage === 'downloading' || installStatus.stage === 'extracting')) {
          setInstalling(true);
          setInstallProgress(installStatus.progress);
          setInstallMessage(t(installStatus.message as any) || installStatus.message);
        }
      } else {
        setPandocStatus('detected');
        setPandocVersion(res);
      }
    } catch (e) {
      setPandocStatus('not_installed');
      console.error(e);
    }
  }, [t]);

  useEffect(() => {
    checkStatus();
  }, [checkStatus]);

  // Global listener for background progress
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    
    const setupListener = async () => {
      unlisten = await listen<InstallProgress>('pandoc_install_progress', (event) => {
        const { stage, progress, message } = event.payload;
        setInstallProgress(progress);
        setInstallMessage(t(message as any) || message);
        if (stage === 'success') {
          setPandocStatus('detected');
          setInstalling(false);
          // Refetch version to update local UI
          invoke<string>('check_pandoc').then(ver => {
            if (ver !== 'not_installed') setPandocVersion(ver);
          }).catch(console.error);
        } else if (stage === 'failed') {
          setInstalling(false);
          setInstallError(message);
        } else if (stage === 'downloading' || stage === 'extracting') {
          setInstalling(true);
        }
      });
    };

    setupListener();

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, [t]);

  const handleInstall = async () => {
    if (installing) return;
    setInstalling(true);
    setInstallProgress(0);
    setInstallMessage(t('Initializing download...'));
    setInstallError(null);

    try {
      await invoke('install_pandoc');
    } catch (e) {
      setInstalling(false);
      setInstallError(String(e));
    }
  };

  // Helper to extract file name and suggest target name
  const handleSelectSource = async () => {
    try {
      const path = await invoke<string>('select_source_file');
      if (path) {
        setSourcePath(path);
        
        // Infer formats and set suggested target output path
        const ext = path.split('.').pop()?.toLowerCase() || '';
        if (SUPPORTED_INPUTS.some(i => i.value === ext)) {
          setFromFormat(ext);
        } else {
          setFromFormat('auto');
        }

        // Generate target suggestion
        const separator = path.includes('\\') ? '\\' : '/';
        const parts = path.split(separator);
        const fullName = parts.pop() || '';
        const nameWithoutExt = fullName.substring(0, fullName.lastIndexOf('.')) || fullName;
        
        const outputExt = toFormat;
        const parentPath = parts.join(separator);
        const suggestedTarget = parentPath ? `${parentPath}${separator}${nameWithoutExt}.${outputExt}` : `${nameWithoutExt}.${outputExt}`;
        setTargetPath(suggestedTarget);
      }
    } catch (e) {
      console.error('File select error:', e);
    }
  };

  // Handle manual saving path selection
  const handleSelectTarget = async () => {
    try {
      let defaultName = 'converted_output';
      if (sourcePath) {
        const separator = sourcePath.includes('\\') ? '\\' : '/';
        const fullName = sourcePath.split(separator).pop() || '';
        const nameWithoutExt = fullName.substring(0, fullName.lastIndexOf('.')) || fullName;
        defaultName = `${nameWithoutExt}.${toFormat}`;
      } else {
        defaultName = `untitled.${toFormat}`;
      }

      const path = await invoke<string>('select_target_file', { defaultName });
      if (path) {
        setTargetPath(path);
      }
    } catch (e) {
      console.error('Save target select error:', e);
    }
  };

  // Form change output extension updates
  const handleToFormatChange = (newFormat: string) => {
    setToFormat(newFormat);
    if (targetPath) {
      const lastDotIdx = targetPath.lastIndexOf('.');
      if (lastDotIdx !== -1) {
        setTargetPath(`${targetPath.substring(0, lastDotIdx)}.${newFormat}`);
      }
    }
  };

  // Trigger Pandoc Conversion
  const handleConvert = async () => {
    if (!sourcePath) {
      setConvertResult({ success: false, message: t('Please select source file') });
      return;
    }
    if (!targetPath) {
      setConvertResult({ success: false, message: t('Please select target path') });
      return;
    }

    setConverting(true);
    setConvertResult(null);
    setExtraArgs(extraArgs.trim());

    // Use regex to parse CLI arguments, keeping space-containing strings inside quotes together.
    const matches = extraArgs.trim().match(/(?:[^\s"']+|"[^"\\]*(?:\\.[^"\\]*)*"|'[^'\\]*(?:\\.[^'\\]*)*')+/g);
    const argsList = matches ? matches.map((arg: string) => {
      let cleaned = arg;
      // 1. Strip outer double or single quotes if the entire token is wrapped
      if ((cleaned.startsWith('"') && cleaned.endsWith('"')) || (cleaned.startsWith("'") && cleaned.endsWith("'"))) {
        cleaned = cleaned.slice(1, -1);
      }
      // 2. Defensive check: Strip inner quotes inside key=value format (e.g., mainfont="Arial" -> mainfont=Arial)
      // This prevents nested quotes in Pandoc templates (like ""Arial"") which breaks Typst compiler.
      const eqIndex = cleaned.indexOf('=');
      if (eqIndex !== -1) {
        const key = cleaned.slice(0, eqIndex);
        let val = cleaned.slice(eqIndex + 1);
        if ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) {
          val = val.slice(1, -1);
        }
        cleaned = `${key}=${val}`;
      }
      return cleaned;
    }) : [];

    try {
      await invoke('run_pandoc_convert', {
        inputPath: sourcePath,
        outputPath: targetPath,
        fromFormat: fromFormat === 'auto' ? null : fromFormat,
        toFormat,
        extraArgs: argsList.length > 0 ? argsList : null
      });

      setConvertResult({ success: true, message: t('Conversion Successful!') });
      
      // Update history
      const newHistoryItem: HistoryItem = {
        id: crypto.randomUUID(),
        time: new Date().toLocaleTimeString(),
        inputPath: sourcePath,
        outputPath: targetPath,
        fromFormat,
        toFormat,
        status: 'success'
      };

      setHistory(prev => {
        const updated = [newHistoryItem, ...prev].slice(0, 10);
        localStorage.setItem('mtool_pandoc_history', JSON.stringify(updated));
        return updated;
      });

    } catch (e) {
      const errMsg = String(e);
      setConvertResult({ success: false, message: `${t('Conversion Failed')}: ${errMsg}` });

      const newHistoryItem: HistoryItem = {
        id: crypto.randomUUID(),
        time: new Date().toLocaleTimeString(),
        inputPath: sourcePath,
        outputPath: targetPath,
        fromFormat,
        toFormat,
        status: 'failed',
        error: errMsg
      };

      setHistory(prev => {
        const updated = [newHistoryItem, ...prev].slice(0, 10);
        localStorage.setItem('mtool_pandoc_history', JSON.stringify(updated));
        return updated;
      });
    } finally {
      setConverting(false);
    }
  };

  // Quick action helpers
  const handleOpenFile = async (path: string) => {
    try {
      await invoke('open_file', { path });
    } catch (e) {
      console.error(e);
    }
  };

  const handleShowInFolder = async (path: string) => {
    try {
      await invoke('reveal_in_explorer', { path });
    } catch (e) {
      console.error(e);
    }
  };

  const clearHistory = () => {
    setHistory([]);
    localStorage.removeItem('mtool_pandoc_history');
  };

  const getFormatLabel = (val: string) => {
    return SUPPORTED_INPUTS.find(i => i.value === val)?.label || val.toUpperCase();
  };

  return (
    <div className="max-w-4xl mx-auto w-full">
      {/* Header */}
      <div className="mb-8 flex items-center justify-between">
        <div>
          <h1 className="text-4xl font-bold tracking-tight th-text mb-2">{t('Doc Converter')}</h1>
          <p className="th-text-3">{t('Convert document formats using Pandoc.')}</p>
        </div>

        {/* Pandoc Status pill */}
        <div className="flex items-center gap-2 px-3 py-1.5 rounded-full th-bg-surface border th-border text-xs font-medium animate-fade-in">
          <span className="th-text-muted">{t('Pandoc Status')}:</span>
          {pandocStatus === 'checking' && (
            <span className="flex items-center gap-1.5 text-amber-400">
              <span className="w-2 h-2 rounded-full bg-amber-400 animate-pulse" />
              {t('Detecting...')}
            </span>
          )}
          {pandocStatus === 'detected' && (
            <span 
              className="flex items-center gap-1.5 text-emerald-400 cursor-help"
              title={pandocVersion}
            >
              <span className="w-2 h-2 rounded-full bg-emerald-400" />
              {t('Detected')}
            </span>
          )}
          {pandocStatus === 'not_installed' && (
            <span className="flex items-center gap-1.5 text-rose-400">
              <span className="w-2 h-2 rounded-full bg-rose-400" />
              {t('Not Installed')}
            </span>
          )}
        </div>
      </div>

      {/* Main panel */}
      {pandocStatus === 'not_installed' ? (
        <div className="space-y-6">
          {/* Missing Plugin Warning / Auto Installer */}
          <div className="th-bg-card border th-border rounded-xl p-6 shadow-xl flex flex-col md:flex-row gap-6 items-start">
            <div className="w-12 h-12 rounded-full bg-indigo-500/10 flex items-center justify-center text-indigo-400 shrink-0 shadow-inner">
              <Download className="w-6 h-6" />
            </div>
            
            <div className="flex-1 space-y-4">
              <div>
                <h3 className="text-lg font-bold th-text-2 mb-2">{t('Install Pandoc')}</h3>
                <p className="text-sm th-text-muted leading-relaxed">
                  {t('Necessary plugin (Pandoc) is missing. Click \'Install\' to automatically download and configure it. You can also install it manually on your system.')}
                </p>
              </div>

              {installing ? (
                <div className="space-y-2 max-w-md bg-indigo-500/5 border border-indigo-500/10 p-4 rounded-xl">
                  <div className="flex justify-between text-xs font-semibold th-text-2">
                    <span className="truncate">{installMessage}</span>
                    <span>{installProgress}%</span>
                  </div>
                  <div className="w-full bg-zinc-800 rounded-full h-2 overflow-hidden shadow-inner">
                    <div 
                      className="bg-indigo-500 h-2 rounded-full transition-all duration-300"
                      style={{ width: `${installProgress}%` }}
                    />
                  </div>
                </div>
              ) : (
                <button
                  onClick={handleInstall}
                  className="px-5 py-2.5 bg-indigo-600 hover:bg-indigo-700 text-white font-medium rounded-lg text-sm transition-colors shadow-lg shadow-indigo-600/20"
                >
                  {t('Install Plugin')}
                </button>
              )}

              {installError && (
                <div className="p-3.5 bg-rose-950/20 border border-rose-800/40 rounded-xl text-xs text-rose-300 flex items-center gap-2 animate-fade-in">
                  <AlertCircle className="w-4 h-4 shrink-0 text-rose-400" />
                  <span>{t('Plugin Installation Failed')}: {installError}</span>
                </div>
              )}
            </div>
          </div>

          {/* Manual Installation Guide */}
          <div className="th-bg-card border th-border rounded-xl p-6 shadow-md space-y-4">
            <h4 className="text-sm font-bold tracking-wider th-text-muted uppercase flex items-center gap-2">
              <HelpCircle className="w-4 h-4 text-indigo-400" />
              {t('Pandoc is not installed on your system. Please follow the instructions below to install it:')}
            </h4>

            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <div className="p-4 rounded-xl th-bg-surface border th-border">
                <p className="font-semibold text-xs text-indigo-400 uppercase tracking-widest mb-2">macOS</p>
                <p className="text-xs th-text-2 mb-3 leading-relaxed">{t('macOS Installation')}</p>
                <code className="block select-all bg-black/30 p-2 rounded text-[11px] font-mono th-text-muted break-all">brew install pandoc</code>
              </div>
              <div className="p-4 rounded-xl th-bg-surface border th-border">
                <p className="font-semibold text-xs text-indigo-400 uppercase tracking-widest mb-2">Windows</p>
                <p className="text-xs th-text-2 mb-3 leading-relaxed">{t('Windows Installation')}</p>
                <code className="block select-all bg-black/30 p-2 rounded text-[11px] font-mono th-text-muted break-all">winget install jgm.pandoc</code>
              </div>
              <div className="p-4 rounded-xl th-bg-surface border th-border">
                <p className="font-semibold text-xs text-indigo-400 uppercase tracking-widest mb-2">Linux</p>
                <p className="text-xs th-text-2 mb-3 leading-relaxed">{t('Linux Installation')}</p>
                <code className="block select-all bg-black/30 p-2 rounded text-[11px] font-mono th-text-muted break-all">sudo apt install pandoc</code>
              </div>
            </div>

            <div className="pt-2 flex justify-end">
              <button 
                onClick={checkStatus}
                className="px-3.5 py-1.5 text-xs font-semibold th-text-3 border th-border rounded-lg th-hover-surface transition-colors"
              >
                {t('Check for Updates')}
              </button>
            </div>
          </div>
        </div>
      ) : (
        <div className="space-y-6">
          {/* Converter Form */}
          <div className="th-bg-card border th-border rounded-xl p-6 shadow-2xl space-y-6">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
              
              {/* Input path */}
              <div className="space-y-2 md:col-span-2">
                <label className="block text-sm font-semibold th-text-2">{t('Source File')}</label>
                <div className="flex gap-3">
                  <input
                    type="text"
                    readOnly
                    placeholder={t('Please select source file')}
                    value={sourcePath}
                    onClick={handleSelectSource}
                    className="flex-1 px-4 py-2.5 rounded-lg th-bg-input border th-border-subtle th-text-2 text-sm focus:outline-none cursor-pointer th-hover-surface transition-colors truncate font-mono"
                  />
                  <button
                    onClick={handleSelectSource}
                    className="px-4 py-2.5 bg-indigo-600/10 hover:bg-indigo-600/25 border border-indigo-500/20 text-indigo-400 font-medium rounded-lg text-sm transition-colors shrink-0"
                  >
                    {t('Select File')}
                  </button>
                </div>
              </div>

              {/* Formats selectors */}
              <div className="space-y-2">
                <label className="block text-sm font-semibold th-text-2">{t('From Format')}</label>
                <div className="relative">
                  <select
                    value={fromFormat}
                    onChange={(e) => setFromFormat(e.target.value)}
                    className="appearance-none w-full th-bg-input border th-border-subtle th-text-2 text-sm rounded-lg focus:ring-indigo-500 focus:border-indigo-500 focus:outline-none p-2.5 pr-10 cursor-pointer shadow-inner"
                  >
                    {SUPPORTED_INPUTS.map(opt => (
                      <option key={opt.value} value={opt.value}>
                        {opt.label === 'Auto Detect' ? t('Auto Detect') : opt.label}
                      </option>
                    ))}
                  </select>
                  <div className="pointer-events-none absolute inset-y-0 right-0 flex items-center px-3 th-text-3">
                    <ChevronDown className="w-4 h-4" />
                  </div>
                </div>
              </div>

              <div className="space-y-2">
                <label className="block text-sm font-semibold th-text-2">{t('To Format')}</label>
                <div className="relative">
                  <select
                    value={toFormat}
                    onChange={(e) => handleToFormatChange(e.target.value)}
                    className="appearance-none w-full th-bg-input border th-border-subtle th-text-2 text-sm rounded-lg focus:ring-indigo-500 focus:border-indigo-500 focus:outline-none p-2.5 pr-10 cursor-pointer shadow-inner"
                  >
                    {SUPPORTED_OUTPUTS.map(opt => (
                      <option key={opt.value} value={opt.value}>
                        {opt.label}
                      </option>
                    ))}
                  </select>
                  <div className="pointer-events-none absolute inset-y-0 right-0 flex items-center px-3 th-text-3">
                    <ChevronDown className="w-4 h-4" />
                  </div>
                </div>
              </div>

              {toFormat === 'pdf' && (
                <div className="md:col-span-2 p-4 bg-amber-500/10 border border-amber-500/20 rounded-lg text-xs th-text-muted flex items-start gap-2.5 animate-fade-in shadow-inner">
                  <AlertCircle className="w-4 h-4 text-amber-500 shrink-0 mt-0.5" />
                  <span className="leading-relaxed th-text-2">
                    {t('Converting to PDF requires a PDF engine on your system (defaults to pdflatex). If you encounter a pdflatex missing error, you can specify an alternative engine in "Custom Arguments" below (e.g. --pdf-engine=wkhtmltopdf or --pdf-engine=typst). Note: For Typst 0.14+, you may also need to append a main font argument, e.g., -V mainfont="Microsoft YaHei" or -V mainfont="Arial".')}
                  </span>
                </div>
              )}

              {/* Target path */}
              <div className="space-y-2 md:col-span-2">
                <label className="block text-sm font-semibold th-text-2">{t('Target File')}</label>
                <div className="flex gap-3">
                  <input
                    type="text"
                    readOnly
                    placeholder={t('Please select target path')}
                    value={targetPath}
                    onClick={handleSelectTarget}
                    className="flex-1 px-4 py-2.5 rounded-lg th-bg-input border th-border-subtle th-text-2 text-sm focus:outline-none cursor-pointer th-hover-surface transition-colors truncate font-mono"
                  />
                  <button
                    onClick={handleSelectTarget}
                    className="px-4 py-2.5 bg-indigo-600/10 hover:bg-indigo-600/25 border border-indigo-500/20 text-indigo-400 font-medium rounded-lg text-sm transition-colors shrink-0"
                  >
                    {t('Save As')}
                  </button>
                </div>
              </div>
            </div>

            {/* Advanced Configuration Accordion */}
            <div className="border th-border rounded-lg th-bg-surface overflow-hidden">
              <button
                type="button"
                onClick={() => setShowAdvanced(!showAdvanced)}
                className="w-full flex items-center justify-between px-4 py-3.5 font-semibold text-xs th-text-2 tracking-wide uppercase transition-colors th-hover-surface border-b th-border"
              >
                <span className="flex items-center gap-2">
                  <Settings2 className="w-4 h-4 text-indigo-400" />
                  {t('Custom Arguments')}
                </span>
                {showAdvanced ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4 text-indigo-400" />}
              </button>

              {showAdvanced && (
                <div className="p-4 space-y-2">
                  <input
                    type="text"
                    placeholder={t('e.g., --pdf-engine=xelatex')}
                    value={extraArgs}
                    onChange={(e) => setExtraArgs(e.target.value)}
                    className="w-full px-4 py-2.5 rounded-lg th-bg-input border th-border-subtle th-text-2 text-sm focus:outline-none font-mono"
                  />
                  <p className="text-[11px] th-text-muted leading-relaxed">
                    * {t('Specify command line arguments for Pandoc. Use spaces to split multiple flags.')}
                  </p>
                </div>
              )}
            </div>

            {/* Submit convert action */}
            <div className="pt-2 flex flex-col items-stretch gap-4">
              <button
                onClick={handleConvert}
                disabled={converting || !sourcePath}
                className="w-full py-3.5 bg-indigo-600 hover:bg-indigo-700 disabled:opacity-50 text-white font-bold rounded-xl transition-all shadow-xl shadow-indigo-600/25 flex items-center justify-center gap-2"
              >
                {converting ? (
                  <>
                    <span className="w-4.5 h-4.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                    {t('Converting...')}
                  </>
                ) : (
                  <>
                    <FileUp className="w-4.5 h-4.5" />
                    {t('Convert')}
                  </>
                )}
              </button>

              {/* Conversion Output Result Card */}
              {convertResult && (
                <div 
                  className={`p-4 rounded-xl border animate-fade-in shadow-lg ${
                    convertResult.success 
                      ? 'bg-emerald-950/20 border-emerald-800/40 text-emerald-300' 
                      : 'bg-rose-950/20 border-rose-800/40 text-rose-300'
                  }`}
                >
                  <div className="flex gap-3 items-start">
                    {convertResult.success ? (
                      <CheckCircle2 className="w-5 h-5 shrink-0 text-emerald-400 mt-0.5" />
                    ) : (
                      <XCircle className="w-5 h-5 shrink-0 text-rose-400 mt-0.5" />
                    )}
                    
                    <div className="flex-1 space-y-3">
                      <p className="text-sm font-semibold">{convertResult.message}</p>
                      {convertResult.success && targetPath && (
                        <div className="flex flex-wrap gap-2.5">
                          <button
                            onClick={() => handleOpenFile(targetPath)}
                            className="px-3 py-1.5 text-xs font-semibold bg-emerald-600/20 border border-emerald-500/20 rounded-md text-emerald-300 hover:bg-emerald-600/35 transition-colors flex items-center gap-1.5"
                          >
                            <ExternalLink className="w-3.5 h-3.5" />
                            {t('Open File')}
                          </button>
                          <button
                            onClick={() => handleShowInFolder(targetPath)}
                            className="px-3 py-1.5 text-xs font-semibold bg-emerald-600/20 border border-emerald-500/20 rounded-md text-emerald-300 hover:bg-emerald-600/35 transition-colors flex items-center gap-1.5"
                          >
                            <FolderOpen className="w-3.5 h-3.5" />
                            {t('Show in Folder')}
                          </button>
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* History Panel */}
          <div className="th-bg-card border th-border rounded-xl p-6 shadow-md space-y-4">
            <div className="flex items-center justify-between border-b th-border pb-3">
              <h3 className="text-sm font-bold tracking-wider th-text-muted uppercase flex items-center gap-2">
                <Clock className="w-4 h-4 text-indigo-400" />
                {t('Recent History')}
              </h3>
              {history.length > 0 && (
                <button
                  onClick={clearHistory}
                  className="text-xs th-text-muted hover:th-text-2 transition-colors cursor-pointer"
                >
                  {t('Clear')}
                </button>
              )}
            </div>

            {history.length === 0 ? (
              <p className="text-sm th-text-muted text-center py-6">{t('No conversion history yet.')}</p>
            ) : (
              <div className="divide-y th-divide overflow-hidden rounded-lg border th-border">
                {history.map((item) => (
                  <div 
                    key={item.id} 
                    className="p-4 th-bg-surface hover:th-bg-surface-h transition-colors flex flex-col md:flex-row gap-4 justify-between md:items-center"
                  >
                    <div className="space-y-1.5 min-w-0 flex-1">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span className="text-xs font-mono th-text-muted">{item.time}</span>
                        <span className="text-xs px-2 py-0.5 rounded-full th-bg-input border th-border-subtle th-text-2 font-medium">
                          {getFormatLabel(item.fromFormat)} &rarr; {getFormatLabel(item.toFormat)}
                        </span>
                        {item.status === 'success' ? (
                          <span className="text-[10px] px-1.5 py-0.5 bg-emerald-600/15 border border-emerald-500/20 text-emerald-400 rounded font-semibold uppercase tracking-wider">
                            {t('Success')}
                          </span>
                        ) : (
                          <span 
                            className="text-[10px] px-1.5 py-0.5 bg-rose-600/15 border border-rose-500/20 text-rose-400 rounded font-semibold uppercase tracking-wider cursor-help"
                            title={item.error}
                          >
                            {t('Failed')}
                          </span>
                        )}
                      </div>

                      <div className="space-y-0.5 text-xs truncate">
                        <p className="th-text-muted truncate">
                          <span className="font-semibold">{t('Source File')}:</span> {item.inputPath}
                        </p>
                        <p className="th-text-muted truncate">
                          <span className="font-semibold">{t('Target File')}:</span> {item.outputPath}
                        </p>
                      </div>
                    </div>

                    {item.status === 'success' && (
                      <div className="flex gap-2 shrink-0 justify-end">
                        <button
                          onClick={() => handleOpenFile(item.outputPath)}
                          title={t('Open File')}
                          className="p-2 rounded-lg border th-border th-hover-surface th-text-2 transition-colors"
                        >
                          <ExternalLink className="w-4 h-4" />
                        </button>
                        <button
                          onClick={() => handleShowInFolder(item.outputPath)}
                          title={t('Show in Folder')}
                          className="p-2 rounded-lg border th-border th-hover-surface th-text-2 transition-colors"
                        >
                          <FolderOpen className="w-4 h-4" />
                        </button>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
