import { Code2, QrCode, Settings, User, Key, Database } from 'lucide-react';
import { useI18n } from '../i18n';

interface SidebarProps {
  activePage: string;
  onNavigate: (page: string) => void;
  jsonEnabled: boolean;
  qrEnabled: boolean;
  pwdEnabled: boolean;
  sqlInEnabled: boolean;
}

export function Sidebar({ activePage, onNavigate, jsonEnabled, qrEnabled, pwdEnabled, sqlInEnabled }: SidebarProps) {
  const { t } = useI18n();

  const navItems = [
    ...(jsonEnabled ? [{ id: 'json', label: t('JSON Formatter'), icon: Code2 }] : []),
    ...(qrEnabled ? [{ id: 'qr', label: t('Text to QR'), icon: QrCode }] : []),
    ...(pwdEnabled ? [{ id: 'pwd', label: t('Password Generator'), icon: Key }] : []),
    ...(sqlInEnabled ? [{ id: 'sqlIn', label: t('SQL IN Builder'), icon: Database }] : []),
  ];

  const bottomItems = [
    { id: 'settings', label: t('Settings'), icon: Settings },
    { id: 'user', label: t('User'), icon: User },
  ];

  const NavItem = ({ item }: { item: any }) => {
    const isActive = activePage === item.id;
    return (
      <button
        onClick={() => onNavigate(item.id)}
        className={`w-full flex items-center gap-3 px-3 py-2 rounded-md font-medium transition-colors ${
          isActive
            ? 'bg-indigo-600/10 text-indigo-400 border border-indigo-500/20 shadow-sm'
            : 'text-slate-400 hover:bg-slate-800 border border-transparent'
        }`}
      >
        <item.icon className="w-[18px] h-[18px]" strokeWidth={isActive ? 2 : 2} />
        <span className="text-[13px]">{item.label}</span>
      </button>
    );
  };

  return (
    <aside className="w-64 flex-shrink-0 flex flex-col border-r border-slate-800 bg-slate-900/50">
      <div className="p-6 flex items-center gap-3">
        <div className="w-8 h-8 bg-indigo-500 rounded-lg flex items-center justify-center shadow-lg shadow-indigo-500/20">
           <svg viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg" className="w-5 h-5 text-white">
              <path d="M4 20V5.5L10 12L16 5.5V19" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"/>
              <circle cx="10" cy="12" r="2" stroke="currentColor" strokeWidth="2.5"/>
              <path d="M4 16C4 16 2.5 17.5 2.5 19C2.5 20.5 4 21 4 21C4 21 5.5 20.5 5.5 19C5.5 17.5 4 16 4 16Z" stroke="currentColor" strokeWidth="2.5"/>
              <path d="M16 16C16 16 14.5 17.5 14.5 19C14.5 20.5 16 21 16 21C16 21 17.5 20.5 17.5 19C17.5 17.5 16 16 16 16Z" stroke="currentColor" strokeWidth="2.5"/>
           </svg>
        </div>
        <span className="font-bold text-lg text-white tracking-tight">MTOOL</span>
      </div>

      <div className="flex-1 px-3 space-y-1">
        <div className="text-[10px] font-bold text-slate-500 uppercase tracking-widest px-3 mb-2 mt-2">{t('Tools')}</div>
        {navItems.map((item) => (
          <NavItem key={item.id} item={item} />
        ))}
      </div>

      <div className="mt-auto p-3 space-y-1">
        <div className="text-[10px] font-bold text-slate-500 uppercase tracking-widest px-3 mb-2">{t('System')}</div>
        {bottomItems.map((item) => (
          <NavItem key={item.id} item={item} />
        ))}
      </div>
    </aside>
  );
}
