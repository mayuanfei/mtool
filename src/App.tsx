import { useState, useEffect, lazy, Suspense } from 'react';
import { Sidebar } from './components/Sidebar';
import { UpdateModal } from './components/UpdateModal';
import { useI18n } from './i18n';
import { useUpdater } from './updater';
import { ErrorBoundary } from './components/ErrorBoundary';

const JsonFormatter = lazy(() => import('./pages/JsonFormatter').then(m => ({ default: m.JsonFormatter })));
const SettingsPage = lazy(() => import('./pages/Settings').then(m => ({ default: m.SettingsPage })));
const TextToQr = lazy(() => import('./pages/TextToQr').then(m => ({ default: m.TextToQr })));
const PasswordGenerator = lazy(() => import('./pages/PasswordGenerator').then(m => ({ default: m.PasswordGenerator })));
const SqlInBuilder = lazy(() => import('./pages/SqlInBuilder').then(m => ({ default: m.SqlInBuilder })));
const MarkdownEditor = lazy(() => import('./pages/MarkdownEditor').then(m => ({ default: m.MarkdownEditor })));
const FileSearch = lazy(() => import('./pages/FileSearch').then(m => ({ default: m.FileSearch })));
const FileDiff = lazy(() => import('./pages/FileDiff').then(m => ({ default: m.FileDiff })));
const JarViewer = lazy(() => import('./pages/JarViewer').then(m => ({ default: m.JarViewer })));
const EncoderDecoder = lazy(() => import('./pages/EncoderDecoder').then(m => ({ default: m.EncoderDecoder })));
const CryptoTool = lazy(() => import('./pages/CryptoTool').then(m => ({ default: m.CryptoTool })));
const FileTransfer = lazy(() => import('./pages/FileTransfer').then(m => ({ default: m.FileTransfer })));
const DocConverter = lazy(() => import('./pages/DocConverter').then(m => ({ default: m.DocConverter })));

export type ToolKey = 'json' | 'qr' | 'pwd' | 'sqlIn' | 'md' | 'fileSearch' | 'fileDiff' | 'jarViewer' | 'encoder' | 'crypto' | 'fileTransfer' | 'docConvert';
export type ToolsEnabled = Record<ToolKey, boolean>;

const DEFAULT_TOOLS: ToolsEnabled = { json: true, qr: true, pwd: true, sqlIn: true, md: true, fileSearch: true, fileDiff: true, jarViewer: true, encoder: true, crypto: true, fileTransfer: true, docConvert: true };

