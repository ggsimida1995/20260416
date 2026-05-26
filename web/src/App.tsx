import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import { check as checkUpdate } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { type ProjectCompareLog } from './components/CompareLogTable';
import { SettingsModal, type Settings } from './components/SettingsModal';
import { ConsoleSidebar } from './components/ConsoleSidebar';
import { WorkspacePanel } from './components/WorkspacePanel';
import { projectLogLine } from './components/logUtils';
import { Layout, Message, Modal } from '@arco-design/web-react';

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

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
  themeMode: 'light',
  account: '',
  password: ''
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
  state: 'checking',
  message: '',
  browserName: '内置 WebView',
  account: '',
  displayName: '',
  checkedAt: ''
};

const LOGIN_SESSION_POLL_INTERVAL_MS = 3000;
const LOGIN_SESSION_POLL_TIMEOUT_MS = 5 * 60 * 1000;

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && Boolean(window.__TAURI_INTERNALS__);
}

function isWindowsClient(): boolean {
  if (typeof navigator === 'undefined') {
    return false;
  }
  return /Windows/i.test(navigator.userAgent) || /^Win/i.test(navigator.platform);
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
  const [appVersion, setAppVersion] = useState<string>('');
  const [sessionRefreshing, setSessionRefreshing] = useState(false);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [workflowElapsedMs, setWorkflowElapsedMs] = useState(0);

  const settings = state?.settings ?? emptySettings;
  const outputs = state?.outputs ?? emptyOutputs;
  const session = state?.session ?? emptySession;
  const isBusy = busy.active;
  const isSaving = busy.active && busy.text === '保存中';
  const isSessionReady = session.state === 'ok';
  const showSessionPreview = !isWindowsClient();
  const sessionStateRef = useRef<string>(session.state);
  const loginPollTimerRef = useRef<number | null>(null);
  const loginPollRunningRef = useRef(false);
  const loginCompletedRef = useRef(false);
  const workflowTimerRef = useRef<number | null>(null);
  const workflowStartedAtRef = useRef<number | null>(null);
  sessionStateRef.current = session.state;

  useEffect(() => {
    void bootstrap();
  }, []);

  useEffect(() => {
    return () => stopLoginSessionPolling();
  }, []);

  useEffect(() => {
    return () => stopWorkflowTimer(false);
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

  useEffect(() => {
    if (!isTauriRuntime()) {
      return () => undefined;
    }
    let unlisten: UnlistenFn | null = null;
    let mounted = true;
    void listen('auth://login-page-detected', () => {
      if (!mounted || loginPollTimerRef.current !== null) {
        return;
      }
      setState((previous) => previous ? {
        ...previous,
        session: {
          state: 'missing',
          message: '已跳转到登录页，请重新登录',
          browserName: previous.session.browserName || '内置 WebView',
          account: '',
          displayName: '',
          checkedAt: new Date().toLocaleString('zh-CN', { hour12: false })
        }
      } : previous);
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
    if (!isTauriRuntime()) {
      return () => undefined;
    }
    const id = window.setInterval(async () => {
      if (sessionStateRef.current !== 'ok') return;
      try {
        const next = await call<SessionStatus>('check_session');
        setState((previous) => previous ? { ...previous, session: next } : previous);
      } catch {
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
    }, 10 * 60 * 1000);
    return () => window.clearInterval(id);
  }, []);

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
    if (showProgress) {
      startWorkflowTimer();
    }
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
      if (showProgress) {
        stopWorkflowTimer();
      }
      setBusy({ active: false, text: '' });
      if (showProgress) {
        window.setTimeout(() => {
          setProgress((current) => current?.status === 'running' ? current : null);
        }, 1600);
      }
    }
  }

  function startWorkflowTimer(): void {
    stopWorkflowTimer();
    const startedAt = Date.now();
    workflowStartedAtRef.current = startedAt;
    setWorkflowElapsedMs(0);
    workflowTimerRef.current = window.setInterval(() => {
      setWorkflowElapsedMs(Date.now() - startedAt);
    }, 1000);
  }

  function stopWorkflowTimer(updateElapsed = true): void {
    if (workflowTimerRef.current !== null) {
      window.clearInterval(workflowTimerRef.current);
      workflowTimerRef.current = null;
    }
    if (workflowStartedAtRef.current !== null) {
      if (updateElapsed) {
        setWorkflowElapsedMs(Date.now() - workflowStartedAtRef.current);
      }
      workflowStartedAtRef.current = null;
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
      startLoginSessionPolling();
      Message.info('登录窗口已打开，登录完成后会自动同步会话');
    } catch (error) {
      showError(error);
    }
  }

  async function openSessionPreviewWindow(): Promise<void> {
    try {
      await call<void>('open_session_preview_window');
      void refreshSession(false);
    } catch (error) {
      showError(error);
    }
  }

  function startLoginSessionPolling(): void {
    stopLoginSessionPolling();
    loginCompletedRef.current = false;
    const deadline = Date.now() + LOGIN_SESSION_POLL_TIMEOUT_MS;
    setSessionRefreshing(true);
    setState((previous) => previous ? {
      ...previous,
      session: {
        ...previous.session,
        state: 'checking',
        message: '等待网页登录完成'
      }
    } : previous);

    loginPollTimerRef.current = window.setInterval(() => {
      if (loginPollRunningRef.current) {
        return;
      }
      loginPollRunningRef.current = true;
      void pollLoginSession(deadline).finally(() => {
        loginPollRunningRef.current = false;
      });
    }, LOGIN_SESSION_POLL_INTERVAL_MS);
  }

  function stopLoginSessionPolling(): void {
    if (loginPollTimerRef.current !== null) {
      window.clearInterval(loginPollTimerRef.current);
      loginPollTimerRef.current = null;
    }
  }

  async function pollLoginSession(deadline: number): Promise<void> {
    try {
      const nextSession = await call<SessionStatus>('check_session');
      if (loginCompletedRef.current) {
        return;
      }
      if (nextSession.state === 'ok') {
        await completeLoginSession(nextSession);
        return;
      }
      if (Date.now() >= deadline) {
        stopLoginSessionPolling();
        setSessionRefreshing(false);
        setState((previous) => previous ? { ...previous, session: nextSession } : previous);
        Message.warning('暂未检测到网页登录成功，完成登录后可点击刷新会话');
      }
    } catch {
      if (Date.now() >= deadline) {
        stopLoginSessionPolling();
        setSessionRefreshing(false);
        Message.warning('暂未检测到网页登录成功，完成登录后可点击刷新会话');
      }
    }
  }

  async function completeLoginSession(nextSession: SessionStatus): Promise<void> {
    if (loginCompletedRef.current) {
      return;
    }
    loginCompletedRef.current = true;
    stopLoginSessionPolling();
    setSessionRefreshing(false);
    setState((previous) => previous ? { ...previous, session: nextSession } : previous);
    await call<void>('close_login_window').catch(() => undefined);
    appendUiLog(`登录成功：${nextSession.displayName || nextSession.account || ''}`);
    Message.success('登录成功');
  }

  async function logout(): Promise<void> {
    setSessionRefreshing(true);
    try {
      await call<void>('clear_login');
      await call<void>('close_login_window');
      appendUiLog('已退出登录');
      Message.success('已退出登录');
      await refreshSession(false);
    } catch (error) {
      reportError(error);
    } finally {
      setSessionRefreshing(false);
    }
  }

  async function refreshSessionAfterLogin(): Promise<void> {
    try {
      const nextSession = await call<SessionStatus>('check_session');
      if (nextSession.state !== 'ok') {
        if (loginPollTimerRef.current === null) {
          setState((previous) => previous ? { ...previous, session: nextSession } : previous);
        }
        return;
      }
      const wasAlreadyOk = sessionStateRef.current === 'ok';
      if (loginPollTimerRef.current !== null) {
        await completeLoginSession(nextSession);
        return;
      }
      setState((previous) => previous ? { ...previous, session: nextSession } : previous);
      if (wasAlreadyOk) {
        // SSO 重定向链会触发多次 cookies-updated 事件,第一次已经提示并关闭窗口,后续静默更新即可。
        return;
      }
      await call<void>('close_login_window').catch(() => undefined);
      appendUiLog(`登录成功：${nextSession.displayName || nextSession.account || ''}`);
      Message.success('登录成功');
    } catch {
      // ignore — user may still be on the login page
    }
  }

  async function refreshSession(showGlobalStatus = true): Promise<void> {
    setSessionRefreshing(true);
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
    } finally {
      setSessionRefreshing(false);
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
      themeMode: normalizeThemeMode(settingsDraft.themeMode),
      account: settingsDraft.account.trim(),
      password: settingsDraft.password
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
      const nextState = await call<AppState>('run_download_only', { fileRoot: currentFileRoot() });
      sync(nextState);
      const summary = latestDownloadSummary(nextState.logs);
      Message.success(summary || '下载完成');
    }, { showProgress: true, resetLogs: true, toast: false });
  }

  async function cancelWorkflow(): Promise<void> {
    try {
      await call<void>('cancel_workflow');
      setProgress((current) => current ? { ...current, message: '正在取消…' } : current);
    } catch (error) {
      showError(error);
    }
  }

  async function openDownloadedDir(): Promise<void> {
    await runPlainAction('打开中', '目录已打开', () => call<boolean>('open_path', { path: currentDownloadRoot() }), { log: false });
  }

  async function checkForUpdate(): Promise<void> {
    if (updateChecking) {
      return;
    }
    setUpdateChecking(true);
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
    } finally {
      setUpdateChecking(false);
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
        <ConsoleSidebar
          collapsed={siderCollapsed}
          onToggleCollapsed={() => setSiderCollapsed((value) => !value)}
          onSetCollapsed={setSiderCollapsed}
          session={session}
          isSessionReady={isSessionReady}
          sessionRefreshing={sessionRefreshing}
          onRefreshSession={() => void refreshSession()}
          showSessionPreview={showSessionPreview}
          onOpenSessionPreview={() => void openSessionPreviewWindow()}
          onOpenLogin={() => void openLoginWindow()}
          onLogout={() => void logout()}
          fileRoot={currentFileRoot()}
          isDirSelected={isDirSelected}
          isBusy={isBusy}
          onOpenSettings={openSettings}
          onOpenDownloadedDir={() => void openDownloadedDir()}
          outputs={outputs}
          onExportSuccess={() => void exportSuccess()}
          onExportError={() => void exportError()}
          appVersion={appVersion}
          updateChecking={updateChecking}
          onCheckUpdate={() => void checkForUpdate()}
        />

        <WorkspacePanel
          isDirSelected={isDirSelected}
          isBusy={isBusy}
          isSessionReady={isSessionReady}
          busyText={busy.text}
          logs={logs}
          progress={progress}
          workflowElapsedMs={workflowElapsedMs}
          onOpenSettings={openSettings}
          onRunBatch={() => void runBatch()}
          onRunDownload={() => void runDownload()}
          onRunCompare={() => void runCompare()}
          onCancelWorkflow={() => void cancelWorkflow()}
          onClearLogs={() => void clearRuntimeLogs()}
        />

      </Layout>

      <SettingsModal
        visible={settingsVisible}
        draft={settingsDraft}
        isSaving={busy.active && busy.text === '保存中'}
        onClose={() => setSettingsVisible(false)}
        onSave={() => void saveSettings()}
        onPatch={patchSettings}
        onChooseFolder={() => void chooseFileRoot()}
      />
    </Layout>
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
    message: '',
    projectName: '',
    projectLog: null
  };
}

function latestDownloadSummary(logs: string[]): string | null {
  for (let index = logs.length - 1; index >= 0; index--) {
    const match = logs[index].match(/\[网页阶段\] 完成: 下载=(\d+) \| 跳过=(\d+) \| 错误=(\d+)/);
    if (!match) {
      continue;
    }
    const [, downloaded, skipped, errors] = match;
    return `下载完成：下载 ${downloaded} 个，跳过 ${skipped} 个，错误 ${errors} 个`;
  }
  return null;
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
