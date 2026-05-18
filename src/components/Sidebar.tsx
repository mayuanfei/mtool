import { useState, useEffect } from 'react';
import { Code2, QrCode, Settings, Key, Database, FileText, PanelLeftClose, PanelLeftOpen, SearchCode, FileDiff, Package, ArrowRightLeft, Shield } from 'lucide-react';
import { useI18n } from '../i18n';

interface SidebarProps {
  activePage: string;
  onNavigate: (page: string) => void;
  jsonEnabled: boolean;
  qrEnabled: boolean;
  pwdEnabled: boolean;
  sqlInEnabled: boolean;
  mdEnabled: boolean;
  fileSearchEnabled: boolean;
  fileDiffEnabled: boolean;
  jarViewerEnabled: boolean;
  encoderEnabled: boolean;
  cryptoEnabled: boolean;
  hasUpdate: boolean;
}

interface NavItemProps {
  item: {
    id: string;
    label: string;
    icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  };
  activePage: string;
  collapsed: boolean;
  onNavigate: (page: string) => void;
}

function NavItem({ item, activePage, collapsed, onNavigate }: NavItemProps) {
  const isActive = activePage === item.id;
  return (
    <button
      onClick={() => onNavigate(item.id)}
      title={collapsed ? item.label : undefined}
      className={`w-full flex items-center ${collapsed ? 'justify-center' : ''} gap-3 px-3 py-2 rounded-md font-medium transition-colors ${
        isActive
          ? 'bg-indigo-600/10 text-indigo-400 border border-indigo-500/20 shadow-sm'
          : 'th-text-3 th-hover-surface border border-transparent'
      }`}
    >
      <item.icon className="w-[18px] h-[18px] flex-shrink-0" strokeWidth={2} />
      {!collapsed && <span className="text-[13px] truncate">{item.label}</span>}
    </button>
  );
}

