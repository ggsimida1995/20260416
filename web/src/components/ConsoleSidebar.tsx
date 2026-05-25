import {
  Alert,
  Button,
  Card,
  Descriptions,
  Divider,
  Layout,
  Space,
  Spin,
  Statistic
} from '@arco-design/web-react';
import {
  IconExport,
  IconFolder,
  IconInfoCircle,
  IconLeft,
  IconRefresh,
  IconRight,
  IconSync
} from '@arco-design/web-react/icon';

type SessionStatus = {
  state: string;
  message: string;
  browserName: string;
  account: string;
  displayName: string;
  checkedAt: string;
};

type Outputs = {
  pendingSuccessCount: number;
  failedCount: number;
  projectCount: number;
};

type Props = {
  collapsed: boolean;
  onToggleCollapsed: () => void;
  onSetCollapsed: (next: boolean) => void;
  session: SessionStatus;
  isSessionReady: boolean;
  sessionRefreshing: boolean;
  onRefreshSession: () => void;
  onOpenLogin: () => void;
  onCloseLogin: () => void;
  onLogout: () => void;
  fileRoot: string;
  isDirSelected: boolean;
  isBusy: boolean;
  onOpenSettings: () => void;
  onOpenDownloadedDir: () => void;
  outputs: Outputs;
  onExportSuccess: () => void;
  onExportError: () => void;
  appVersion: string;
  updateChecking: boolean;
  onCheckUpdate: () => void;
};

export function ConsoleSidebar({
  collapsed,
  onToggleCollapsed,
  onSetCollapsed,
  session,
  isSessionReady,
  sessionRefreshing,
  onRefreshSession,
  onOpenLogin,
  onCloseLogin,
  onLogout,
  fileRoot,
  isDirSelected,
  isBusy,
  onOpenSettings,
  onOpenDownloadedDir,
  outputs,
  onExportSuccess,
  onExportError,
  appVersion,
  updateChecking,
  onCheckUpdate
}: Props) {
  return (
    <Layout.Sider
      className="console-deck"
      width={300}
      collapsedWidth={0}
      breakpoint="lg"
      collapsed={collapsed}
      onCollapse={onSetCollapsed}
      trigger={null}
    >
      <Button
        className="sider-toggle"
        size="mini"
        type="primary"
        icon={collapsed ? <IconRight /> : <IconLeft />}
        onClick={onToggleCollapsed}
        aria-label={collapsed ? '展开侧栏' : '收起侧栏'}
      />

      <Card className="console-card" bordered={false}>
        <div className="console-section">
          <div className="console-section-head">
            <span className="console-section-title">会话状态</span>
            <Space size={4}>
              <Button
                size="mini"
                type="text"
                icon={<IconRefresh />}
                loading={sessionRefreshing}
                onClick={onRefreshSession}
              >
                刷新
              </Button>
              <Button
                size="mini"
                type="text"
                disabled={sessionRefreshing || session.state === 'checking'}
                onClick={onOpenLogin}
              >
                {isSessionReady ? '重新登录' : '登录系统'}
              </Button>
              <Button
                size="mini"
                type="text"
                disabled={sessionRefreshing}
                onClick={onCloseLogin}
              >
                关窗
              </Button>
              {isSessionReady && (
                <Button
                  size="mini"
                  type="text"
                  status="danger"
                  disabled={sessionRefreshing}
                  onClick={onLogout}
                >
                  退出
                </Button>
              )}
            </Space>
          </div>
          <Spin loading={sessionRefreshing} block>
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
          </Spin>
        </div>

        <Divider style={{ margin: '12px 0' }} />

        <div className="console-section">
          <div className="console-section-head">
            <span className="console-section-title">项目工作目录</span>
          </div>
          <div className="dir-path-text">
            {fileRoot || '⚠️ 尚未选择工作路径，程序无法启动'}
          </div>
          <Space style={{ width: '100%', justifyContent: 'space-between', marginTop: 8 }}>
            <Button type="primary" size="small" onClick={onOpenSettings}>
              配置路径
            </Button>
            <Button
              type="secondary"
              size="small"
              icon={<IconFolder />}
              disabled={!isDirSelected}
              onClick={onOpenDownloadedDir}
            >
              打开目录
            </Button>
          </Space>
        </div>

        <Divider style={{ margin: '12px 0' }} />

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
                onClick={onExportSuccess}
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
                onClick={onExportError}
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
          loading={updateChecking}
          onClick={onCheckUpdate}
        >
          检查更新
        </Button>
      </div>
    </Layout.Sider>
  );
}
