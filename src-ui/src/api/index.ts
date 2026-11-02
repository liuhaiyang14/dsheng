/**
 * 运行时客户端选择：
 * - 默认 TauriClient（window.__TAURI__ 可用时）；
 * - 无 Tauri 环境时返回 mode='none'，UI 显示「未连接核心」空状态，
 *   绝不静默回退到 FixtureClient；
 * - FixtureClient 仅在显式要求时启用（query ?dsheng-fixtures=1 或
 *   localStorage.dsheng_fixtures==='1'），用于页面开发与演示。
 */

import type { DshengClient } from './client';
import { FixtureClient } from './fixtureClient';
import { TauriClient, tauriAvailable } from './tauriClient';

export type RuntimeClientMode = 'tauri' | 'fixture' | 'none';

export interface RuntimeClient {
  mode: RuntimeClientMode;
  client: DshengClient | null;
}

export function fixtureExplicitlyRequested(): boolean {
  if (typeof window === 'undefined') return false;
  const params = new URLSearchParams(window.location.search);
  if (params.get('dsheng-fixtures') === '1') return true;
  try {
    return window.localStorage.getItem('dsheng_fixtures') === '1';
  } catch {
    return false;
  }
}

export function createRuntimeClient(): RuntimeClient {
  if (tauriAvailable()) {
    return { mode: 'tauri', client: new TauriClient() };
  }
  if (fixtureExplicitlyRequested()) {
    return { mode: 'fixture', client: new FixtureClient() };
  }
  return { mode: 'none', client: null };
}

export * from './types';
export * from './format';
export type {
  AnalyzeInput,
  DiagnosisContinueRequest,
  DiagnosisGenerateRequest,
  DshengClient,
  ExportResult,
  TaskGenerateRequest,
  TaskReconcileDryRunResult,
  TaskTimelineQuery,
} from './client';
export { FixtureClient } from './fixtureClient';
export { FIXTURE_DEMO_ANNOTATION } from './fixtureData';
