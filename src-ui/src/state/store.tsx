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

export function AppProvider({
  children,
  runtime,
}: {
  children: ReactNode;
  /** 测试可注入 mock/fixture client。 */
  runtime?: { mode: RuntimeClientMode; client: DshengClient | null };
}) {
  const [mode, setMode] = useState<RuntimeClientMode>('none');
  const [client, setClient] = useState<DshengClient | null>(null);
  const [workspaces, setWorkspaces] = useState<WorkspaceView[]>([]);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);
  const [bootstrapped, setBootstrapped] = useState(false);
  const [dataMap, setDataMap] = useState<Record<string, WorkspaceData>>({});
  const [screen, setScreen] = useState<ScreenId>('source');
  const [error, setError] = useState<UiError | null>(null);
  const [loading, setLoading] = useState(false);

  // runtime 只在挂载时取一次，避免每次渲染重建 client 导致的循环。
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

  const loadWorkspaceData = useCallback(
    async (
      target: DshengClient,
      workspaceId: string,
      /** 保留既有扫描摘要：真实扫描结果来自用户显式操作，刷新不得用空目录扫描覆盖。 */
      keepScan?: ScanSummaryView | null,
    ): Promise<WorkspaceData> => {
      // 只读取该档案自己的数据；跨档案隔离由核心保证，这里不合并其他档案缓存。
      const [policies, providers, tasks, history] = await Promise.all([
        target.periodList(workspaceId).catch(() => []),
        target.providerList(workspaceId).catch(() => []),
        target.taskList(workspaceId).catch(() => []),
        target.historyList(workspaceId).catch(() => []),
      ]);
      let currentDetail: HistoryDetailView | null = null;
      if (history.length > 0) {
        currentDetail = await target
          .historyShow(workspaceId, history[0].run_id)
          .catch(() => null);
      }
      return {
        scan: keepScan ?? null,
        policies,
        providers,
        tasks,
        history,
        currentDetail,
        loaded: true,
      };
    },
    [],
  );

  // 初始加载：workspace 列表 + 默认档案数据
  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!client) return;
      setLoading(true);
      try {
        const list = await client.workspaceList();
        if (cancelled) return;
        setWorkspaces(list);
        setBootstrapped(true);
        if (list.length > 0) {
          const first = list[0].workspace_id;
          setActiveWorkspaceId(first);
          const loaded = await loadWorkspaceData(client, first);
          if (cancelled) return;
          setDataMap({ [first]: loaded });
        }
      } catch (err) {
        if (!cancelled) setError(toUiError(err));
        if (!cancelled) setBootstrapped(true);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, loadWorkspaceData]);

  const reloadWorkspaceData = useCallback(
    async (opts?: { keepScan?: ScanSummaryView | null }) => {
    if (!client || !activeWorkspaceId) return;
    setLoading(true);
    try {
      const loaded = await loadWorkspaceData(
        client,
        activeWorkspaceId,
        opts?.keepScan !== undefined ? opts.keepScan : (dataMap[activeWorkspaceId]?.scan ?? null),
      );
      setDataMap((prev) => ({ ...prev, [activeWorkspaceId]: loaded }));
      const fresh = await client.workspaceList();
      setWorkspaces(fresh);
    } catch (err) {
      setError(toUiError(err));
    } finally {
      setLoading(false);
    }
  },
  [client, activeWorkspaceId, loadWorkspaceData, dataMap]);

  const createWorkspace = useCallback(
    async (input: {
      name: string;
      cadence: 'day' | 'week' | 'month' | 'quarter';
      effective_from: string;
      timezone_id: string;
      week_start_day: number;
      fiscal_year_start_month: number;
    }): Promise<WorkspaceView> => {
      if (!client) throw new Error('未连接核心，无法创建企业档案');
      setError(null);
      setLoading(true);
      try {
        const view = await client.workspaceCreate({
          schema_version: '1',
          operation: 'workspace_create',
          name: input.name,
          initial_period_policy: {
            cadence: input.cadence,
            week_start_day: input.week_start_day,
            fiscal_year_start_month: input.fiscal_year_start_month,
            timezone_id: input.timezone_id,
            effective_from: input.effective_from,
          },
        });
        const list = await client.workspaceList();
        setWorkspaces(list);
        const loaded = await loadWorkspaceData(client, view.workspace_id);
        setActiveWorkspaceId(view.workspace_id);
        setDataMap((prev) => ({ ...prev, [view.workspace_id]: loaded }));
        setScreen('source');
        return view;
      } catch (err) {
        setError(toUiError(err));
        throw err;
      } finally {
        setLoading(false);
      }
    },
    [client, loadWorkspaceData],
  );

  const switchWorkspace = useCallback(
    async (workspaceId: string) => {
      if (!client || workspaceId === activeWorkspaceId) return;
      setError(null);
      setLoading(true);
      try {
        await client.workspaceUse({
          schema_version: '1',
          operation: 'workspace_use',
          workspace_id: workspaceId,
        });
        // 切换后只呈现目标档案自己的数据（无缓存跨用）。
        const loaded = await loadWorkspaceData(client, workspaceId);
        setActiveWorkspaceId(workspaceId);
        setDataMap((prev) => ({ ...prev, [workspaceId]: loaded }));
      } catch (err) {
        setError(toUiError(err));
      } finally {
        setLoading(false);
      }
    },
    [client, activeWorkspaceId, loadWorkspaceData],
  );

  const value: AppState = {
    mode,
    client,
    connected: client != null,
    bootstrapped,
    workspaces,
    activeWorkspaceId,
    activeWorkspace,
    data: activeWorkspaceId
      ? dataMap[activeWorkspaceId] ?? EMPTY_WORKSPACE_DATA
      : EMPTY_WORKSPACE_DATA,
    screen,
    error,
    loading,
    setScreen,
    createWorkspace,
    switchWorkspace,
    refresh: reloadWorkspaceData,
    clearError: () => setError(null),
    reloadWorkspaceData,
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
