import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import { check as checkUpdate } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import {
  Button,
  Card,
  Divider,
  Empty,
  Form,
  Input,
  InputNumber,
  Layout,
  Message,
  Modal,
  Progress,
  Select,
  Space,
  Switch,
  Tabs,
  Tag,
  Alert,
  Descriptions,
  Statistic,
  Radio,
  Collapse
} from '@arco-design/web-react';import {
  IconCheckCircle,
  IconDelete,
  IconDownload,
  IconExport,
  IconFolder,
  IconRefresh,
  IconSettings,
  IconSync,
  IconInfoCircle,
  IconFile,
  IconLeft,
  IconRight
} from '@arco-design/web-react/icon';

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

type Settings = {
  lastFileRoot: string;
  aiEnabled: boolean;
  aiBaseUrl: string;
  aiApiKey: string;
  aiModel: string;
  ocrBaseUrl: string;
  ocrApiKey: string;
  requestTimeoutSeconds: number;
  imageMaxKb: number;
  themeMode: string;
};

type Outputs = {
  mode: string;
  updatedAt: string;
  pendingSuccessCount: number;
  failedCount: number;
  projectCount: number;
  downloadedProjectNames: string[];
};

type SessionStatus = {
  state: string;
  message: string;
  browserName: string;
  account: string;
  displayName: string;
  checkedAt: string;
};

type AppState = {
  windowTitle: string;
  settings: Settings;
  session: SessionStatus;
  logs: string[];
  outputs: Outputs;
};

type SuccessExportResult = {
  workbookPath: string;
  status: string;
  pendingCount: number;
  appendedCount: number;
  duplicateCount: number;
};

type BusyState = { active: boolean; text: string };
type ActionOptions = { showProgress?: boolean; log?: boolean; toast?: boolean; resetLogs?: boolean };
type SyncOptions = { resetLogs?: boolean; replaceLogs?: boolean };
type ProjectCompareLogRow = {
  fileName: string;
  projectCode: string;
  projectName: string;
  contactName: string;
  contactPhone: string;
  acceptanceTime: string;
  startTime: string;
  amount?: string;
  hasRedStamp?: string;
};
type ProjectCompareLog = {
  projectName: string;
  projectCode: string;
  passed: boolean;
  summary: string;
  finishedAt: string;
  rows: ProjectCompareLogRow[];
};
type WorkflowProgress = {
  taskId: string;
  stage: string;
  status: string;
  current: number;
  total: number;
  percent: number;
  message: string;
  projectName: string;
  projectLog?: ProjectCompareLog | null;
};

const TabPane = Tabs.TabPane;

const emptySettings: Settings = {
  lastFileRoot: '',
  aiEnabled: true,
  aiBaseUrl: '',
  aiApiKey: '',
  aiModel: '',
  ocrBaseUrl: '',
  ocrApiKey: '',
  requestTimeoutSeconds: 30,
  imageMaxKb: 100,
  themeMode: 'light'
};

const emptyOutputs: Outputs = {
  mode: '',
  updatedAt: '',
  pendingSuccessCount: 0,
  failedCount: 0,
  projectCount: 0,
  downloadedProjectNames: []
};

const emptySession: SessionStatus = {
  state: 'unknown',
  message: '',
  browserName: '内置 WebView',
  account: '',
  displayName: '',
  checkedAt: ''
};

const themeOptions = [
  { label: '白天', value: 'light' },
  { label: '跟随系统', value: 'system' },
  { label: '夜间', value: 'dark' }
];

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && Boolean(window.__TAURI_INTERNALS__);
}

function previewState(): AppState {
  return {
    windowTitle: '项目资料比对助手',
    settings: emptySettings,
    session: { ...emptySession, state: 'missing', message: '未登录' },
    logs: [],
    outputs: emptyOutputs
  };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    throw new Error('当前页面未运行在 Tauri 应用内');
  }
  return await invoke<T>(command, args);
}

