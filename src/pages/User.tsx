import { User as UserIcon } from 'lucide-react';
import { useI18n } from '../i18n';

export function UserPage() {
  const { t } = useI18n();

  return (
    <div className="max-w-4xl max-w-5xl mx-auto w-full">
      <div className="mb-10">
        <h1 className="text-4xl font-bold tracking-tight text-white mb-2">{t('User')}</h1>
        <p className="text-slate-400">{t('User profile and preferences.')}</p>
      </div>
      <section className="bg-slate-900 border border-slate-800 rounded-xl overflow-hidden shadow-2xl p-12">
        <div className="flex flex-col items-center gap-4 text-slate-500">
          <UserIcon className="w-16 h-16" strokeWidth={1} />
          <p className="text-lg">{t('Coming soon')}</p>
        </div>
      </section>
    </div>
  );
}
