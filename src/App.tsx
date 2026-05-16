import { useState, useEffect } from 'react';
import { Sidebar } from './components/Sidebar';
import { UpdateModal } from './components/UpdateModal';
import { JsonFormatter } from './pages/JsonFormatter';
import { SettingsPage } from './pages/Settings';
import { TextToQr } from './pages/TextToQr';
import { PasswordGenerator } from './pages/PasswordGenerator';
import { SqlInBuilder } from './pages/SqlInBuilder';
import { MarkdownEditor } from './pages/MarkdownEditor';
import { FileSearch } from './pages/FileSearch';
import { FileDiff } from './pages/FileDiff';
import { JarViewer } from './pages/JarViewer';
import { UserPage } from './pages/User';
import { useI18n } from './i18n';
import { useUpdater } from './updater';

export type ToolKey = 'json' | 'qr' | 'pwd' | 'sqlIn' | 'md' | 'fileSearch' | 'fileDiff' | 'jarViewer';
export type ToolsEnabled = Record<ToolKey, boolean>;

const DEFAULT_TOOLS: ToolsEnabled = { json: true, qr: true, pwd: true, sqlIn: true, md: true, fileSearch: true, fileDiff: true, jarViewer: true };

export default function App() {
  const [activePage, setActivePage] = useState(() => {
    const page = localStorage.getItem('mtool_active_page') || 'settings';
    return page === 'user' ? 'settings' : page;
  });
  const { t } = useI18n();
  const updater = useUpdater();
  const { hasUpdate, checkForUpdate } = updater;

  const [showUpdateModal, setShowUpdateModal] = useState(false);

  useEffect(() => {
    const isAuto = localStorage.getItem('mtool_auto_update');
    const shouldAutoUpdate = isAuto !== null ? isAuto === 'true' : true;
    if (shouldAutoUpdate) {
      checkForUpdate();
    }
  }, [checkForUpdate]);

  useEffect(() => {
    if (hasUpdate && updater.updateInfo) {
      const skipped = localStorage.getItem('mtool_skipped_version');
      if (skipped !== updater.updateInfo.version) {
        setShowUpdateModal(true);
      }
    }
  }, [hasUpdate, updater.updateInfo?.version]);

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

  return (
    <div className="flex h-screen overflow-hidden font-sans th-text-2 antialiased th-bg-app">
      <Sidebar 
        activePage={activePage} 
        onNavigate={setActivePage} 
        jsonEnabled={toolsEnabled.json}
        qrEnabled={toolsEnabled.qr}
        pwdEnabled={toolsEnabled.pwd}
        sqlInEnabled={toolsEnabled.sqlIn}
        mdEnabled={toolsEnabled.md}
        fileSearchEnabled={toolsEnabled.fileSearch}
        fileDiffEnabled={toolsEnabled.fileDiff}
        jarViewerEnabled={toolsEnabled.jarViewer}
        hasUpdate={hasUpdate}
      />
      
      <div className="flex-1 flex flex-col min-w-0 th-bg-main">
        
        <main className="flex-1 overflow-y-auto p-6">
          {activePage === 'json' && toolsEnabled.json && <JsonFormatter />}
          {activePage === 'qr' && toolsEnabled.qr && <TextToQr />}
          {activePage === 'pwd' && toolsEnabled.pwd && <PasswordGenerator />}
          {activePage === 'sqlIn' && toolsEnabled.sqlIn && <SqlInBuilder />}
          {activePage === 'md' && toolsEnabled.md && <MarkdownEditor />}
          {activePage === 'fileSearch' && toolsEnabled.fileSearch && <FileSearch />}
          {activePage === 'fileDiff' && toolsEnabled.fileDiff && <FileDiff />}
          {activePage === 'jarViewer' && toolsEnabled.jarViewer && <JarViewer />}
          {activePage === 'user' && <UserPage />}
          {activePage === 'settings' && (
            <SettingsPage 
              toolsEnabled={toolsEnabled}
              toggleTool={toggleTool}
              activePage={activePage}
              setActivePage={setActivePage}
              updater={updater}
              setShowModal={setShowUpdateModal}            />
          )}
          {activePage !== 'settings' && activePage !== 'user' &&
           !(activePage === 'json' && toolsEnabled.json) && 
           !(activePage === 'qr' && toolsEnabled.qr) && 
           !(activePage === 'pwd' && toolsEnabled.pwd) && 
           !(activePage === 'sqlIn' && toolsEnabled.sqlIn) && 
           !(activePage === 'md' && toolsEnabled.md) && 
           !(activePage === 'fileSearch' && toolsEnabled.fileSearch) &&
           !(activePage === 'fileDiff' && toolsEnabled.fileDiff) &&
           !(activePage === 'jarViewer' && toolsEnabled.jarViewer) && (
             <div className="flex items-center justify-center h-full th-text-muted font-medium">{t('Select a tool from the sidebar')}</div>
          )}
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
