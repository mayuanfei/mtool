import { Globe, Wrench } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../i18n';

const Toggle = ({ checked, onChange }: { checked: boolean; onChange: () => void }) => {
  return (
    <button
      onClick={onChange}
      className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 focus:ring-offset-slate-900 ${
        checked ? 'bg-indigo-500' : 'bg-slate-700'
      }`}
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
}

export function SettingsPage({ jsonEnabled, setJsonEnabled, qrEnabled, setQrEnabled, pwdEnabled, setPwdEnabled, sqlInEnabled, setSqlInEnabled, mdEnabled, setMdEnabled, fileSearchEnabled, setFileSearchEnabled }: SettingsPageProps) {
  const { t, language, setLanguage } = useI18n();

  return (
    <div className="max-w-4xl max-w-5xl mx-auto w-full">
      <div className="mb-10">
        <h1 className="text-4xl font-bold tracking-tight text-white mb-2">{t('Settings')}</h1>
        <p className="text-slate-400">{t('Configure MTOOL behaviors and active utilities.')}</p>
      </div>

      <div className="space-y-6">
        {/* Utility Configuration Card */}
        <section className="bg-slate-900 border border-slate-800 rounded-xl overflow-hidden shadow-2xl">
          <div className="px-6 py-4 border-b border-slate-800 flex items-center gap-3 bg-slate-800/50">
            <Wrench className="w-5 h-5 text-indigo-400" />
            <h2 className="text-sm font-bold tracking-tighter text-slate-300 uppercase">{t('Utility Configuration')}</h2>
          </div>
          
          <div className="divide-y divide-slate-800">
            <div className="px-6 py-5 flex items-center justify-between hover:bg-slate-800/50 transition-colors">
              <div>
                <p className="text-base font-medium text-slate-200 mb-1">{t('File Search')}</p>
                <p className="text-sm text-slate-500">{t('Search and find files by name, size, or content.')}</p>
              </div>
              <Toggle
                checked={fileSearchEnabled}
                onChange={async () => {
                  const next = !fileSearchEnabled;
                  setFileSearchEnabled(next);
                  if (!next) {
                    await invoke('disable_file_search').catch(console.error);
                  } else {
                    await invoke('build_index').catch(console.error);
                  }
                }}
              />
            </div>

            <div className="px-6 py-5 flex items-center justify-between hover:bg-slate-800/50 transition-colors">
              <div>
                <p className="text-base font-medium text-slate-200 mb-1">{t('Markdown Editor')}</p>
                <p className="text-sm text-slate-500">{t('View and edit Markdown files with live preview.')}</p>
              </div>
              <Toggle checked={mdEnabled} onChange={() => setMdEnabled(!mdEnabled)} />
            </div>

            <div className="px-6 py-5 flex items-center justify-between hover:bg-slate-800/50 transition-colors">
              <div>
                <p className="text-base font-medium text-slate-200 mb-1">{t('JSON Formatter')}</p>
                <p className="text-sm text-slate-500">{t('Parse, validate, and beautify raw JSON payloads.')}</p>
              </div>
              <Toggle checked={jsonEnabled} onChange={() => setJsonEnabled(!jsonEnabled)} />
            </div>

            <div className="px-6 py-5 flex items-center justify-between hover:bg-slate-800/50 transition-colors">
              <div>
                <p className="text-base font-medium text-slate-200 mb-1">{t('Text to QR')}</p>
                <p className="text-sm text-slate-500">{t('Generate scannable QR codes from string inputs.')}</p>
              </div>
              <Toggle checked={qrEnabled} onChange={() => setQrEnabled(!qrEnabled)} />
            </div>

            <div className="px-6 py-5 flex items-center justify-between hover:bg-slate-800/50 transition-colors">
              <div>
                <p className="text-base font-medium text-slate-200 mb-1">{t('Password Generator')}</p>
                <p className="text-sm text-slate-500">{t('Create secure passwords.')}</p>
              </div>
              <Toggle checked={pwdEnabled} onChange={() => setPwdEnabled(!pwdEnabled)} />
            </div>

            <div className="px-6 py-5 flex items-center justify-between hover:bg-slate-800/50 transition-colors">
              <div>
                <p className="text-base font-medium text-slate-200 mb-1">{t('SQL IN Builder')}</p>
                <p className="text-sm text-slate-500">{t('Build SQL IN clause from column values.')}</p>
              </div>
              <Toggle checked={sqlInEnabled} onChange={() => setSqlInEnabled(!sqlInEnabled)} />
            </div>

          </div>
        </section>

        {/* General Settings Card */}
        <section className="bg-slate-900 border border-slate-800 rounded-xl overflow-hidden shadow-2xl">
          <div className="px-6 py-4 border-b border-slate-800 flex items-center gap-3 bg-slate-800/50">
            <Globe className="w-5 h-5 text-indigo-400" />
            <h2 className="text-sm font-bold tracking-tighter text-slate-300 uppercase">{t('General Settings')}</h2>
          </div>
          
          <div className="px-6 py-5 flex items-center justify-between hover:bg-slate-800/50 transition-colors">
            <div>
              <p className="text-base font-medium text-slate-200 mb-1">{t('Language')}</p>
              <p className="text-sm text-slate-500">{t('Select application interface language.')}</p>
            </div>
            
            <div className="relative">
              <select 
                value={language}
                onChange={(e) => setLanguage(e.target.value as any)}
                className="appearance-none bg-slate-950 border border-slate-700 text-slate-300 text-sm rounded-lg focus:ring-indigo-500 focus:border-indigo-500 focus:outline-none block w-40 p-2.5 pr-8 hover:border-slate-500 transition-colors cursor-pointer shadow-inner"
              >
                <option value="en">English</option>
                <option value="zh">简体中文</option>
              </select>
              <div className="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 text-gray-400">
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M19 9l-7 7-7-7"></path></svg>
              </div>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}
