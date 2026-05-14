import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  Button,
  Card,
  Empty,
  Form,
  Input,
  InputNumber,
  Message,
  Modal,
  Progress,
  Select,
  Space,
  Switch,
  Tabs,
  Tag
} from '@arco-design/web-react';
import {
  IconCheckCircle,
  IconDelete,
  IconDownload,
  IconExport,
  IconFolder,
  IconRefresh,
  IconSettings,
  IconSync
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
  browserKind: string;
  browserUserDataDir: string;
  browserProfile: string;
  browserSafeStorageService: string;
  themeMode: string;
};

type Outputs = {
  mode: string;
  updatedAt: string;
  successProjectCodes: string[];
  errorProjectCodes: string[];
  successCount: number;
  pendingSuccessCount: number;
  failedCount: number;
  projectCount: number;
  downloadedProjectNames: string[];
};

type SessionStatus = {
  state: string;
  message: string;
  browserName: string;
  userDataDir: string;
  profile: string;
  cookieDb: string;
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
type ResultHistoryKind = 'success' | 'error';
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
  aiEnabled: false,
  aiBaseUrl: '',
  aiApiKey: '',
  aiModel: '',
  ocrBaseUrl: '',
  ocrApiKey: '',
  requestTimeoutSeconds: 30,
  imageMaxKb: 100,
  browserKind: 'chrome',
  browserUserDataDir: '',
  browserProfile: 'auto',
  browserSafeStorageService: 'Chrome Safe Storage',
  themeMode: 'light'
};

const emptyOutputs: Outputs = {
  mode: '',
  updatedAt: '',
  successProjectCodes: [],
  errorProjectCodes: [],
  successCount: 0,
  pendingSuccessCount: 0,
  failedCount: 0,
  projectCount: 0,
  downloadedProjectNames: []
};

const emptySession: SessionStatus = {
  state: 'unknown',
  message: '',
  browserName: '浏览器',
  userDataDir: '',
  profile: 'auto',
  cookieDb: '',
  account: '',
  displayName: '',
  checkedAt: ''
};