export function Sidebar({ activePage, onNavigate, jsonEnabled, qrEnabled, pwdEnabled, sqlInEnabled, mdEnabled, fileSearchEnabled, fileDiffEnabled, jarViewerEnabled, encoderEnabled, cryptoEnabled, hasUpdate }: SidebarProps) {
  const { t } = useI18n();

  const [collapsed, setCollapsed] = useState(() => {
    const saved = localStorage.getItem('mtool_sidebar_collapsed');
    return saved === 'true';
  });

  useEffect(() => {
    localStorage.setItem('mtool_sidebar_collapsed', collapsed.toString());
  }, [collapsed]);

  const navItems = [
    ...(fileSearchEnabled ? [{ id: 'fileSearch', label: t('File Search'), icon: SearchCode }] : []),
    ...(fileDiffEnabled ? [{ id: 'fileDiff', label: t('File Compare'), icon: FileDiff }] : []),
    ...(jarViewerEnabled ? [{ id: 'jarViewer', label: t('Jar Viewer'), icon: Package }] : []),
    ...(encoderEnabled ? [{ id: 'encoder', label: t('Encoder / Decoder'), icon: ArrowRightLeft }] : []),
    ...(cryptoEnabled ? [{ id: 'crypto', label: t('Crypto Tool'), icon: Shield }] : []),
    ...(mdEnabled ? [{ id: 'md', label: t('Markdown Editor'), icon: FileText }] : []),
    ...(jsonEnabled ? [{ id: 'json', label: t('JSON Formatter'), icon: Code2 }] : []),
    ...(qrEnabled ? [{ id: 'qr', label: t('Text to QR'), icon: QrCode }] : []),
    ...(pwdEnabled ? [{ id: 'pwd', label: t('Password Generator'), icon: Key }] : []),
    ...(sqlInEnabled ? [{ id: 'sqlIn', label: t('SQL IN Builder'), icon: Database }] : []),
  ];

  return (
    <aside className={`${collapsed ? 'w-16' : 'w-64'} flex-shrink-0 flex flex-col border-r th-border th-bg-card transition-all duration-200 ease-in-out`} style={{ opacity: 0.95 }}>
      {/* Header */}
      <div className={`p-4 flex items-center ${collapsed ? 'justify-center' : 'justify-between'}`}>
        <div className={`flex items-center gap-3 ${collapsed ? '' : ''}`}>
          <div className="w-8 h-8 bg-indigo-500 rounded-lg flex items-center justify-center shadow-lg shadow-indigo-500/20 flex-shrink-0">
             <svg viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg" className="w-5 h-5 text-white">
                <path d="M4 20V5.5L10 12L16 5.5V19" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"/>
                <circle cx="10" cy="12" r="2" stroke="currentColor" strokeWidth="2.5"/>
                <path d="M4 16C4 16 2.5 17.5 2.5 19C2.5 20.5 4 21 4 21C4 21 5.5 20.5 5.5 19C5.5 17.5 4 16 4 16Z" stroke="currentColor" strokeWidth="2.5"/>
                <path d="M16 16C16 16 14.5 17.5 14.5 19C14.5 20.5 16 21 16 21C16 21 17.5 20.5 17.5 19C17.5 17.5 16 16 16 16Z" stroke="currentColor" strokeWidth="2.5"/>
             </svg>
          </div>
          {!collapsed && <span className="font-bold text-lg th-text tracking-tight">MTOOL</span>}
        </div>
        {!collapsed && (
          <button
            onClick={() => setCollapsed(true)}
            className="th-text-muted hover:th-text-2 transition-colors p-1 rounded th-hover-surface"
            title={t('Collapse Sidebar')}
          >
            <PanelLeftClose className="w-4 h-4" />
          </button>
        )}
      </div>

      {/* Expand button when collapsed */}
      {collapsed && (
        <div className="px-3 mb-2">
          <button
            onClick={() => setCollapsed(false)}
            className="w-full flex items-center justify-center py-1.5 th-text-muted hover:th-text-2 transition-colors rounded th-hover-surface"
            title={t('Expand Sidebar')}
          >
            <PanelLeftOpen className="w-4 h-4" />
          </button>
        </div>
      )}

      {/* Nav items */}
      <div className="flex-1 px-3 space-y-1">
        {!collapsed && (
          <div className="text-[10px] font-bold th-text-muted uppercase tracking-widest px-3 mb-2 mt-2">{t('Tools')}</div>
        )}
        {navItems.map((item) => (
          <NavItem key={item.id} item={item} activePage={activePage} collapsed={collapsed} onNavigate={onNavigate} />
        ))}
      </div>

      {/* Bottom items */}
      <div className="mt-auto p-3 space-y-1">
        {!collapsed && (
          <div className="text-[10px] font-bold th-text-muted uppercase tracking-widest px-3 mb-2">{t('System')}</div>
        )}
        {/* Settings with optional update dot */}
        <button
          onClick={() => onNavigate('settings')}
          title={collapsed ? t('Settings') : undefined}
          className={`w-full flex items-center ${collapsed ? 'justify-center' : ''} gap-3 px-3 py-2 rounded-md font-medium transition-colors ${
            activePage === 'settings'
              ? 'bg-indigo-600/10 text-indigo-400 border border-indigo-500/20 shadow-sm'
              : 'th-text-3 th-hover-surface border border-transparent'
          }`}
        >
          <span className="relative flex-shrink-0">
            <Settings className="w-[18px] h-[18px]" strokeWidth={2} />
            {hasUpdate && (
              <span className="absolute -top-1 -right-1 w-2 h-2 bg-amber-400 rounded-full" />
            )}
          </span>
          {!collapsed && <span className="text-[13px] truncate">{t('Settings')}</span>}
        </button>
        {/* User feature coming soon: <NavItem item={{ id: 'user', label: t('User'), icon: User }} /> */}
      </div>
    </aside>
  );
}