export default function App() {
  const [state, setState] = useState<AppState | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [busy, setBusy] = useState<BusyState>({ active: false, text: '' });
  const [settingsVisible, setSettingsVisible] = useState(false);
  const [siderCollapsed, setSiderCollapsed] = useState(false);
  const [settingsDraft, setSettingsDraft] = useState<Settings>(emptySettings);
  const [progress, setProgress] = useState<WorkflowProgress | null>(null);
  const [logFilter, setLogFilter] = useState<'all' | 'success' | 'failed' | 'system'>('all');
  const [appVersion, setAppVersion] = useState<string>('');
  const logEndRef = useRef<HTMLDivElement | null>(null);

  const settings = state?.settings ?? emptySettings;
  const outputs = state?.outputs ?? emptyOutputs;
  const session = state?.session ?? emptySession;
  const isBusy = busy.active;
  const isSaving = busy.active && busy.text === '保存中';
  const isSessionReady = session.state === 'ok';

  useEffect(() => {
    void bootstrap();
  }, []);

  useEffect(() => {
    getVersion()
      .then((v) => setAppVersion(v))
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return () => undefined;
    }
    let unlisten: UnlistenFn | null = null;
    let mounted = true;
    void listen<WorkflowProgress>('workflow-progress', (event) => {
      if (mounted) {
        setProgress(event.payload);
        if (event.payload.projectLog) {
          setLogs((current) => [...current, projectLogLine(event.payload.projectLog as ProjectCompareLog)]);
        }
      }
    }).then((handler) => {
      if (mounted) {
        unlisten = handler;
      } else {
        handler();
      }
    });
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    return applyTheme(settings.themeMode);
  }, [settings.themeMode]);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return () => undefined;
    }
    let unlisten: UnlistenFn | null = null;
    let mounted = true;
    void listen('auth://cookies-updated', () => {
      if (mounted) {
        void refreshSessionAfterLogin();
      }
    }).then((handler) => {
      if (mounted) {
        unlisten = handler;
      } else {
        handler();
      }
    });
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, []);

  // 解析出所有的比对日志，提供统一过滤器
  const allParsedLogs = useMemo(() => {
    return logs.filter(shouldShowRuntimeLog).map((line) => {
      const projectLog = parseCompareLogLine(line);
      return {
        line,
        projectLog
      };
    });
  }, [logs]);

  // 统计不同状态下的日志量，用于显示在 Radio.Button 上
  const statsCounts = useMemo(() => {
    let success = 0;
    let failed = 0;
    let system = 0;
    allParsedLogs.forEach(item => {
      if (item.projectLog) {
        if (item.projectLog.passed) {
          success++;
        } else {
          failed++;
        }
      } else {
        system++;
      }
    });
    return {
      all: allParsedLogs.length,
      success,
      failed,
      system
    };
  }, [allParsedLogs]);

  // 根据当前过滤器筛选出的日志
  const filteredLogs = useMemo(() => {
    if (logFilter === 'all') return allParsedLogs;
    if (logFilter === 'success') return allParsedLogs.filter(item => item.projectLog?.passed === true);
    if (logFilter === 'failed') return allParsedLogs.filter(item => item.projectLog && item.projectLog.passed === false);
    return allParsedLogs.filter(item => !item.projectLog);
  }, [allParsedLogs, logFilter]);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ block: 'nearest' });
  }, [filteredLogs.length]);

  async function bootstrap(): Promise<void> {
    if (!isTauriRuntime()) {
      sync(previewState(), { resetLogs: true });
      return;
    }
    try {
      sync(await call<AppState>('bootstrap'), { resetLogs: true });
      void refreshSession(false);
    } catch (error) {
      reportError(error);
    }
  }

  function sync(next: AppState, options: SyncOptions = {}): void {
    document.title = next.windowTitle;
    setState((previous) => {
      const currentSession = previous?.session;
      const sessionValue = next.session.state === 'unknown' && currentSession ? currentSession : next.session;
      return { ...next, session: sessionValue };
    });
    if (options.resetLogs) {
      setLogs([]);
    } else if (options.replaceLogs) {
      setLogs(next.logs);
    }
  }

  function appendUiLog(message: string): void {
    const timestamp = new Date().toLocaleString('zh-CN', { hour12: false });
    setLogs((current) => [...current, `${timestamp} | ${message}`]);
  }

  function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  function reportError(error: unknown): void {
    const message = errorMessage(error);
    appendUiLog(`执行失败: ${message}`);
    Message.error(message);
  }

  function showError(error: unknown): void {
    const message = error instanceof Error ? error.message : String(error);
    Message.error(message);
  }

  function currentFileRoot(): string {
    return state?.settings.lastFileRoot || '';
  }

  function currentDownloadRoot(): string {
    const root = currentFileRoot().replace(/[\\/]+$/, '');
    if (!root) {
      return '';
    }
    const separator = root.includes('\\') ? '\\' : '/';
    return `${root}${separator}file`;
  }

  async function runPlainAction(
    runningText: string,
    successText: string,
    action: () => Promise<unknown>,
    options: ActionOptions = {}
  ): Promise<void> {
    const showProgress = options.showProgress ?? false;
    const shouldLog = options.log ?? true;
    const shouldToast = options.toast ?? true;
    if (options.resetLogs) {
      setLogs([]);
    }
    setBusy({ active: true, text: runningText });
    setProgress(showProgress ? defaultProgressFor(runningText) : null);
    if (shouldLog) {
      appendUiLog(runningText);
    }
    try {
      await action();
      if (shouldLog) {
        appendUiLog(successText);
      }
      if (shouldToast) {
        Message.success(successText);
      }
      if (showProgress) {
        setProgress((current) => current ? { ...current, status: 'done', percent: 100, message: successText } : null);
      }
    } catch (error) {
      if (showProgress) {
        setProgress((current) => current ? { ...current, status: 'error', message: errorMessage(error) } : null);
      }
      if (shouldLog) {
        reportError(error);
      } else {
        showError(error);
      }
    } finally {
      setBusy({ active: false, text: '' });
      if (showProgress) {
        window.setTimeout(() => {
          setProgress((current) => current?.status === 'running' ? current : null);
        }, 1600);
      }
    }
  }

  async function runStateAction(
    runningText: string,
    successText: string,
    action: () => Promise<AppState>,
    options: ActionOptions = {}
  ): Promise<void> {
    await runPlainAction(runningText, successText, async () => {
      sync(await call<AppState>('action'));
    }, options);
  }

  async function openLoginWindow(): Promise<void> {
    try {
      await call<void>('open_login_window');
    } catch (error) {
      showError(error);
    }
  }

  async function logout(): Promise<void> {
    try {
      await call<void>('clear_login');
      await call<void>('close_login_window');
      appendUiLog('已退出登录');
      Message.success('已退出登录');
      void refreshSession(false);
    } catch (error) {
      reportError(error);
    }
  }

  async function refreshSessionAfterLogin(): Promise<void> {
    try {
      const nextSession = await call<SessionStatus>('check_session');
      setState((previous) => previous ? { ...previous, session: nextSession } : previous);
      if (nextSession.state === 'ok') {
        await call<void>('close_login_window').catch(() => undefined);
        appendUiLog(`登录成功：${nextSession.displayName || nextSession.account || ''}`);
        Message.success('登录成功');
      }
    } catch {
      // ignore — user may still be on the login page
    }
  }

  async function refreshSession(showGlobalStatus = true): Promise<void> {
    try {
      const nextSession = await call<SessionStatus>('check_session');
      setState((previous) => previous ? { ...previous, session: nextSession } : previous);
      if (showGlobalStatus) {
        Message.success('会话刷新完成');
      }
    } catch (error) {
      if (showGlobalStatus) {
        showError(error);
      } else {
        setState((previous) => previous ? {
          ...previous,
          session: {
            state: 'missing',
            message: '未登录',
            browserName: '内置 WebView',
            account: '',
            displayName: '',
            checkedAt: ''
          }
        } : previous);
      }
    }
  }

  function openSettings(): void {
    setSettingsDraft({
      ...settings,
      themeMode: normalizeThemeMode(settings.themeMode)
    });
    setSettingsVisible(true);
  }

  function patchSettings<K extends keyof Settings>(key: K, value: Settings[K]): void {
    setSettingsDraft((current) => ({ ...current, [key]: value }));
  }

  async function chooseFileRoot(): Promise<void> {
    const selected = await call<string | null>('choose_file_root');
    if (selected) {
      patchSettings('lastFileRoot', selected);
    }
  }

  async function saveSettings(): Promise<void> {
    const payload: Settings = {
      lastFileRoot: settingsDraft.lastFileRoot.trim(),
      aiEnabled: settingsDraft.aiEnabled,
      aiBaseUrl: settingsDraft.aiBaseUrl.trim(),
      aiApiKey: settingsDraft.aiApiKey.trim(),
      aiModel: settingsDraft.aiModel.trim(),
      ocrBaseUrl: settingsDraft.ocrBaseUrl.trim(),
      ocrApiKey: settingsDraft.ocrApiKey.trim(),
      requestTimeoutSeconds: Number(settingsDraft.requestTimeoutSeconds || 30),
      imageMaxKb: Number(settingsDraft.imageMaxKb || 100),
      themeMode: normalizeThemeMode(settingsDraft.themeMode)
    };
    await runPlainAction('保存中', '设置已保存', async () => {
      sync(await call<AppState>('save_settings', { payload }));
      setSettingsVisible(false);
    }, { log: false });
  }

  async function runCompare(): Promise<void> {
    await runPlainAction('比对中', '比对完成', async () => {
      sync(await call<AppState>('run_compare_only', { fileRoot: currentFileRoot() }));
    }, { showProgress: true, resetLogs: true });
  }

  async function runBatch(): Promise<void> {
    await runPlainAction('批处理中', '批处理完成', async () => {
      sync(await call<AppState>('run_batch', { fileRoot: currentFileRoot() }));
    }, { showProgress: true, resetLogs: true });
  }

  async function runDownload(): Promise<void> {
    await runPlainAction('下载中', '下载完成', async () => {
      sync(await call<AppState>('run_download_only', { fileRoot: currentFileRoot() }));
    }, { showProgress: true, resetLogs: true });
  }

  async function openDownloadedDir(): Promise<void> {
    await runPlainAction('打开中', '目录已打开', () => call<boolean>('open_path', { path: currentDownloadRoot() }), { log: false });
  }

  async function checkForUpdate(): Promise<void> {
    try {
      Message.loading({ id: 'updater', content: '正在检查更新…' });
      const update = await checkUpdate();
      if (!update) {
        Message.success({ id: 'updater', content: '当前已是最新版本' });
        return;
      }
      Message.clear();
      Modal.confirm({
        title: '发现新版本',
        content: (
          <div>
            <div style={{ marginBottom: 8 }}>
              新版本：<b>v{update.version}</b>（当前 v{update.currentVersion}）
            </div>
            {update.body && (
              <pre style={{
                maxHeight: 240,
                overflow: 'auto',
                background: 'var(--color-fill-2)',
                padding: 8,
                borderRadius: 4,
                fontSize: 12,
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-word'
              }}>{update.body}</pre>
            )}
            <div style={{ color: 'var(--color-text-3)', fontSize: 12, marginTop: 8 }}>
              点击「立即更新」后将自动下载并重启应用。
            </div>
          </div>
        ),
        okText: '立即更新',
        cancelText: '稍后',
        onOk: async () => {
          try {
            let downloaded = 0;
            let contentLength = 0;
            await update.downloadAndInstall((event) => {
              if (event.event === 'Started') {
                contentLength = event.data.contentLength ?? 0;
                Message.loading({ id: 'updater', content: '开始下载更新…', duration: 0 });
              } else if (event.event === 'Progress') {
                downloaded += event.data.chunkLength;
                const percent = contentLength > 0
                  ? Math.floor((downloaded / contentLength) * 100)
                  : 0;
                Message.loading({ id: 'updater', content: `下载更新中 ${percent}%`, duration: 0 });
              } else if (event.event === 'Finished') {
                Message.success({ id: 'updater', content: '下载完成，即将重启…' });
              }
            });
            await relaunch();
          } catch (error) {
            Message.error({ id: 'updater', content: `更新失败：${(error as Error).message || error}` });
          }
        }
      });
    } catch (error) {
      Message.error({ id: 'updater', content: `检查更新失败：${(error as Error).message || error}` });
    }
  }

  async function clearRuntimeLogs(): Promise<void> {
    await runPlainAction('清空中', '日志已清空', async () => {
      sync(await call<AppState>('clear_runtime_logs', { fileRoot: currentFileRoot() }));
      setLogs([]);
    }, { log: false });
  }

  async function exportSuccess(): Promise<void> {
    await runPlainAction('导出中', '导出完成', async () => {
      const result = await call<SuccessExportResult>('export_success_results', { fileRoot: currentFileRoot() });
      if (result.status === 'missing_workbook') {
        throw new Error(`缺少成功台账: ${result.workbookPath}`);
      }
      await call<boolean>('open_path', { path: result.workbookPath });
      sync(await call<AppState>('bootstrap'));
    }, { log: false });
  }

  async function exportError(): Promise<void> {
    await runPlainAction('导出中', '导出完成', async () => {
      const path = await call<string>('export_error_results', { fileRoot: currentFileRoot() });
      await call<boolean>('open_path', { path });
    }, { log: false });
  }

  const isDirSelected = Boolean(currentFileRoot());

  return (
    <Layout className="app-shell">
      <Layout className="main-layout" hasSider>
        <Layout.Sider
          className="console-deck"
          width={300}
          collapsedWidth={0}
          breakpoint="lg"
          collapsed={siderCollapsed}
          onCollapse={(collapsed) => setSiderCollapsed(collapsed)}
          trigger={null}
        >
          <Button
            className="sider-toggle"
            size="mini"
            type="primary"
            icon={siderCollapsed ? <IconRight /> : <IconLeft />}
            onClick={() => setSiderCollapsed((value) => !value)}
            aria-label={siderCollapsed ? '展开侧栏' : '收起侧栏'}
          />

          {/* 整块控制台卡片 */}
          <Card className="console-card" bordered={false}>

            {/* 区段 1：会话状态 */}
            <div className="console-section">
              <div className="console-section-head">
                <span className="console-section-title">会话状态</span>
                <Space size={4}>
                  <Button
                    size="mini"
                    type="text"
                    icon={<IconRefresh />}
                    loading={session.state === 'checking'}
                    onClick={() => void refreshSession()}
                  >
                    刷新
                  </Button>
                  <Button
                    size="mini"
                    type="text"
                    disabled={session.state === 'checking'}
                    onClick={() => void openLoginWindow()}
                  >
                    {isSessionReady ? '重新登录' : '登录系统'}
                  </Button>
                  {isSessionReady && (
                    <Button
                      size="mini"
                      type="text"
                      status="danger"
                      onClick={() => void logout()}
                    >
                      退出
                    </Button>
                  )}
                </Space>
              </div>
              <Alert
                type={session.state === 'ok' ? 'success' : (session.state === 'checking' ? 'info' : 'warning')}
                showIcon
                title={session.state === 'ok' ? '和利时系统联通正常' : (session.state === 'checking' ? '检测中...' : '会话失效/未登录')}
                content={session.state === 'ok' ? `${session.displayName || session.account}已成功同步` : session.message || '点击右上「登录系统」按钮在内置窗口完成登录'}
              />
              {isSessionReady && (
                <Descriptions
                  border
                  size="mini"
                  column={1}
                  layout="horizontal"
                  style={{ marginTop: 8 }}
                  data={[
                    { label: '系统账号', value: session.account || '-' },
                    { label: '中文姓名', value: session.displayName || '-' },
                    { label: '会话来源', value: session.browserName || '-' },
                    ...(session.checkedAt ? [{ label: '检测时间', value: session.checkedAt }] : [])
                  ]}
                />
              )}
            </div>

            <Divider style={{ margin: '12px 0' }} />

            {/* 区段 2：项目工作目录 */}
            <div className="console-section">
              <div className="console-section-head">
                <span className="console-section-title">项目工作目录</span>
              </div>
              <div className="dir-path-text">
                {currentFileRoot() || '⚠️ 尚未选择工作路径，程序无法启动'}
              </div>
              <Space style={{ width: '100%', justifyContent: 'space-between', marginTop: 8 }}>
                <Button type="primary" size="small" onClick={openSettings}>
                  配置路径
                </Button>
                <Button
                  type="secondary"
                  size="small"
                  icon={<IconFolder />}
                  disabled={!isDirSelected}
                  onClick={() => void openDownloadedDir()}
                >
                  打开目录
                </Button>
              </Space>
            </div>

            <Divider style={{ margin: '12px 0' }} />

            {/* 区段 3：成果统计与导出 */}
            <div className="console-section">
              <div className="console-section-head">
                <span className="console-section-title">成果统计与数据导出</span>
              </div>
              <div className="stats-grid">
                <div className="stat-row">
                  <Statistic title="待导出成功项" value={outputs.pendingSuccessCount} precision={0} suffix="项" groupSeparator />
                  <Button
                    size="mini"
                    status="success"
                    type="primary"
                    icon={<IconExport />}
                    disabled={isBusy || outputs.pendingSuccessCount === 0}
                    onClick={() => void exportSuccess()}
                  >
                    导出台账
                  </Button>
                </div>
                <div className="stat-row">
                  <Statistic title="比对异常数" value={outputs.failedCount} precision={0} suffix="个" groupSeparator />
                  <Button
                    size="mini"
                    status="danger"
                    type="outline"
                    icon={<IconExport />}
                    disabled={isBusy || outputs.failedCount === 0}
                    onClick={() => void exportError()}
                  >
                    导出异常
                  </Button>
                </div>
                <div className="stat-row">
                  <Statistic title="已处理项目数" value={outputs.projectCount} precision={0} suffix="个" groupSeparator />
                  <IconInfoCircle style={{ color: 'var(--color-text-3)', fontSize: 16 }} />
                </div>
              </div>
            </div>

          </Card>

          <div className="console-footer">
            <span className="console-footer-version">{appVersion ? `v${appVersion}` : ''}</span>
            <Button
              className="console-footer-update"
              size="mini"
              type="text"
              icon={<IconSync />}
              onClick={() => void checkForUpdate()}
            >
              检查更新
            </Button>
          </div>

        </Layout.Sider>

        {/* 右侧主工作区 */}
        <Layout.Content className={`workspace-deck ${progress ? 'has-progress' : 'no-progress'}`}>

          {/* 整块工作区卡片 */}
          <Card className="workspace-card" bordered={false}>

            {/* 区段 1：操作区 */}
            {!isDirSelected ? (
              <div className="action-content">
                <div className="action-head">
                  <span />
                  <Button
                    icon={<IconSettings />}
                    type="text"
                    size="small"
                    disabled={isBusy}
                    title="全局设置"
                    onClick={openSettings}
                  />
                </div>
                <div className="empty-guide">
                  <Empty description={
                    <div>
                      <div style={{ fontSize: 15, fontWeight: 600, color: 'var(--color-text-1)', marginBottom: 8 }}>
                        请先完成工作目录配置
                      </div>
                      <div style={{ color: 'var(--color-text-3)', fontSize: 12 }}>
                        请在左侧点击“配置路径”指定保存项目关闭资料的数据根目录。
                      </div>
                    </div>
                  } />
                </div>
              </div>
            ) : (
              <div className="action-content">
                <div className="action-head">
                  <div className="action-hint">
                    <IconInfoCircle />
                    <span>批处理将循环拉取未下载的项目，并在下载完成后立即启动数据内容校验</span>
                  </div>
                  <Button
                    icon={<IconSettings />}
                    type="text"
                    size="small"
                    disabled={isBusy}
                    title="全局设置"
                    onClick={openSettings}
                  />
                </div>
                <Space size="medium" wrap>
                  <Button
                    type="primary"
                    icon={<IconSync />}
                    disabled={isBusy || !isSessionReady}
                    loading={busy.text === '批处理中'}
                    onClick={() => void runBatch()}
                  >
                    执行批处理
                  </Button>
                  <Button
                    type="outline"
                    icon={<IconDownload />}
                    disabled={isBusy || !isSessionReady}
                    loading={busy.text === '下载中'}
                    onClick={() => void runDownload()}
                  >
                    仅下载附件
                  </Button>
                  <Button
                    type="secondary"
                    icon={<IconCheckCircle />}
                    disabled={isBusy}
                    loading={busy.text === '比对中'}
                    onClick={() => void runCompare()}
                  >
                    仅本地比对
                  </Button>
                </Space>
              </div>
            )}

            {/* 区段 2：运行时进度 */}
            {progress && (
              <>
                <Divider style={{ margin: '12px 0' }} />
                <div className="progress-section">
                  <div className="progress-head">
                    <div className="progress-title">
                      <span>{progress.stage || busy.text || '执行中'}</span>
                      <Tag color={progress.status === 'error' ? 'red' : progress.status === 'done' ? 'green' : 'arcoblue'} style={{ marginLeft: 8 }}>
                        {progress.total > 0 ? `${progress.current}/${progress.total}` : '处理中'}
                      </Tag>
                    </div>
                    <span className="progress-message">{progress.message}</span>
                  </div>
                  <Progress
                    percent={progress.percent}
                    status={progress.status === 'error' ? 'error' : progress.status === 'done' ? 'success' : 'normal'}
                    size="small"
                    strokeWidth={6}
                  />
                </div>
              </>
            )}

            <Divider style={{ margin: '12px 0' }} />

            {/* 区段 3：日志与比对详情 */}
            <div className="log-explorer-section">
              <div className="console-section-head">
                <span className="console-section-title">比对详情及运行日志</span>
                <Button
                  size="mini"
                  status="danger"
                  type="text"
                  icon={<IconDelete />}
                  disabled={isBusy || !logs.length}
                  onClick={() => void clearRuntimeLogs()}
                >
                  清空日志
                </Button>
              </div>

              {logs.length > 0 && (
                <div className="log-filter-bar">
                  <Radio.Group
                    type="button"
                    size="small"
                    value={logFilter}
                    onChange={(val) => setLogFilter(val as any)}
                  >
                    <Radio value="all">全部 ({statsCounts.all})</Radio>
                    <Radio value="success">比对成功 ({statsCounts.success})</Radio>
                    <Radio value="failed">比对失败 ({statsCounts.failed})</Radio>
                    <Radio value="system">系统日志 ({statsCounts.system})</Radio>
                  </Radio.Group>
                </div>
              )}

              <div className="log-body">
                {filteredLogs.length ? (
                  <div className="runtime-log-list">
                    {filteredLogs.map((item, index) => item.projectLog ? (
                      <CompareLogTable log={item.projectLog} key={`${item.projectLog.finishedAt}-${item.projectLog.projectCode}-${index}`} />
                    ) : (
                      <pre className="plain-log-text" key={`${item.line}-${index}`}>{item.line}</pre>
                    ))}
                  </div>
                ) : (
                  <div className="log-empty">
                    <Empty description="暂无符合条件的日志或比对报告" />
                  </div>
                )}
                <div ref={logEndRef} />
              </div>
            </div>

          </Card>

        </Layout.Content>

      </Layout>

      {/* 设置对话框 */}
      <Modal
        className="settings-modal"
        title="配置中心"
        visible={settingsVisible}
        maskClosable={false}
        style={{ width: 760 }}
        onCancel={() => setSettingsVisible(false)}
        footer={(
          <Space>
            <Button disabled={isSaving} onClick={() => setSettingsVisible(false)}>取消</Button>
            <Button type="primary" loading={isSaving} onClick={() => void saveSettings()}>保存设置</Button>
          </Space>
        )}
      >
        <Tabs type="capsule" size="small" destroyOnHide={false} lazyload={false} className="settings-tabs">
          <TabPane key="basic" title="基础设置">
            <div className="settings-pane">
              <Form layout="horizontal" labelAlign="left" labelCol={{ span: 5 }} wrapperCol={{ span: 19 }} colon={false}>
                <Form.Item label="工作目录">
                  <Input
                    value={settingsDraft.lastFileRoot}
                    onChange={(value) => patchSettings('lastFileRoot', value)}
                    addAfter={<Button size="small" type="primary" onClick={() => void chooseFileRoot()}>选择</Button>}
                  />
                </Form.Item>
                <Form.Item label="背景模式">
                  <Select value={settingsDraft.themeMode} options={themeOptions} onChange={(value) => patchSettings('themeMode', String(value))} />
                </Form.Item>
              </Form>
            </div>
          </TabPane>
          <TabPane key="recognition" title="识别设置">
            <div className="settings-pane">
              <Alert
                type="info"
                style={{ marginBottom: 12 }}
                content="验收报告的手写签名、电话需要通过 AI 接口识别，请填写下方接口信息。"
              />
              <Form layout="horizontal" labelAlign="left" labelCol={{ span: 5 }} wrapperCol={{ span: 19 }} colon={false}>
                <Form.Item label="接口地址" extra="兼容 OpenAI 接口的服务地址">
                  <Input
                    value={settingsDraft.aiBaseUrl}
                    placeholder="例如 https://api.openai.com/v1"
                    onChange={(value) => patchSettings('aiBaseUrl', value)}
                  />
                </Form.Item>
                <Form.Item label="接口密钥">
                  <Input
                    type="password"
                    value={settingsDraft.aiApiKey}
                    placeholder="服务商提供的 API Key"
                    onChange={(value) => patchSettings('aiApiKey', value)}
                  />
                </Form.Item>
                <Form.Item label="模型名称" extra="需支持图片识别的多模态模型">
                  <Input
                    value={settingsDraft.aiModel}
                    placeholder="例如 gpt-4o / qwen-vl-max"
                    onChange={(value) => patchSettings('aiModel', value)}
                  />
                </Form.Item>
                <Collapse bordered={false} style={{ background: 'transparent' }}>
                  <Collapse.Item
                    name="advanced"
                    header="高级设置（一般无需修改）"
                  >
                    <Form.Item label="独立 OCR 地址" extra="留空则使用上方接口地址">
                      <Input
                        value={settingsDraft.ocrBaseUrl}
                        placeholder="若 OCR 走另一家服务再填"
                        onChange={(value) => patchSettings('ocrBaseUrl', value)}
                      />
                    </Form.Item>
                    <Form.Item label="独立 OCR 密钥">
                      <Input
                        type="password"
                        value={settingsDraft.ocrApiKey}
                        placeholder="留空则复用上方密钥"
                        onChange={(value) => patchSettings('ocrApiKey', value)}
                      />
                    </Form.Item>
                    <Form.Item label="请求超时" extra="单位：秒，网络差时可调大">
                      <InputNumber
                        min={1}
                        max={300}
                        value={settingsDraft.requestTimeoutSeconds}
                        onChange={(value) => patchSettings('requestTimeoutSeconds', Number(value || 30))}
                      />
                    </Form.Item>
                    <Form.Item label="图片压缩上限" extra="单位：KB，越大越清晰但耗时更长">
                      <InputNumber
                        min={20}
                        max={1024}
                        value={settingsDraft.imageMaxKb}
                        onChange={(value) => patchSettings('imageMaxKb', Number(value || 100))}
                      />
                    </Form.Item>
                  </Collapse.Item>
                </Collapse>
              </Form>
            </div>
          </TabPane>
        </Tabs>
      </Modal>
    </Layout>
  );
}

