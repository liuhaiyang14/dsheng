/**
 * 应用状态层（纯前端状态；无业务计算）。
 *
 * - 顶部企业切换 = workspace use 语义：切换后仅加载该档案自己的
 *   数据（provider/period/task/history/scan），绝不跨档案读取；
 * - 空白档案（无扫描文件/无历史/无任务）在所有数据屏显示空状态；
 * - 业务规则与数值全部来自 client（核心）返回；本层只做缓存与呈现状态。
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import {
  createRuntimeClient,
  type DshengClient,
  type Cadence,
  type HistoryDetailView,
  type HistoryEntryView,
  type PeriodPolicyView,
  type ProviderView,
  type RuntimeClientMode,
  type ScanSummaryView,
  type TaskView,
  type WorkspaceView,
} from '../api';
import { DshengCliError } from '../api';

export type ScreenId =
  | 'source'
  | 'analysis'
  | 'diagnosis'
  | 'task'
  | 'history'
  | 'provider'
  | 'system-settings';

export const SCREEN_LABELS: Record<ScreenId, string> = {
  source: '数据源',
  analysis: '经营分析',
  diagnosis: '经营问题诊断',
  task: '任务单',
  history: '历史查询',
  provider: 'Provider 设置',
  'system-settings': '系统设置',
};

export interface UiError {
  code: string;
  message: string;
}

interface WorkspaceData {
  scan: ScanSummaryView | null;
  policies: PeriodPolicyView[];
  providers: ProviderView[];
  tasks: TaskView[];
  history: HistoryEntryView[];
  currentDetail: HistoryDetailView | null;
  loaded: boolean;
}

const EMPTY_WORKSPACE_DATA: WorkspaceData = {
  scan: null,
  policies: [],
  providers: [],
  tasks: [],
  history: [],
  currentDetail: null,
  loaded: false,
};

interface RunSelection {
  runId: string;
  /** undefined 表示跟随该期间当前版本；数字表示明确的历史快照。 */
  revision?: number;
}

interface AppState {
  mode: RuntimeClientMode;
  client: DshengClient | null;
  /** true=已连核心（或显式 fixture）；false=未连接核心空状态。 */
  connected: boolean;
  /** 首批 workspace 列表已加载完成（区分「加载中」与「确实没有档案」）。 */
  bootstrapped: boolean;
  workspaces: WorkspaceView[];
  activeWorkspaceId: string | null;
  activeWorkspace: WorkspaceView | null;
  data: WorkspaceData;
  screen: ScreenId;
  selectedRun: RunSelection | null;
  sourceDirectory: string | null;
  contextLockReason: string | null;
  setContextLock: (reason: string | null) => void;
  rememberSourceDirectory: (path: string) => void;
  selectRun: (runId: string, revision?: number) => Promise<void>;
  runLocalAnalysis: (period: { from: string; to: string; grain: Cadence }) => Promise<void>;
  error: UiError | null;
  loading: boolean;
  setScreen: (screen: ScreenId) => void;
  /** 创建企业档案（首启向导与顶栏「新建档案」共用；创建后切换为当前档案）。 */
  createWorkspace: (input: {
    name: string;
    cadence: 'day' | 'week' | 'month' | 'quarter';
    effective_from: string;
    timezone_id: string;
    week_start_day: number;
    fiscal_year_start_month: number;
  }) => Promise<WorkspaceView>;
  switchWorkspace: (workspaceId: string) => Promise<void>;
  refresh: () => Promise<void>;
  clearError: () => void;
  /** 用核心返回刷新当前档案数据（供各屏在显式操作后调用；keepScan 显式保留扫描摘要）。 */
  reloadWorkspaceData: (opts?: { keepScan?: ScanSummaryView | null }) => Promise<void>;
}

const AppContext = createContext<AppState | null>(null);

function toUiError(err: unknown): UiError {
  if (err instanceof DshengCliError) {
    return { code: err.code, message: err.message };
  }
  if (err instanceof Error) {
    return { code: 'UNKNOWN', message: err.message };
  }
  return { code: 'UNKNOWN', message: String(err) };
}

function sourceDirectoryKey(workspaceId: string) {
  return `dsheng.source-directory.${workspaceId}`;
}

function savedSourceDirectory(workspaceId: string): string | null {
  try { return window.localStorage.getItem(sourceDirectoryKey(workspaceId)) || null; }
  catch { return null; }
}

