import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Button,
  Card,
  Divider,
  Empty,
  Layout,
  Progress,
  Radio,
  Space,
  Tag
} from '@arco-design/web-react';
import {
  IconCheckCircle,
  IconDelete,
  IconDownload,
  IconInfoCircle,
  IconSettings,
  IconSync
} from '@arco-design/web-react/icon';
import { CompareLogTable } from './CompareLogTable';
import { parseCompareLogLine, shouldShowRuntimeLog } from './logUtils';

type WorkflowProgress = {
  taskId: string;
  stage: string;
  status: string;
  current: number;
  total: number;
  percent: number;
  message: string;
  projectName: string;
};

type Props = {
  isDirSelected: boolean;
  isBusy: boolean;
  isSessionReady: boolean;
  busyText: string;
  logs: string[];
  progress: WorkflowProgress | null;
  onOpenSettings: () => void;
  onRunBatch: () => void;
  onRunDownload: () => void;
  onRunCompare: () => void;
  onCancelWorkflow: () => void;
  onClearLogs: () => void;
};

type LogFilter = 'all' | 'success' | 'failed' | 'system';

export function WorkspacePanel({
  isDirSelected,
  isBusy,
  isSessionReady,
  busyText,
  logs,
  progress,
  onOpenSettings,
  onRunBatch,
  onRunDownload,
  onRunCompare,
  onCancelWorkflow,
  onClearLogs
}: Props) {
  const [logFilter, setLogFilter] = useState<LogFilter>('all');
  const logEndRef = useRef<HTMLDivElement | null>(null);

  const allParsedLogs = useMemo(() => {
    return logs.filter(shouldShowRuntimeLog).map((line) => ({
      line,
      projectLog: parseCompareLogLine(line)
    }));
  }, [logs]);

  const statsCounts = useMemo(() => {
    let success = 0;
    let failed = 0;
    let system = 0;
    allParsedLogs.forEach(item => {
      if (item.projectLog) {
        if (item.projectLog.passed) success++;
        else failed++;
      } else {
        system++;
      }
    });
    return { all: allParsedLogs.length, success, failed, system };
  }, [allParsedLogs]);

  const filteredLogs = useMemo(() => {
    if (logFilter === 'all') return allParsedLogs;
    if (logFilter === 'success') return allParsedLogs.filter(item => item.projectLog?.passed === true);
    if (logFilter === 'failed') return allParsedLogs.filter(item => item.projectLog && item.projectLog.passed === false);
    return allParsedLogs.filter(item => !item.projectLog);
  }, [allParsedLogs, logFilter]);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ block: 'nearest' });
  }, [filteredLogs.length]);

  return (
    <Layout.Content className={`workspace-deck ${progress ? 'has-progress' : 'no-progress'}`}>
      <Card className="workspace-card" bordered={false}>
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
                onClick={onOpenSettings}
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
                onClick={onOpenSettings}
              />
            </div>
            <Space size="medium" wrap>
              <Button
                type="primary"
                icon={<IconSync />}
                disabled={isBusy || !isSessionReady}
                loading={busyText === '批处理中'}
                onClick={onRunBatch}
              >
                执行批处理
              </Button>
              <Button
                type="outline"
                icon={<IconDownload />}
                disabled={isBusy || !isSessionReady}
                loading={busyText === '下载中'}
                onClick={onRunDownload}
              >
                仅下载附件
              </Button>
              <Button
                type="secondary"
                icon={<IconCheckCircle />}
                disabled={isBusy}
                loading={busyText === '比对中'}
                onClick={onRunCompare}
              >
                仅本地比对
              </Button>
            </Space>
          </div>
        )}

        {progress && (
          <>
            <Divider style={{ margin: '12px 0' }} />
            <div className="progress-section">
              <div className="progress-head">
                <div className="progress-title">
                  <span>{progress.stage || busyText || '执行中'}</span>
                  <Tag color={progress.status === 'error' ? 'red' : progress.status === 'done' ? 'green' : 'arcoblue'} style={{ marginLeft: 8 }}>
                    {progress.total > 0 ? `${progress.current}/${progress.total}` : '处理中'}
                  </Tag>
                </div>
                <Space>
                  <span className="progress-message">{progress.message}</span>
                  {progress.status === 'running' && (
                    <Button size="mini" status="danger" onClick={onCancelWorkflow}>
                      取消
                    </Button>
                  )}
                </Space>
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

        <div className="log-explorer-section">
          <div className="console-section-head">
            <span className="console-section-title">比对详情及运行日志</span>
            <Button
              size="mini"
              status="danger"
              type="text"
              icon={<IconDelete />}
              disabled={isBusy || !logs.length}
              onClick={onClearLogs}
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
                onChange={(val) => setLogFilter(val as LogFilter)}
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
  );
}
