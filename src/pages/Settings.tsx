import { useState } from 'react';
import { Globe, Wrench, Palette, RefreshCw } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../i18n';
import { useTheme } from '../theme';
import { UpdateModal } from '../components/UpdateModal';
import type { Theme } from '../theme';
import type { UseUpdaterReturn } from '../updater';

const Toggle = ({ checked, onChange, disabled }: { checked: boolean; onChange: () => void; disabled?: boolean }) => {
  return (
    <button
      onClick={onChange}
      disabled={disabled}
      className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 th-ring-offset ${
        checked ? 'bg-indigo-500' : 'th-bg-surface'
      } ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
    >
      <span
        className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
          checked ? 'translate-x-6' : 'translate-x-1'
        }`}
      />
    </button>
  );
};

interface SettingsPageProps {
  jsonEnabled: boolean;
  setJsonEnabled: (v: boolean) => void;
  qrEnabled: boolean;
  setQrEnabled: (v: boolean) => void;
  pwdEnabled: boolean;
  setPwdEnabled: (v: boolean) => void;
  sqlInEnabled: boolean;
  setSqlInEnabled: (v: boolean) => void;
  mdEnabled: boolean;
  setMdEnabled: (v: boolean) => void;
  fileSearchEnabled: boolean;
  setFileSearchEnabled: (v: boolean) => void;
  activePage: string;
  setActivePage: (page: string) => void;
  updater: UseUpdaterReturn;
}

export function SettingsPage({ jsonEnabled, setJsonEnabled, qrEnabled, setQrEnabled, pwdEnabled, setPwdEnabled, sqlInEnabled, setSqlInEnabled, mdEnabled, setMdEnabled, fileSearchEnabled, setFileSearchEnabled, activePage, setActivePage, updater }: SettingsPageProps) {
  const { t, language, setLanguage } = useI18n();
  const { theme, setTheme } = useTheme();
  const [isProcessing, setIsProcessing] = useState(false);
  const { hasUpdate, updateInfo, checking, downloading, progress, error, installed,
          autoUpdate, setAutoUpdate, checkForUpdate, startInstall, doRelaunch, dismissUpdate } = updater;
  const [showModal, setShowModal] = useState(false);
  const [lastCheckDone, setLastCheckDone] = useState(false);

  const handleCheck = async () => {
    await checkForUpdate();
    setLastCheckDone(true);
  };

  return (
    <div className="max-w-4xl max-w-5xl mx-auto w-full">
      <div className="mb-10">
        <h1 className="text-4xl font-bold tracking-tight th-text mb-2">{t('Settings')}</h1>
        <p className="th-text-3">{t('Configure MTOOL behaviors and active utilities.')}</p>
      </div>

      <div className="space-y-6">
        {/* Utility Configuration Card */}
        <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl">
          <div className="px-6 py-4 border-b th-border flex items-center gap-3 th-bg-surface-h">
            <Wrench className="w-5 h-5 text-indigo-400" />
            <h2 className="text-sm font-bold tracking-tighter th-text-2 uppercase">{t('Utility Configuration')}</h2>
          </div>
          
          <div className="divide-y th-divide">
            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <div>
                <p className="text-base font-medium th-text-2 mb-1">{t('File Search')}</p>
                <p className="text-sm th-text-muted">{t('Search and find files by name, size, or content.')}</p>
              </div>
              <Toggle
                checked={fileSearchEnabled}
                disabled={isProcessing}
                onChange={async () => {
                  if (isProcessing) return;
                  setIsProcessing(true);
                  const next = !fileSearchEnabled;
                  setFileSearchEnabled(next);
                  if (!next && activePage === 'fileSearch') setActivePage('settings');
                  try {
                    if (!next) await invoke('disable_file_search');
                    else await invoke('build_index');
                  } catch (e) {
                    console.error(e);
                    setFileSearchEnabled(!next);
                  } finally {
                    setIsProcessing(false);
                  }
                }}
              />
            </div>

            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <div>
                <p className="text-base font-medium th-text-2 mb-1">{t('Markdown Editor')}</p>
                <p className="text-sm th-text-muted">{t('View and edit Markdown files with live preview.')}</p>
              </div>
              <Toggle checked={mdEnabled} onChange={() => {
                const next = !mdEnabled;
                setMdEnabled(next);
                if (!next && activePage === 'md') setActivePage('settings');
              }} />
            </div>

            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <div>
                <p className="text-base font-medium th-text-2 mb-1">{t('JSON Formatter')}</p>
                <p className="text-sm th-text-muted">{t('Parse, validate, and beautify raw JSON payloads.')}</p>
              </div>
              <Toggle checked={jsonEnabled} onChange={() => {
                const next = !jsonEnabled;
                setJsonEnabled(next);
                if (!next && activePage === 'json') setActivePage('settings');
              }} />
            </div>

            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <div>
                <p className="text-base font-medium th-text-2 mb-1">{t('Text to QR')}</p>
                <p className="text-sm th-text-muted">{t('Generate scannable QR codes from string inputs.')}</p>
              </div>
              <Toggle checked={qrEnabled} onChange={() => {
                const next = !qrEnabled;
                setQrEnabled(next);
                if (!next && activePage === 'qr') setActivePage('settings');
              }} />
            </div>

            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <div>
                <p className="text-base font-medium th-text-2 mb-1">{t('Password Generator')}</p>
                <p className="text-sm th-text-muted">{t('Create secure passwords.')}</p>
              </div>
              <Toggle checked={pwdEnabled} onChange={() => {
                const next = !pwdEnabled;
                setPwdEnabled(next);
                if (!next && activePage === 'pwd') setActivePage('settings');
              }} />
            </div>

            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <div>
                <p className="text-base font-medium th-text-2 mb-1">{t('SQL IN Builder')}</p>
                <p className="text-sm th-text-muted">{t('Build SQL IN clause from column values.')}</p>
              </div>
              <Toggle checked={sqlInEnabled} onChange={() => {
                const next = !sqlInEnabled;
                setSqlInEnabled(next);
                if (!next && activePage === 'sqlIn') setActivePage('settings');
              }} />
            </div>

          </div>
        </section>

        {/* General Settings Card */}
        <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl">
          <div className="px-6 py-4 border-b th-border flex items-center gap-3 th-bg-surface-h">
            <Globe className="w-5 h-5 text-indigo-400" />
            <h2 className="text-sm font-bold tracking-tighter th-text-2 uppercase">{t('General Settings')}</h2>
          </div>
          
          <div className="divide-y th-divide">
            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <div>
                <p className="text-base font-medium th-text-2 mb-1">{t('Language')}</p>
                <p className="text-sm th-text-muted">{t('Select application interface language.')}</p>
              </div>
              
              <div className="relative">
                <select 
                  value={language}
                  onChange={(e) => setLanguage(e.target.value as any)}
                  className="appearance-none th-bg-input border th-border-subtle th-text-2 text-sm rounded-lg focus:ring-indigo-500 focus:border-indigo-500 focus:outline-none block w-40 p-2.5 pr-8 transition-colors cursor-pointer shadow-inner"
                >
                  <option value="en">English</option>
                  <option value="zh">简体中文</option>
                </select>
                <div className="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 th-text-3">
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M19 9l-7 7-7-7"></path></svg>
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* Appearance Card */}
        <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl">
          <div className="px-6 py-4 border-b th-border flex items-center gap-3 th-bg-surface-h">
            <Palette className="w-5 h-5 text-indigo-400" />
            <h2 className="text-sm font-bold tracking-tighter th-text-2 uppercase">{t('Appearance')}</h2>
          </div>
          
          <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
            <div>
              <p className="text-base font-medium th-text-2 mb-1">{t('Theme')}</p>
              <p className="text-sm th-text-muted">{t('Choose light or dark color scheme.')}</p>
            </div>
            
            <div className="flex gap-3">
              {([
                { value: 'dark' as Theme, label: t('Dark'), icon: '🌙' },
                { value: 'light' as Theme, label: t('Light'), icon: '☀️' },
              ]).map(opt => (
                <button
                  key={opt.value}
                  onClick={() => setTheme(opt.value)}
                  className={`px-4 py-2 rounded-lg text-sm font-medium transition-all border focus:outline-none flex items-center gap-2 ${
                    theme === opt.value
                      ? 'bg-indigo-600/15 text-indigo-400 border-indigo-500/30 shadow-sm'
                      : 'th-bg-input-alt th-text-3 th-border-subtle th-hover-surface'
                  }`}
                >
                  <span>{opt.icon}</span>
                  {opt.label}
                </button>
              ))}
            </div>
          </div>
        </section>

        {/* Updates Card */}
        <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl">
          <div className="px-6 py-4 border-b th-border flex items-center gap-3 th-bg-surface-h">
            <RefreshCw className="w-5 h-5 text-indigo-400" />
            <h2 className="text-sm font-bold tracking-tighter th-text-2 uppercase">{t('Updates')}</h2>
          </div>

          <div className="divide-y th-divide">
            {/* Auto-update toggle */}
            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <div>
                <p className="text-base font-medium th-text-2 mb-1">{t('Auto-update')}</p>
                <p className="text-sm th-text-muted">{t('Automatically check for updates on startup.')}</p>
              </div>
              <Toggle checked={autoUpdate} onChange={() => setAutoUpdate(!autoUpdate)} />
            </div>

            {/* Current version */}
            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <p className="text-base font-medium th-text-2">{t('Current version')}</p>
              <span className="text-sm font-mono th-text-muted">v1.0.0</span>
            </div>

            {/* Check for updates */}
            <div className="px-6 py-5 flex items-center justify-between th-hover-surface transition-colors">
              <div>
                <p className="text-base font-medium th-text-2 mb-1">{t('Check for Updates')}</p>
                <p className="text-sm">
                  {checking
                    ? <span className="th-text-muted">{t('Checking...')}</span>
                    : hasUpdate && updateInfo
                    ? <span className="text-amber-400 font-medium">v{updateInfo.version} {t('available')}</span>
                    : lastCheckDone
                    ? <span className="text-emerald-400">{t('Up to date')}</span>
                    : null}
                </p>
              </div>
              <div className="flex items-center gap-3">
                {hasUpdate && updateInfo && (
                  <button
                    onClick={() => setShowModal(true)}
                    className="px-3 py-1.5 text-sm font-medium bg-indigo-600 hover:bg-indigo-700 text-white rounded-lg transition-colors"
                  >
                    {t('Install & Restart')}
                  </button>
                )}
                <button
                  onClick={handleCheck}
                  disabled={checking}
                  className="px-3 py-1.5 text-sm th-text-3 th-bg-input-alt border th-border-subtle rounded-lg th-hover-surface transition-colors disabled:opacity-50 flex items-center gap-2"
                >
                  <RefreshCw className={`w-3.5 h-3.5 ${checking ? 'animate-spin' : ''}`} />
                  {checking ? t('Checking...') : t('Check for Updates')}
                </button>
              </div>
            </div>
          </div>
        </section>
      </div>

      {/* Update install modal */}
      {showModal && updateInfo && (
        <UpdateModal
          open={showModal}
          updateInfo={updateInfo}
          downloading={downloading}
          progress={progress}
          error={error}
          installed={installed}
          onClose={() => { if (!downloading && !installed) { setShowModal(false); dismissUpdate(); } }}
          onInstall={startInstall}
          onRelaunch={doRelaunch}
        />
      )}
    </div>
  );
}
