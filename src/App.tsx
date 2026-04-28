import { useState } from 'react';
import { Sidebar } from './components/Sidebar';
import { JsonFormatter } from './pages/JsonFormatter';
import { SettingsPage } from './pages/Settings';
import { TextToQr } from './pages/TextToQr';

export default function App() {
  const [activePage, setActivePage] = useState('settings');

  return (
    <div className="flex h-screen overflow-hidden font-sans text-slate-300 antialiased" style={{ backgroundColor: '#0f1115' }}>
      <Sidebar activePage={activePage} onNavigate={setActivePage} />
      
      <div className="flex-1 flex flex-col min-w-0 bg-slate-950">
        
        <main className="flex-1 overflow-y-auto p-6">
          {activePage === 'json' && <JsonFormatter />}
          {activePage === 'qr' && <TextToQr />}
          {activePage === 'settings' && <SettingsPage />}
          {/* Default fallback just in case */}
          {activePage !== 'json' && activePage !== 'qr' && activePage !== 'settings' && (
             <div className="text-white">Select a tool from the sidebar</div>
          )}
        </main>
      </div>
    </div>
  );
}
