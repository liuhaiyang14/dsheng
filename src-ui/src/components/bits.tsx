import type { ReactNode } from 'react';
import { useApp } from '../state/store';

export function PanelNote({ children }: { children: ReactNode }) {
  return <span className="panel-note">{children}</span>;
}

/** 顶部错误横幅：展示核心 error envelope 的 message（脱敏中文文本）。 */
export function ErrorBanner() {
  const { error, clearError } = useApp();
  if (!error) return null;
  return (
    <div className="error-banner" role="alert" data-testid="error-banner">
      <div>
        <span className="code">{error.code}</span>
        <div>{error.message}</div>
      </div>
      <button
        type="button"
        className="outline"
        onClick={clearError}
        aria-label="关闭错误提示"
      >
        ×
      </button>
    </div>
  );
}

export function Tag({
  tone,
  children,
}: {
  tone: 'red' | 'green' | 'gold' | 'blue' | 'gray';
  children: ReactNode;
}) {
  return <span className={`tag ${tone}`}>{children}</span>;
}
