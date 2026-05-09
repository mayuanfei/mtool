import { X } from 'lucide-react';
import { useI18n } from '../i18n';
import type { UpdateInfo } from '../updater';

interface UpdateModalProps {
  open: boolean;
  updateInfo: UpdateInfo;
  downloading: boolean;
  progress: number;
  error: string | null;
  installed: boolean;
  onClose: () => void;
  onInstall: () => void;
  onRelaunch: () => void;
}

export function UpdateModal({ open, updateInfo, downloading, progress, error, installed, onClose, onInstall, onRelaunch }: UpdateModalProps) {
  const { t } = useI18n();
  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="th-bg-card border th-border rounded-2xl shadow-2xl w-full max-w-lg mx-4 overflow-hidden">
        {/* Header */}
        <div className="px-6 py-4 border-b th-border flex items-center justify-between th-bg-surface-h">
          <h2 className="text-base font-semibold th-text">
            {t("What's new in")} v{updateInfo.version}
          </h2>
          {!downloading && !installed && (
            <button
              onClick={onClose}
              className="th-text-muted hover:th-text-2 transition-colors rounded p-1 th-hover-surface"
            >
              <X className="w-4 h-4" />
            </button>
          )}
        </div>

        {/* Changelog */}
        <div className="px-6 py-4 max-h-64 overflow-y-auto">
          {updateInfo.notes ? (
            <pre className="text-sm th-text-3 whitespace-pre-wrap font-sans leading-relaxed">
              {updateInfo.notes}
            </pre>
          ) : (
            <p className="text-sm th-text-muted italic">No changelog provided.</p>
          )}
        </div>

        {/* Progress bar */}
        {(downloading || installed) && (
          <div className="px-6 pb-4">
            <div className="w-full bg-slate-200 dark:bg-slate-700 rounded-full h-2 overflow-hidden">
              <div
                className="bg-indigo-500 h-2 rounded-full transition-all duration-300"
                style={{ width: `${progress}%` }}
              />
            </div>
            <p className="text-xs th-text-muted mt-2 text-center">
              {installed ? t('Ready to restart') : `${t('Downloading...')} ${progress}%`}
            </p>
          </div>
        )}

        {/* Error */}
        {error && (
          <div className="px-6 pb-4">
            <p className="text-xs text-red-400">{t('Update error')}: {error}</p>
          </div>
        )}

        {/* Actions */}
        <div className="px-6 py-4 border-t th-border flex justify-end gap-3 th-bg-surface-h">
          {!downloading && !installed && (
            <button
              onClick={onClose}
              className="px-4 py-2 text-sm th-text-3 th-bg-input-alt border th-border-subtle rounded-lg th-hover-surface transition-colors"
            >
              {t('Cancel')}
            </button>
          )}
          {installed ? (
            <button
              onClick={onRelaunch}
              className="px-4 py-2 text-sm font-medium bg-indigo-600 hover:bg-indigo-700 text-white rounded-lg transition-colors"
            >
              {t('Restart Now')}
            </button>
          ) : (
            <button
              onClick={onInstall}
              disabled={downloading}
              className="px-4 py-2 text-sm font-medium bg-indigo-600 hover:bg-indigo-700 disabled:opacity-60 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
            >
              {downloading ? `${t('Downloading...')} ${progress}%` : t('Install & Restart')}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