export function AppProvider({
  children,
  runtime,
}: {
  children: ReactNode;
  runtime?: { mode: RuntimeClientMode; client: DshengClient | null };
}) {
  const [mode, setMode] = useState<RuntimeClientMode>('none');
  const [client, setClient] = useState<DshengClient | null>(null);
  const [workspaces, setWorkspaces] = useState<WorkspaceView[]>([]);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);
  const [bootstrapped, setBootstrapped] = useState(false);
  const [dataMap, setDataMap] = useState<Record<string, WorkspaceData>>({});
  const [selectionMap, setSelectionMap] = useState<Record<string, RunSelection | null>>({});
  const [directoryMap, setDirectoryMap] = useState<Record<string, string>>({});
  const [screen, setScreenState] = useState<ScreenId>('source');
  const [contextLockReason, setContextLockReason] = useState<string | null>(null);
  const contextLockRef = useRef<string | null>(null);
  const setContextLock = useCallback((reason: string | null) => {
    contextLockRef.current = reason;
    setContextLockReason(reason);
  }, []);
  const setScreen = useCallback((next: ScreenId) => {
    if (!contextLockRef.current) setScreenState(next);
  }, []);
  const [error, setError] = useState<UiError | null>(null);
  const [loading, setLoading] = useState(false);
  // 跨页面使用同一选择；过期的企业/期间/刷新结果不得覆盖新选择。
  const requestEpoch = useRef(0);
  const activeWorkspaceRef = useRef<string | null>(null);
  const dataRef = useRef(dataMap);
  dataRef.current = dataMap;
  const selectionRef = useRef(selectionMap);
  selectionRef.current = selectionMap;
  const directoryRef = useRef(directoryMap);
  directoryRef.current = directoryMap;

  const runtimeRef = useRef(runtime);
  useEffect(() => {
    const rt = runtimeRef.current ?? createRuntimeClient();
    setMode(rt.mode);
    setClient(rt.client);
  }, []);

  const activeWorkspace = useMemo(
    () => workspaces.find((w) => w.workspace_id === activeWorkspaceId) ?? null,
    [workspaces, activeWorkspaceId],
  );

  const rememberSelection = useCallback((workspaceId: string, selection: RunSelection | null) => {
    selectionRef.current = { ...selectionRef.current, [workspaceId]: selection };
    setSelectionMap(selectionRef.current);
  }, []);

  const loadWorkspaceData = useCallback(async (
    target: DshengClient,
    workspaceId: string,
    preferred: RunSelection | null | undefined,
    keepScan?: ScanSummaryView | null,
  ) => {
    const [policies, providers, history] = await Promise.all([
      target.periodList(workspaceId).catch(() => []),
      target.providerList(workspaceId).catch(() => []),
      target.historyList(workspaceId),
    ]);
    // 缺省选期间日期最大的已保存结果，绝不按更新时间冒充“当期”。
    const newest = [...history].sort((a, b) =>
      b.period.to.localeCompare(a.period.to) || b.period.from.localeCompare(a.period.from) || a.run_id.localeCompare(b.run_id),
    )[0];
    const entry = history.find((h) => h.run_id === preferred?.runId) ?? newest;
    const selection = entry
      ? { runId: entry.run_id, revision: preferred?.runId === entry.run_id ? preferred.revision : undefined }
      : null;
    const [currentDetail, tasks] = entry
      ? await Promise.all([
          target.historyShow(workspaceId, entry.run_id, selection?.revision),
          target.taskList(workspaceId, entry.run_id),
        ])
      : [null, []];
    const data: WorkspaceData = {
      scan: keepScan ?? null, policies, providers, history, tasks, currentDetail, loaded: true,
    };
    return { data, selection };
  }, []);

  const applyLoaded = useCallback((workspaceId: string, loaded: Awaited<ReturnType<typeof loadWorkspaceData>>) => {
    setDataMap((prev) => ({ ...prev, [workspaceId]: loaded.data }));
    rememberSelection(workspaceId, loaded.selection);
  }, [rememberSelection]);

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    const epoch = ++requestEpoch.current;
    setLoading(true);
    void (async () => {
      try {
        const list = await client.workspaceList();
        if (cancelled || epoch !== requestEpoch.current) return;
        setWorkspaces(list);
        setBootstrapped(true);
        if (list.length > 0) {
          const first = list[0].workspace_id;
          activeWorkspaceRef.current = first;
          setActiveWorkspaceId(first);
          const loaded = await loadWorkspaceData(client, first, null);
          if (!cancelled && epoch === requestEpoch.current) applyLoaded(first, loaded);
        }
      } catch (err) {
        if (!cancelled && epoch === requestEpoch.current) setError(toUiError(err));
      } finally {
        if (!cancelled && epoch === requestEpoch.current) {
          setBootstrapped(true);
          setLoading(false);
        }
      }
    })();
    return () => { cancelled = true; };
  }, [client, loadWorkspaceData, applyLoaded]);

  const reloadWorkspaceData = useCallback(async (opts?: { keepScan?: ScanSummaryView | null }) => {
    const workspaceId = activeWorkspaceId;
    if (!client || !workspaceId || activeWorkspaceRef.current !== workspaceId) return;
    const epoch = ++requestEpoch.current;
    setLoading(true);
    setError(null);
    try {
      const loaded = await loadWorkspaceData(
        client, workspaceId, selectionRef.current[workspaceId],
        opts?.keepScan !== undefined ? opts.keepScan : dataRef.current[workspaceId]?.scan,
      );
      const fresh = await client.workspaceList();
      if (epoch !== requestEpoch.current || activeWorkspaceRef.current !== workspaceId) return;
      applyLoaded(workspaceId, loaded);
      setWorkspaces(fresh);
    } catch (err) {
      if (epoch === requestEpoch.current) setError(toUiError(err));
    } finally {
      if (epoch === requestEpoch.current) setLoading(false);
    }
  }, [client, activeWorkspaceId, loadWorkspaceData, applyLoaded]);

  const selectRun = useCallback(async (runId: string, revision?: number) => {
    const workspaceId = activeWorkspaceId;
    if (!client || !workspaceId || activeWorkspaceRef.current !== workspaceId || contextLockRef.current) return;
    if (!dataRef.current[workspaceId]?.history.some((h) => h.run_id === runId)) return;
    const selection = { runId, revision };
    const epoch = ++requestEpoch.current;
    rememberSelection(workspaceId, selection);
    setDataMap((prev) => ({ ...prev, [workspaceId]: { ...prev[workspaceId], currentDetail: null, tasks: [] } }));
    setError(null);
    setLoading(true);
    try {
      const loaded = await loadWorkspaceData(client, workspaceId, selection, dataRef.current[workspaceId]?.scan);
      if (epoch === requestEpoch.current && activeWorkspaceRef.current === workspaceId) applyLoaded(workspaceId, loaded);
    } catch (err) {
      if (epoch === requestEpoch.current) setError(toUiError(err));
    } finally {
      if (epoch === requestEpoch.current) setLoading(false);
    }
  }, [client, activeWorkspaceId, loadWorkspaceData, applyLoaded, rememberSelection]);

  const rememberSourceDirectory = useCallback((path: string) => {
    if (!activeWorkspaceId) return;
    directoryRef.current = { ...directoryRef.current, [activeWorkspaceId]: path };
    setDirectoryMap(directoryRef.current);
    // 仅保存用户选择的目录路径，不保存表内容、Provider 或 Key。
    try { window.localStorage.setItem(sourceDirectoryKey(activeWorkspaceId), path); } catch { /* 受限环境仍保留内存状态 */ }
  }, [activeWorkspaceId]);

  const runLocalAnalysis = useCallback(async (period: { from: string; to: string; grain: Cadence }) => {
    const workspaceId = activeWorkspaceId;
    if (!client || !workspaceId || activeWorkspaceRef.current !== workspaceId || contextLockRef.current) return;
    const directory = directoryRef.current[workspaceId] ?? savedSourceDirectory(workspaceId);
    if (!directory) {
      setError({ code: 'DIRECTORY_REQUIRED', message: '请先到数据源选择并成功扫描真实目录，再运行本地分析。' });
      return;
    }
    if (!period.from || !period.to || period.from > period.to) {
      setError({ code: 'INVALID_PERIOD', message: '请填写有效的起止日期，开始日期不能晚于结束日期。' });
      return;
    }
    const epoch = ++requestEpoch.current;
    setError(null);
    setLoading(true);
    try {
      const run = await client.analyze({ workspace_id: workspaceId, directory_label: directory, ...period });
      if (epoch !== requestEpoch.current || activeWorkspaceRef.current !== workspaceId) return;
      const loaded = await loadWorkspaceData(client, workspaceId, { runId: run.run_id }, dataRef.current[workspaceId]?.scan);
      if (epoch === requestEpoch.current && activeWorkspaceRef.current === workspaceId) applyLoaded(workspaceId, loaded);
    } catch (err) {
      if (epoch === requestEpoch.current) setError(toUiError(err));
    } finally {
      if (epoch === requestEpoch.current) setLoading(false);
    }
  }, [client, activeWorkspaceId, loadWorkspaceData, applyLoaded]);

  const createWorkspace = useCallback(async (input: {
    name: string; cadence: Cadence; effective_from: string; timezone_id: string;
    week_start_day: number; fiscal_year_start_month: number;
  }): Promise<WorkspaceView> => {
    if (!client) throw new Error('未连接核心，无法创建企业档案');
    if (contextLockRef.current) throw new Error(contextLockRef.current);
    const epoch = ++requestEpoch.current;
    setError(null);
    setLoading(true);
    try {
      const view = await client.workspaceCreate({ schema_version: '1', operation: 'workspace_create', name: input.name,
        initial_period_policy: { cadence: input.cadence, week_start_day: input.week_start_day,
          fiscal_year_start_month: input.fiscal_year_start_month, timezone_id: input.timezone_id, effective_from: input.effective_from } });
      const list = await client.workspaceList();
      const loaded = await loadWorkspaceData(client, view.workspace_id, null);
      if (epoch === requestEpoch.current) {
        setWorkspaces(list);
        activeWorkspaceRef.current = view.workspace_id;
        setActiveWorkspaceId(view.workspace_id);
        applyLoaded(view.workspace_id, loaded);
        setScreenState('source');
      }
      return view;
    } catch (err) {
      if (epoch === requestEpoch.current) setError(toUiError(err));
      throw err;
    } finally {
      if (epoch === requestEpoch.current) setLoading(false);
    }
  }, [client, loadWorkspaceData, applyLoaded]);

  const switchWorkspace = useCallback(async (workspaceId: string) => {
    if (!client || workspaceId === activeWorkspaceRef.current || contextLockRef.current) return;
    const epoch = ++requestEpoch.current;
    // 立即隔离旧页面、预览和候选，再读取目标企业数据。
    activeWorkspaceRef.current = workspaceId;
    setActiveWorkspaceId(workspaceId);
    setError(null);
    setLoading(true);
    try {
      await client.workspaceUse({ schema_version: '1', operation: 'workspace_use', workspace_id: workspaceId });
      const loaded = await loadWorkspaceData(client, workspaceId, selectionRef.current[workspaceId], dataRef.current[workspaceId]?.scan);
      if (epoch === requestEpoch.current) applyLoaded(workspaceId, loaded);
    } catch (err) {
      if (epoch === requestEpoch.current) setError(toUiError(err));
    } finally {
      if (epoch === requestEpoch.current) setLoading(false);
    }
  }, [client, loadWorkspaceData, applyLoaded]);

  const value: AppState = {
    mode, client, connected: client != null, bootstrapped, workspaces, activeWorkspaceId, activeWorkspace,
    data: activeWorkspaceId ? dataMap[activeWorkspaceId] ?? EMPTY_WORKSPACE_DATA : EMPTY_WORKSPACE_DATA,
    selectedRun: activeWorkspaceId ? selectionMap[activeWorkspaceId] ?? null : null,
    sourceDirectory: activeWorkspaceId ? directoryMap[activeWorkspaceId] ?? savedSourceDirectory(activeWorkspaceId) : null,
    rememberSourceDirectory, selectRun, runLocalAnalysis, contextLockReason, setContextLock,
    screen, error, loading, setScreen, createWorkspace, switchWorkspace,
    refresh: reloadWorkspaceData, clearError: () => setError(null), reloadWorkspaceData,
  };
  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppState {
  const ctx = useContext(AppContext);
  if (!ctx) {
    throw new Error('useApp 必须在 AppProvider 内使用');
  }
  return ctx;
}

/** 当前档案是否有数据（用于空档案空状态，用例 34）。 */
export function workspaceHasData(data: WorkspaceData): boolean {
  return (
    (data.scan?.files.length ?? 0) > 0 ||
    data.history.length > 0 ||
    data.tasks.length > 0
  );
}
