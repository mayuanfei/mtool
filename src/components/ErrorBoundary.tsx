import { Component, ErrorInfo, ReactNode } from 'react';
import { useI18n, TranslationKey } from '../i18n';

interface Props {
  children: ReactNode;
  fallback?: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

interface InnerProps extends Props {
  t: (key: TranslationKey) => string;
}

class ErrorBoundaryInner extends Component<InnerProps, State> {
  public state: State = {
    hasError: false,
    error: null,
  };

  public static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  public componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error("ErrorBoundary caught an error:", error, errorInfo);
  }

  public render() {
    const { t } = this.props;
    if (this.state.hasError) {
      return (
        this.props.fallback || (
          <div className="flex flex-col items-center justify-center h-full p-6 text-center th-bg-main th-text">
            <div className="text-red-400 text-lg font-semibold mb-2">
              {t('Failed to load components, please restart the app or refresh')}
            </div>
            <div className="text-sm th-text-muted mb-6 max-w-md break-all">
              {this.state.error?.message || t('Unknown error occurred')}
            </div>
            <button
              onClick={() => window.location.reload()}
              className="px-4 py-2 bg-indigo-600 hover:bg-indigo-700 text-white rounded-md text-sm font-medium transition-colors"
            >
              {t('Reload')}
            </button>
          </div>
        )
      );
    }

    return this.props.children;
  }
}

export function ErrorBoundary(props: Props) {
  const { t } = useI18n();
  return <ErrorBoundaryInner {...props} t={t} />;
}
