import type { ProjectCompareLog } from './CompareLogTable';

export function parseCompareLogLine(line: string): ProjectCompareLog | null {
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

export function projectLogLine(log: ProjectCompareLog): string {
  return `[项目比对] ${JSON.stringify(log)}`;
}

export function shouldShowRuntimeLog(line: string): boolean {
  return !line.includes('[缓存]');
}
