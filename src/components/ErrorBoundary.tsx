import { Component, ErrorInfo, ReactNode } from 'react';

interface Props {
  children: ReactNode;
  fallback?: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
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
    if (this.state.hasError) {
      return (
        this.props.fallback || (
          <div className="flex flex-col items-center justify-center h-full p-6 text-center th-bg-main th-text">
            <div className="text-red-400 text-lg font-semibold mb-2">
              加载加载项失败，请重启应用或刷新
            </div>
            <div className="text-sm th-text-muted mb-6 max-w-md break-all">
              {this.state.error?.message || "Unknown error occurred"}
            </div>
            <button
              onClick={() => window.location.reload()}
              className="px-4 py-2 bg-indigo-600 hover:bg-indigo-700 text-white rounded-md text-sm font-medium transition-colors"
            >
              重新加载 / Reload
            </button>
          </div>
        )
      );
    }

    return this.props.children;
  }
}