const browserOptions = [
  { label: 'Google Chrome', value: 'chrome' },
  { label: 'Microsoft Edge', value: 'edge' },
  { label: 'Chromium', value: 'chromium' },
  { label: '自定义 Chromium', value: 'custom' }
];

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
  const [settingsDraft, setSettingsDraft] = useState<Settings>(emptySettings);
  const [progress, setProgress] = useState<WorkflowProgress | null>(null);
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

  const renderedLogs = useMemo(() => logs.filter(shouldShowRuntimeLog).map((line) => ({
    line,
    projectLog: parseCompareLogLine(line)
  })), [logs]);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ block: 'nearest' });
  }, [logs.length]);

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
      sync(await action());
    }, options);
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
            browserName: previous.settings.browserKind || '浏览器',
            userDataDir: previous.settings.browserUserDataDir || '',
            profile: previous.settings.browserProfile || 'auto',
            cookieDb: '',
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
      browserKind: normalizeBrowserKind(settings.browserKind),
      browserProfile: settings.browserProfile || 'auto',
      browserSafeStorageService: settings.browserSafeStorageService || defaultSafeStorageService(settings.browserKind),
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

  async function chooseBrowserUserDataDir(): Promise<void> {
    const selected = await call<string | null>('choose_browser_user_data_dir');
    if (selected) {
      patchSettings('browserUserDataDir', selected);
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
      browserKind: normalizeBrowserKind(settingsDraft.browserKind),
      browserUserDataDir: settingsDraft.browserUserDataDir.trim(),
      browserProfile: settingsDraft.browserProfile.trim() || 'auto',
      browserSafeStorageService: settingsDraft.browserSafeStorageService.trim(),
      themeMode: normalizeThemeMode(settingsDraft.themeMode)
    };
    await runPlainAction('保存中', '设置已保存', async () => {
      sync(await call<AppState>('save_settings', { payload }));
      setSettingsVisible(false);
    }, { log: false });
  }

  async function runCompare(): Promise<void> {
    await runStateAction('比对中', '比对完成', () => call<AppState>('run_compare_only', { fileRoot: currentFileRoot() }), { showProgress: true, resetLogs: true });
  }

  async function runBatch(): Promise<void> {
    await runStateAction('批处理中', '批处理完成', () => call<AppState>('run_batch', { fileRoot: currentFileRoot() }), { showProgress: true, resetLogs: true });
  }

  async function runDownload(): Promise<void> {
    await runStateAction('下载中', '下载完成', () => call<AppState>('run_download_only', { fileRoot: currentFileRoot() }), { showProgress: true, resetLogs: true });
  }

  async function openDownloadedDir(): Promise<void> {
    await runPlainAction('打开中', '目录已打开', () => call<boolean>('open_path', { path: currentFileRoot() }), { log: false });
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

  async function exportHistory(kind: ResultHistoryKind): Promise<void> {
    await runPlainAction('导出中', '导出完成', async () => {
      const path = await call<string>('export_result_history', { fileRoot: currentFileRoot(), kind });
      await call<boolean>('open_path', { path });
    }, { log: false });
  }

  const identity = isSessionReady ? (session.displayName || session.account) : '';
  const sessionTitle = identity ? `${identity}，欢迎您使用` : sessionFallbackTitle(session.state);
  const profile = session.profile && session.profile !== 'auto' ? ` | ${session.profile}` : '';
  const sessionDetail = [session.browserName ? `${session.browserName}${profile}` : '浏览器', session.checkedAt].filter(Boolean).join(' | ');
  const sessionClass = `session-button ${sessionTone(session.state)}`;

  return (
    <div className={`app-shell ${progress ? 'has-progress' : 'no-progress'}`}>
      <Card className="topbar-card" bordered bodyStyle={{ padding: 5 }}>
        <button className={sessionClass} type="button" disabled={isBusy} onClick={() => void refreshSession()}>
          <span className="session-lines">
            <span className={identity ? 'session-main known' : 'session-main pending'}>{sessionTitle}</span>
            <span className="session-detail">{sessionDetail || '未读取浏览器 Cookie'}</span>
          </span>
          <IconRefresh className="session-refresh-icon" />
        </button>
        <Space size={8} wrap className="toolbar">
          <Button type="primary" icon={<IconSync />} disabled={isBusy || !isSessionReady} loading={busy.text === '批处理中'} onClick={() => void runBatch()}>
            开始批处理
          </Button>
          <Button icon={<IconDownload />} disabled={isBusy || !isSessionReady} loading={busy.text === '下载中'} onClick={() => void runDownload()}>
            仅下载资料
          </Button>
          <Button type="primary" icon={<IconCheckCircle />} disabled={isBusy} loading={busy.text === '比对中'} onClick={() => void runCompare()}>
            仅本地比对
          </Button>
          <Button icon={<IconSettings />} disabled={isBusy} onClick={openSettings}>
            设置
          </Button>
        </Space>
      </Card>

      {progress && (
        <Card className="progress-card" bordered bodyStyle={{ padding: 10 }}>
          <div className="progress-head">
            <div className="progress-title">
              <span>{progress.stage || busy.text || '处理中'}</span>
              <Tag color={progress.status === 'error' ? 'red' : progress.status === 'done' ? 'green' : 'arcoblue'}>
                {progress.total > 0 ? `${progress.current}/${progress.total}` : '处理中'}
              </Tag>
            </div>
            <span className="progress-message">{progress.message}</span>
          </div>
          <Progress
            percent={progress.percent}
            status={progress.status === 'error' ? 'error' : progress.status === 'done' ? 'success' : 'normal'}
            size="small"
            strokeWidth={10}
            animation={progress.status === 'running'}
          />
        </Card>
      )}

      <Card
        className="log-card"
        bordered
        title={<span className="card-title">运行日志</span>}
        extra={(
          <Space size={8} wrap>
            <Button size="mini" type="outline" icon={<IconFolder />} disabled={isBusy} onClick={() => void openDownloadedDir()}>
              打开目录
            </Button>
            <Button size="mini" status="success" icon={<IconExport />} disabled={isBusy || outputs.pendingSuccessCount === 0} onClick={() => void exportSuccess()}>
              导出结果
            </Button>
            <Button size="mini" status="warning" icon={<IconExport />} disabled={isBusy || outputs.failedCount === 0} onClick={() => void exportError()}>
              导出异常
            </Button>
            <Button size="mini" status="success" icon={<IconExport />} disabled={isBusy || outputs.successCount === 0} onClick={() => void exportHistory('success')}>
              成功历史
            </Button>
            <Button size="mini" status="danger" icon={<IconExport />} disabled={isBusy || outputs.failedCount === 0} onClick={() => void exportHistory('error')}>
              失败历史
            </Button>
            <Button size="mini" status="danger" icon={<IconDelete />} disabled={isBusy || !logs.length} onClick={() => void clearRuntimeLogs()}>
              清空日志
            </Button>
          </Space>
        )}
      >
        <div className={`log-body ${renderedLogs.length ? '' : 'empty'}`}>
          {renderedLogs.length ? (
            <div className="runtime-log-list">
              {renderedLogs.map((item, index) => item.projectLog ? (
                <CompareLogTable log={item.projectLog} key={`${item.projectLog.finishedAt}-${item.projectLog.projectCode}-${index}`} />
              ) : (
                <pre className="plain-log-text" key={`${item.line}-${index}`}>{item.line}</pre>
              ))}
            </div>
          ) : (
            <div className="log-empty">
              <Empty description="暂无运行日志" />
            </div>
          )}
          <div ref={logEndRef} />
        </div>
      </Card>

      <Modal
        className="settings-modal"
        title="设置"
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
                <Form.Item label="保存目录">
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
          <TabPane key="browser" title="浏览器设置">
            <div className="settings-pane">
              <Form layout="horizontal" labelAlign="left" labelCol={{ span: 5 }} wrapperCol={{ span: 19 }} colon={false}>
                <Form.Item label="浏览器">
                  <Select
                    value={settingsDraft.browserKind}
                    options={browserOptions}
                    onChange={(value) => {
                      const browserKind = normalizeBrowserKind(String(value));
                      setSettingsDraft((current) => ({
                        ...current,
                        browserKind,
                        browserSafeStorageService: current.browserSafeStorageService || defaultSafeStorageService(browserKind)
                      }));
                    }}
                  />
                </Form.Item>
                <Form.Item label="数据目录">
                  <Input
                    value={settingsDraft.browserUserDataDir}
                    placeholder="留空使用默认目录"
                    onChange={(value) => patchSettings('browserUserDataDir', value)}
                    addAfter={<Button size="small" type="primary" onClick={() => void chooseBrowserUserDataDir()}>选择</Button>}
                  />
                </Form.Item>
                <Form.Item label="Profile">
                  <Input
                    value={settingsDraft.browserProfile}
                    placeholder="auto / Default / Profile 1"
                    onChange={(value) => patchSettings('browserProfile', value)}
                  />
                </Form.Item>
                <Form.Item label="钥匙串服务">
                  <Input
                    value={settingsDraft.browserSafeStorageService}
                    placeholder="Chrome Safe Storage"
                    onChange={(value) => patchSettings('browserSafeStorageService', value)}
                  />
                </Form.Item>
              </Form>
            </div>
          </TabPane>
          <TabPane key="recognition" title="识别设置">
            <div className="settings-pane">
              <Form layout="horizontal" labelAlign="left" labelCol={{ span: 5 }} wrapperCol={{ span: 19 }} colon={false}>
                <Form.Item label="在线识别">
                  <Switch checked={settingsDraft.aiEnabled} onChange={(checked) => patchSettings('aiEnabled', checked)} checkedText="启用" uncheckedText="停用" />
                </Form.Item>
                <Form.Item label="AI 接口">
                  <Input value={settingsDraft.aiBaseUrl} onChange={(value) => patchSettings('aiBaseUrl', value)} />
                </Form.Item>
                <Form.Item label="AI Key">
                  <Input type="password" value={settingsDraft.aiApiKey} onChange={(value) => patchSettings('aiApiKey', value)} />
                </Form.Item>
                <Form.Item label="AI 模型">
                  <Input value={settingsDraft.aiModel} onChange={(value) => patchSettings('aiModel', value)} />
                </Form.Item>
                <Form.Item label="OCR 接口">
                  <Input value={settingsDraft.ocrBaseUrl} onChange={(value) => patchSettings('ocrBaseUrl', value)} />
                </Form.Item>
                <Form.Item label="OCR Key">
                  <Input type="password" value={settingsDraft.ocrApiKey} onChange={(value) => patchSettings('ocrApiKey', value)} />
                </Form.Item>
                <Form.Item label="超时(秒)">
                  <InputNumber min={1} max={300} value={settingsDraft.requestTimeoutSeconds} onChange={(value) => patchSettings('requestTimeoutSeconds', Number(value || 30))} />
                </Form.Item>
                <Form.Item label="图片上限(KB)">
                  <InputNumber min={20} max={1024} value={settingsDraft.imageMaxKb} onChange={(value) => patchSettings('imageMaxKb', Number(value || 100))} />
                </Form.Item>
              </Form>
            </div>
          </TabPane>
        </Tabs>
      </Modal>
    </div>
  );
}

