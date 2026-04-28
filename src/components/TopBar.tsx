import { Bell } from 'lucide-react';

export function TopBar() {
  return (
    <header className="h-14 border-b border-slate-800 flex items-center justify-end px-6 bg-slate-900/20">
      <button className="text-slate-400 hover:text-white transition-colors relative">
        <Bell className="w-5 h-5" />
        <span className="absolute top-0 right-0 w-2 h-2 bg-indigo-500 rounded-full border border-slate-900"></span>
      </button>
    </header>
  );
}