// 严整高亮且克制的比对表格组件
function CompareLogTable({ log }: { log: ProjectCompareLog }) {
  const statusText = log.passed ? '比对成功' : '比对失败';

  // 1. 提取最后一行的结果行 (fileName === "比对结果")
  const resultRow = useMemo(() => {
    return log.rows.find(row => row.fileName === '比对结果');
  }, [log.rows]);

  // 2. 检测哪些列的值是 "❌"
  const conflicts = useMemo(() => {
    return {
      projectCode: resultRow?.projectCode === '❌',
      projectName: resultRow?.projectName === '❌',
      contactName: resultRow?.contactName === '❌',
      contactPhone: resultRow?.contactPhone === '❌',
      startTime: resultRow?.startTime === '❌',
      acceptanceTime: resultRow?.acceptanceTime === '❌',
      amount: resultRow?.amount === '❌',
      hasRedStamp: resultRow?.hasRedStamp === '❌'
    };
  }, [resultRow]);

  // 3. 决定单元格样式类，仅在冲突列着淡色，绝不炫技
  const getCellClass = (field: keyof typeof conflicts, isResultRow: boolean, val: string) => {
    const value = String(val || '').trim();
    if (isResultRow) {
      if (value === '❌') return 'conflict-cell';
      if (value === '✅') return 'valid-cell';
      return '';
    }
    return conflicts[field] ? 'conflict-cell' : '';
  };

  return (
    <section className={`compare-log-item ${log.passed ? 'success' : 'error'}`}>
      <div className="compare-log-head">
        <div className="compare-log-title">
          <span className="compare-log-name">{log.projectCode || log.projectName || '未识别项目'}</span>
          <Tag color={log.passed ? 'green' : 'red'} style={{ marginLeft: '8px' }}>{statusText}</Tag>
        </div>
        <span className="compare-log-time">{log.finishedAt}</span>
      </div>
      
      {/* 比对异常简述 */}
      {log.summary && (
        <div className="compare-log-summary">
          <IconFile />
          <span>{log.summary}</span>
        </div>
      )}

      <div className="compare-table-wrap">
        <table className="compare-log-table">
          <thead>
            <tr>
              <th>文档来源</th>
              <th>项目编号</th>
              <th>项目全称</th>
              <th>用户姓名</th>
              <th>联系电话</th>
              <th>开始时间</th>
              <th>验收时间</th>
              <th>合同金额</th>
              <th>是否有红章</th>
            </tr>
          </thead>
          <tbody>
            {log.rows.map((row, index) => {
              const isResultRow = row.fileName === '比对结果';
              return (
                <tr className={isResultRow ? 'result-row' : ''} key={`${row.fileName}-${index}`}>
                  <td><strong>{valueOrDash(row.fileName)}</strong></td>
                  <td className={getCellClass('projectCode', isResultRow, row.projectCode)}>{valueOrDash(row.projectCode)}</td>
                  <td className={getCellClass('projectName', isResultRow, row.projectName)}>{valueOrDash(row.projectName)}</td>
                  <td className={getCellClass('contactName', isResultRow, row.contactName)}>{valueOrDash(row.contactName)}</td>
                  <td className={getCellClass('contactPhone', isResultRow, row.contactPhone)}>{valueOrDash(row.contactPhone)}</td>
                  <td className={getCellClass('startTime', isResultRow, row.startTime)}>{valueOrDash(row.startTime)}</td>
                  <td className={getCellClass('acceptanceTime', isResultRow, row.acceptanceTime)}>{valueOrDash(row.acceptanceTime)}</td>
                  <td className={getCellClass('amount', isResultRow, row.amount || '')}>{valueOrDash(row.amount || '')}</td>
                  <td className={getCellClass('hasRedStamp', isResultRow, row.hasRedStamp || '')}>{valueOrDash(row.hasRedStamp || '')}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function defaultProgressFor(text: string): WorkflowProgress | null {
  const stageMap: Record<string, string> = {
    '比对中': '本地比对',
    '批处理中': '批处理',
    '下载中': '网页下载'
  };
  const stage = stageMap[text];
  if (!stage) {
    return null;
  }
  return {
    taskId: text,
    stage,
    status: 'running',
    current: 0,
    total: 0,
    percent: text === '比对中' || text === '批处理中' ? 0 : 20,
    message: text,
    projectName: '',
    projectLog: null
  };
}

function parseCompareLogLine(line: string): ProjectCompareLog | null {
  const marker = '[项目比对] ';
  const start = line.indexOf(marker);
  if (start < 0) {
    return null;
  }
  try {
    const parsed = JSON.parse(line.slice(start + marker.length)) as ProjectCompareLog;
    if (!Array.isArray(parsed.rows)) {
      return null;
    }
    return parsed;
  } catch {
    return null;
  }
}

function projectLogLine(log: ProjectCompareLog): string {
  return `[项目比对] ${JSON.stringify(log)}`;
}

function shouldShowRuntimeLog(line: string): boolean {
  return !line.includes('[缓存]');
}

function valueOrDash(value: string): string {
  const normalized = String(value || '').trim();
  return normalized || '-';
}

function normalizeThemeMode(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === 'system' || normalized === 'auto' || normalized === '跟随系统') return 'system';
  if (normalized === 'dark' || normalized === 'night' || normalized === '夜间') return 'dark';
  return 'light';
}

function applyTheme(mode: string): () => void {
  const normalized = normalizeThemeMode(mode);
  const media = window.matchMedia('(prefers-color-scheme: dark)');

  const update = () => {
    const isDark = normalized === 'dark' || (normalized === 'system' && media.matches);
    if (isDark) {
      document.documentElement.dataset.theme = 'dark';
      document.body.setAttribute('arco-theme', 'dark');
    } else {
      document.documentElement.dataset.theme = 'light';
      document.body.removeAttribute('arco-theme');
    }
  };

  update();
  if (normalized === 'system') {
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }
  return () => undefined;
}

