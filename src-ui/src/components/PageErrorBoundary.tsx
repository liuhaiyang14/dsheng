import { Component, type ErrorInfo, type ReactNode } from 'react';

/** 页面渲染异常仅隔离内容区；导航、企业切换与重试仍可用。 */
export class PageErrorBoundary extends Component<
  { children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('页面渲染失败', error, info.componentStack);
    void window.__TAURI__?.core.invoke('js_log', {
      message: `page-render: ${error.message}\n${info.componentStack ?? ''}`,
    }).catch(() => undefined);
  }

  render() {
    if (this.state.failed) {
      return (
        <div className="panel panel-body" role="alert">
          <h3>页面暂时无法显示</h3>
          <p>请重试加载，也可以通过左侧导航查看其他页面。</p>
          <button type="button" className="outline" onClick={() => this.setState({ failed: false })}>
            重新加载页面
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
