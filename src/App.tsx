import { useState, useEffect } from 'react';
import { Sidebar } from './components/Sidebar';
import { JsonFormatter } from './pages/JsonFormatter';
import { SettingsPage } from './pages/Settings';
import { TextToQr } from './pages/TextToQr';
import { PasswordGenerator } from './pages/PasswordGenerator';
import { SqlInBuilder } from './pages/SqlInBuilder';
import { MarkdownEditor } from './pages/MarkdownEditor';
import { FileSearch } from './pages/FileSearch';
import { UserPage } from './pages/User';
import { useI18n } from './i18n';

export default function App() {
  const [activePage, setActivePage] = useState(() => {
    return localStorage.getItem('mtool_active_page') || 'settings';
  });
  const { t } = useI18n();

  const [jsonEnabled, setJsonEnabled] = useState(() => {
    const saved = localStorage.getItem('mtool_json_enabled');
    return saved !== null ? saved === 'true' : true;
  });
  
  const [qrEnabled, setQrEnabled] = useState(() => {
    const saved = localStorage.getItem('mtool_qr_enabled');
    return saved !== null ? saved === 'true' : true;
  });

  const [pwdEnabled, setPwdEnabled] = useState(() => {
    const saved = localStorage.getItem('mtool_pwd_enabled');
    return saved !== null ? saved === 'true' : true;
  });

  const [sqlInEnabled, setSqlInEnabled] = useState(() => {
    const saved = localStorage.getItem('mtool_sqlin_enabled');
    return saved !== null ? saved === 'true' : true;
  });

  const [mdEnabled, setMdEnabled] = useState(() => {
    const saved = localStorage.getItem('mtool_md_enabled');
    return saved !== null ? saved === 'true' : true;
  });

  const [fileSearchEnabled, setFileSearchEnabled] = useState(() => {
    const saved = localStorage.getItem('mtool_filesearch_enabled');
    return saved !== null ? saved === 'true' : true;
  });

  useEffect(() => {
    localStorage.setItem('mtool_json_enabled', jsonEnabled.toString());
  }, [jsonEnabled]);

  useEffect(() => {
    localStorage.setItem('mtool_qr_enabled', qrEnabled.toString());
  }, [qrEnabled]);

  useEffect(() => {
    localStorage.setItem('mtool_pwd_enabled', pwdEnabled.toString());
  }, [pwdEnabled]);

  useEffect(() => {
    localStorage.setItem('mtool_sqlin_enabled', sqlInEnabled.toString());
  }, [sqlInEnabled]);

  useEffect(() => {
    localStorage.setItem('mtool_md_enabled', mdEnabled.toString());
  }, [mdEnabled]);

  useEffect(() => {
    localStorage.setItem('mtool_filesearch_enabled', fileSearchEnabled.toString());
  }, [fileSearchEnabled]);

  useEffect(() => {
    localStorage.setItem('mtool_active_page', activePage);
  }, [activePage]);

  return (
    <div className="flex h-screen overflow-hidden font-sans th-text-2 antialiased th-bg-app">
      <Sidebar 
        activePage={activePage} 
        onNavigate={setActivePage} 
        jsonEnabled={jsonEnabled}
        qrEnabled={qrEnabled}
        pwdEnabled={pwdEnabled}
        sqlInEnabled={sqlInEnabled}
        mdEnabled={mdEnabled}
        fileSearchEnabled={fileSearchEnabled}
      />
      
      <div className="flex-1 flex flex-col min-w-0 th-bg-main">
        
        <main className="flex-1 overflow-y-auto p-6">
          {activePage === 'json' && jsonEnabled && <JsonFormatter />}
          {activePage === 'qr' && qrEnabled && <TextToQr />}
          {activePage === 'pwd' && pwdEnabled && <PasswordGenerator />}
          {activePage === 'sqlIn' && sqlInEnabled && <SqlInBuilder />}
          {activePage === 'md' && mdEnabled && <MarkdownEditor />}
          {activePage === 'fileSearch' && fileSearchEnabled && <FileSearch />}
          {activePage === 'user' && <UserPage />}
          {activePage === 'settings' && (
            <SettingsPage 
              jsonEnabled={jsonEnabled} 
              setJsonEnabled={setJsonEnabled}
              qrEnabled={qrEnabled}
              setQrEnabled={setQrEnabled}
              pwdEnabled={pwdEnabled}
              setPwdEnabled={setPwdEnabled}
              sqlInEnabled={sqlInEnabled}
              setSqlInEnabled={setSqlInEnabled}
              mdEnabled={mdEnabled}
              setMdEnabled={setMdEnabled}
              fileSearchEnabled={fileSearchEnabled}
              setFileSearchEnabled={setFileSearchEnabled}
            />
          )}
          {activePage !== 'settings' && activePage !== 'user' &&
           !(activePage === 'json' && jsonEnabled) && 
           !(activePage === 'qr' && qrEnabled) && 
           !(activePage === 'pwd' && pwdEnabled) && 
           !(activePage === 'sqlIn' && sqlInEnabled) && 
           !(activePage === 'md' && mdEnabled) && 
           !(activePage === 'fileSearch' && fileSearchEnabled) && (
             <div className="flex items-center justify-center h-full th-text-muted font-medium">{t('Select a tool from the sidebar')}</div>
          )}
        </main>
      </div>
    </div>
  );
}
