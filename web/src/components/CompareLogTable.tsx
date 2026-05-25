import { useMemo } from 'react';
import { Tag } from '@arco-design/web-react';
import { IconFile } from '@arco-design/web-react/icon';

export type ProjectCompareLogRow = {
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

export type ProjectCompareLog = {
  projectName: string;
  projectCode: string;
  passed: boolean;
  summary: string;
  finishedAt: string;
  rows: ProjectCompareLogRow[];
};

function valueOrDash(value: string): string {
  const normalized = String(value || '').trim();
  return normalized || '-';
}

export function CompareLogTable({ log }: { log: ProjectCompareLog }) {
  const statusText = log.passed ? '比对成功' : '比对失败';

  const resultRow = useMemo(() => {
    return log.rows.find(row => row.fileName === '比对结果');
  }, [log.rows]);

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
