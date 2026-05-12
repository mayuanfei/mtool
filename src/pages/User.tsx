import { User as UserIcon } from 'lucide-react';
import { useI18n } from '../i18n';

export function UserPage() {
  const { t } = useI18n();

  return (
    <div className="max-w-5xl mx-auto w-full">
      <div className="mb-10">
        <h1 className="text-4xl font-bold tracking-tight th-text mb-2">{t('User')}</h1>
        <p className="th-text-3">{t('User profile and preferences.')}</p>
      </div>
      <section className="th-bg-card border th-border rounded-xl overflow-hidden shadow-2xl p-12">
        <div className="flex flex-col items-center gap-4 th-text-muted">
          <UserIcon className="w-16 h-16" strokeWidth={1} />
          <p className="text-lg">{t('Coming soon')}</p>
        </div>
      </section>
    </div>
  );
}