function CompareLogTable({ log }: { log: ProjectCompareLog }) {
  const statusText = log.passed ? '比对成功' : '比对失败';
  return (
    <section className={`compare-log-item ${log.passed ? 'success' : 'error'}`}>
      <div className="compare-log-head">
        <div className="compare-log-title">
          <span className="compare-log-name">{log.projectCode || log.projectName || '未识别项目'}</span>
          <Tag color={log.passed ? 'green' : 'red'}>{statusText}</Tag>
        </div>
        <span className="compare-log-time">{log.finishedAt}</span>
      </div>
      <div className="compare-table-wrap">
        <table className="compare-log-table">
          <thead>
            <tr>
              <th>文件名称</th>
              <th>项目编号</th>
              <th>项目全称</th>
              <th>用户姓名</th>
              <th>联系电话</th>
              <th>验收时间</th>
              <th>开始时间</th>
              <th>金额</th>
              <th>是否有红章</th>
            </tr>
          </thead>
          <tbody>
            {log.rows.map((row, index) => (
              <tr className={row.fileName === '比对结果' ? 'result-row' : ''} key={`${row.fileName}-${index}`}>
                <td>{valueOrDash(row.fileName)}</td>
                <td>{valueOrDash(row.projectCode)}</td>
                <td>{valueOrDash(row.projectName)}</td>
                <td>{valueOrDash(row.contactName)}</td>
                <td>{valueOrDash(row.contactPhone)}</td>
                <td>{valueOrDash(row.acceptanceTime)}</td>
                <td>{valueOrDash(row.startTime)}</td>
                <td>{valueOrDash(row.amount || '')}</td>
                <td>{valueOrDash(row.hasRedStamp || '')}</td>
              </tr>
            ))}
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
  return normalized;
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

function normalizeBrowserKind(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === 'edge' || normalized === 'microsoft_edge' || normalized === 'microsoft-edge') return 'edge';
  if (normalized === 'chromium') return 'chromium';
  if (normalized === 'custom' || normalized === 'custom_chromium' || normalized === 'custom-chromium') return 'custom';
  return 'chrome';
}

function defaultSafeStorageService(kind: string): string {
  const normalized = normalizeBrowserKind(kind);
  if (normalized === 'edge') return 'Microsoft Edge Safe Storage';
  if (normalized === 'chromium' || normalized === 'custom') return 'Chromium Safe Storage';
  return 'Chrome Safe Storage';
}

function sessionTone(stateName: string): string {
  if (stateName === 'ok') return 'success';
  if (stateName === 'checking') return 'idle';
  if (stateName === 'unknown') return 'idle';
  if (stateName === 'expired' || stateName === 'missing' || stateName === 'error') return 'warning';
  return 'idle';
}

function sessionFallbackTitle(stateName: string): string {
  if (stateName === 'checking') return '正在检测会话';
  if (stateName === 'unknown') return '点击刷新会话';
  return '未登录';
}