export default function App() {
  const [activePage, setActivePage] = useState(() => {
    const page = localStorage.getItem('mtool_active_page') || 'settings';
    return page === 'user' ? 'settings' : page;
  });
  const { t } = useI18n();
  const updater = useUpdater();
  const { hasUpdate, checkForUpdate, updateInfo } = updater;

  const [showUpdateModal, setShowUpdateModal] = useState(false);

  useEffect(() => {
    const isAuto = localStorage.getItem('mtool_auto_update');
    const shouldAutoUpdate = isAuto !== null ? isAuto === 'true' : true;
    if (shouldAutoUpdate) {
      checkForUpdate();
    }
  }, [checkForUpdate]);

  useEffect(() => {
    if (hasUpdate && updateInfo) {
      const skipped = localStorage.getItem('mtool_skipped_version');
      if (skipped !== updateInfo.version) {
        setShowUpdateModal(true);
      }
    }
  }, [hasUpdate, updateInfo]);

  const handleSkip = () => {
    if (updater.updateInfo) {
      localStorage.setItem('mtool_skipped_version', updater.updateInfo.version);
    }
    setShowUpdateModal(false);
    updater.dismissUpdate();
  };

  const [toolsEnabled, setToolsEnabled] = useState<ToolsEnabled>(() => {
    try {
      const saved = localStorage.getItem('mtool_tools_enabled');
      if (saved) return { ...DEFAULT_TOOLS, ...JSON.parse(saved) };
    } catch (e) {}
    const getOld = (key: string): boolean => {
      const v = localStorage.getItem(key);
      localStorage.removeItem(key);
      return v !== null ? v === 'true' : true;
    };
    return {
      json: getOld('mtool_json_enabled'),
      qr: getOld('mtool_qr_enabled'),
      pwd: getOld('mtool_pwd_enabled'),
      sqlIn: getOld('mtool_sqlin_enabled'),
      md: getOld('mtool_md_enabled'),
      fileSearch: getOld('mtool_filesearch_enabled'),
      fileDiff: getOld('mtool_filediff_enabled'),
      jarViewer: getOld('mtool_jarviewer_enabled'),
      encoder: getOld('mtool_encoder_enabled'),
      crypto: getOld('mtool_crypto_enabled'),
      fileTransfer: getOld('mtool_filetransfer_enabled'),
      docConvert: getOld('mtool_docconvert_enabled'),
    };
  });

  useEffect(() => {
    localStorage.setItem('mtool_tools_enabled', JSON.stringify(toolsEnabled));
  }, [toolsEnabled]);

  const toggleTool = (tool: ToolKey, enabled: boolean) => {
    setToolsEnabled((prev) => ({ ...prev, [tool]: enabled }));
  };

  useEffect(() => {
    localStorage.setItem('mtool_active_page', activePage);
  }, [activePage]);

  const [mdDirty, setMdDirty] = useState(false);

  const handleNavigate = (page: string) => {
    if (activePage === 'md' && mdDirty) {
      if (!window.confirm(t('You have unsaved changes in Markdown Editor. Are you sure you want to leave?'))) {
        return;
      }
    }
    setActivePage(page);
  };

  const isToolActive = activePage !== 'settings' && activePage in toolsEnabled && toolsEnabled[activePage as ToolKey];

  return (
    <div className="flex h-screen overflow-hidden font-sans th-text-2 antialiased th-bg-app">
      <Sidebar 
        activePage={activePage} 
        onNavigate={handleNavigate} 
        jsonEnabled={toolsEnabled.json}
        qrEnabled={toolsEnabled.qr}
        pwdEnabled={toolsEnabled.pwd}
        sqlInEnabled={toolsEnabled.sqlIn}
        mdEnabled={toolsEnabled.md}
        fileSearchEnabled={toolsEnabled.fileSearch}
        fileDiffEnabled={toolsEnabled.fileDiff}
        jarViewerEnabled={toolsEnabled.jarViewer}
        encoderEnabled={toolsEnabled.encoder}
        cryptoEnabled={toolsEnabled.crypto}
        fileTransferEnabled={toolsEnabled.fileTransfer}
        docConvertEnabled={toolsEnabled.docConvert}
        hasUpdate={hasUpdate}
      />
      
      <div className="flex-1 flex flex-col min-w-0 th-bg-main">
        
        <main className={`flex-1 overflow-y-auto ${activePage === 'jarViewer' || activePage === 'fileDiff' ? 'p-0' : 'p-6'}`}>
          <ErrorBoundary>
            <Suspense fallback={
              <div className="flex items-center justify-center h-full th-text-muted font-medium">
                {t('Loading...')}
              </div>
            }>
              {activePage === 'json' && toolsEnabled.json && <JsonFormatter />}
              {activePage === 'qr' && toolsEnabled.qr && <TextToQr />}
              {activePage === 'pwd' && toolsEnabled.pwd && <PasswordGenerator />}
              {activePage === 'sqlIn' && toolsEnabled.sqlIn && <SqlInBuilder />}
              {activePage === 'md' && toolsEnabled.md && <MarkdownEditor setMdDirty={setMdDirty} />}
              {activePage === 'fileSearch' && toolsEnabled.fileSearch && <FileSearch />}
              {activePage === 'fileDiff' && toolsEnabled.fileDiff && <FileDiff />}
              {activePage === 'jarViewer' && toolsEnabled.jarViewer && <JarViewer />}
              {activePage === 'encoder' && toolsEnabled.encoder && <EncoderDecoder />}
              {activePage === 'crypto' && toolsEnabled.crypto && <CryptoTool />}
              {activePage === 'fileTransfer' && toolsEnabled.fileTransfer && <FileTransfer />}
              {activePage === 'docConvert' && toolsEnabled.docConvert && <DocConverter />}
              {activePage === 'settings' && (
                <SettingsPage 
                  toolsEnabled={toolsEnabled}
                  toggleTool={toggleTool}
                  activePage={activePage}
                  setActivePage={handleNavigate}
                  updater={updater}
                  setShowModal={setShowUpdateModal}            />
              )}
              {activePage !== 'settings' && !isToolActive && (
                 <div className="flex items-center justify-center h-full th-text-muted font-medium">{t('Select a tool from the sidebar')}</div>
              )}
            </Suspense>
          </ErrorBoundary>
        </main>
      </div>

      {showUpdateModal && updater.updateInfo && (
        <UpdateModal
          open={showUpdateModal}
          updateInfo={updater.updateInfo}
          downloading={updater.downloading}
          progress={updater.progress}
          error={updater.error}
          installed={updater.installed}
          onClose={() => {
            if (!updater.downloading && !updater.installed) {
              setShowUpdateModal(false);
              updater.dismissUpdate();
            }
          }}
          onSkip={handleSkip}
          onInstall={updater.startInstall}
          onRelaunch={updater.doRelaunch}
        />
      )}
    </div>
  );
}
