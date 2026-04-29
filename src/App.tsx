import { useState, useEffect } from 'react';
import { Sidebar } from './components/Sidebar';
import { JsonFormatter } from './pages/JsonFormatter';
import { SettingsPage } from './pages/Settings';
import { TextToQr } from './pages/TextToQr';

export default function App() {
  const [activePage, setActivePage] = useState('settings');

  const [jsonEnabled, setJsonEnabled] = useState(() => {
    const saved = localStorage.getItem('mtool_json_enabled');
    return saved !== null ? saved === 'true' : true;
  });
  
  const [qrEnabled, setQrEnabled] = useState(() => {
    const saved = localStorage.getItem('mtool_qr_enabled');
    return saved !== null ? saved === 'true' : true;
  });

  useEffect(() => {
    localStorage.setItem('mtool_json_enabled', jsonEnabled.toString());
  }, [jsonEnabled]);

  useEffect(() => {
    localStorage.setItem('mtool_qr_enabled', qrEnabled.toString());
  }, [qrEnabled]);

  return (
    <div className="flex h-screen overflow-hidden font-sans text-slate-300 antialiased" style={{ backgroundColor: '#0f1115' }}>
      <Sidebar 
        activePage={activePage} 
        onNavigate={setActivePage} 
        jsonEnabled={jsonEnabled}
        qrEnabled={qrEnabled}
      />
      
      <div className="flex-1 flex flex-col min-w-0 bg-slate-950">
        
        <main className="flex-1 overflow-y-auto p-6">
          {activePage === 'json' && jsonEnabled && <JsonFormatter />}
          {activePage === 'qr' && qrEnabled && <TextToQr />}
          {activePage === 'settings' && (
            <SettingsPage 
              jsonEnabled={jsonEnabled} 
              setJsonEnabled={setJsonEnabled}
              qrEnabled={qrEnabled}
              setQrEnabled={setQrEnabled}
            />
          )}
          {activePage !== 'settings' && 
           !(activePage === 'json' && jsonEnabled) && 
           !(activePage === 'qr' && qrEnabled) && (
             <div className="text-white flex items-center justify-center h-full text-slate-500 font-medium">Select a tool from the sidebar</div>
          )}
        </main>
      </div>
    </div>
  );
}
