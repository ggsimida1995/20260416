const appBridge = {
  state: null,
  modalOpen: false,
  themeStorageKey: 'project-file-compare-theme',

  bindEvents() {
    document.getElementById('startButton').addEventListener('click', () => this.callApi('handle_start_stop'));
    document.getElementById('downloadButton').addEventListener('click', () => this.callApi('run_download_only'));
    document.getElementById('compareButton').addEventListener('click', () => this.callApi('run_compare_only'));
    document.getElementById('refreshSessionButton').addEventListener('click', () => this.callApi('refresh_session'));
    document.getElementById('clearLogsButton').addEventListener('click', () => this.callApi('clear_logs'));
    document.getElementById('settingsButton').addEventListener('click', () => this.openSettings());
    document.getElementById('themeToggleButton').addEventListener('click', () => this.toggleTheme());
    document.getElementById('closeSettingsButton').addEventListener('click', () => this.closeSettings());
    document.getElementById('cancelSettingsButton').addEventListener('click', () => this.closeSettings());
    document.getElementById('saveSettingsButton').addEventListener('click', () => this.saveSettings());
    document.getElementById('chooseDirButton').addEventListener('click', () => this.chooseFileRoot());
    document.getElementById('openDirButton').addEventListener('click', () => this.callApi('open_file_root'));
    document.getElementById('openDirInlineButton').addEventListener('click', () => this.callApi('open_file_root'));
    document.getElementById('openDownloadedDirButton').addEventListener('click', () => this.openOutputDir('', 'projectRoot'));
    document.getElementById('openSuccessWorkbookButton').addEventListener('click', () => this.openOutputFile('successLogPath'));
    document.getElementById('openSuccessWorkbookDirButton').addEventListener('click', () => this.openOutputFile('successWorkbookPath'));
    document.getElementById('clearSuccessLogButton').addEventListener('click', () => this.callApi('clear_success_log'));
    document.getElementById('openFirstErrorReportButton').addEventListener('click', () => this.openOutputFile('errorLogPath'));
    document.getElementById('openErrorDirButton').addEventListener('click', () => this.openOutputFile('errorLogPath'));
    document.getElementById('clearErrorLogButton').addEventListener('click', () => this.callApi('clear_error_log'));
    document.getElementById('settingsModal').addEventListener('click', (event) => {
      if (event.target.id === 'settingsModal') {
        this.closeSettings();
      }
    });
  },

  async init() {
    this.applySavedTheme();
    this.bindEvents();
    this.syncThemeToggle();
    const state = await window.pywebview.api.bootstrap();
    this.sync(state);
  },

  applySavedTheme() {
    const theme = this.getSavedTheme();
    if (theme) {
      document.documentElement.dataset.theme = theme;
      return;
    }
    document.documentElement.removeAttribute('data-theme');
  },

  getSavedTheme() {
    try {
      const theme = localStorage.getItem(this.themeStorageKey);
      return theme === 'light' || theme === 'dark' ? theme : '';
    } catch (error) {
      return '';
    }
  },

  getEffectiveTheme() {
    const explicitTheme = document.documentElement.dataset.theme;
    if (explicitTheme === 'light' || explicitTheme === 'dark') {
      return explicitTheme;
    }
    if (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) {
      return 'dark';
    }
    return 'light';
  },

  toggleTheme() {
    const nextTheme = this.getEffectiveTheme() === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = nextTheme;
    try {
      localStorage.setItem(this.themeStorageKey, nextTheme);
    } catch (error) {
      // Theme still switches for the current window even when storage is blocked.
    }
    this.syncThemeToggle();
  },

  syncThemeToggle() {
    const button = document.getElementById('themeToggleButton');
    const icon = document.getElementById('themeToggleIcon');
    if (!button || !icon) {
      return;
    }

    const isDark = this.getEffectiveTheme() === 'dark';
    const nextLabel = isDark ? '白天' : '夜间';
    const title = `切换到${nextLabel}模式`;
    button.title = title;
    button.setAttribute('aria-label', title);
    icon.innerHTML = isDark
      ? '<circle cx="12" cy="12" r="4"></circle><path d="M12 2v2"></path><path d="M12 20v2"></path><path d="m4.93 4.93 1.41 1.41"></path><path d="m17.66 17.66 1.41 1.41"></path><path d="M2 12h2"></path><path d="M20 12h2"></path><path d="m6.34 17.66-1.41 1.41"></path><path d="m19.07 4.93-1.41 1.41"></path>'
      : '<path d="M12 3a6 6 0 0 0 9 7.5A9 9 0 1 1 12 3z"></path>';
  },

  sync(state) {
    if (!state) {
      return;
    }

    this.state = state;
    document.title = state.windowTitle;
    document.getElementById('appTitle').innerText = state.header.title;
    document.getElementById('appSubtitle').innerText = state.header.subtitle;

    const statusBadge = document.getElementById('statusBadge');
    statusBadge.innerText = state.status.text;
    statusBadge.className = `status-pill ${state.status.tone}`;

    const logOutput = document.getElementById('logOutput');
    if (state.logs.length === 0) {
      logOutput.innerHTML = '<span class="log-placeholder">运行日志会显示在这里。</span>';
    } else {
      logOutput.textContent = state.logs.join('\n');
      logOutput.scrollTop = logOutput.scrollHeight;
    }

    this.syncOutputs(state.outputs || {});

    const running = state.running;
    const busyState = state.busy || {
      active: Boolean(state.startupLoading),
      kind: state.startupLoading ? 'startup' : '',
      title: '',
      detail: ''
    };
    const busy = Boolean(busyState.active);
    const startupLoading = busyState.kind === 'startup' || Boolean(state.startupLoading);
    document.getElementById('busyStrip').classList.toggle('hidden', !busy);
    document.getElementById('busyTitle').innerText = busyState.title || '正在处理中';
    document.getElementById('busyDetail').innerText = busyState.detail || '请稍候';
    document.getElementById('startButtonLabel').innerText = startupLoading
      ? '启动检测中'
      : running
        ? '请求停止'
        : '开始批处理';
    document.getElementById('startButtonDetail').innerText = startupLoading
      ? '页面已打开，后台加载中'
      : running
        ? '当前步骤结束后停止'
        : '下载 / 比对 / 清理';
    document.getElementById('startButton').disabled = busy && !running;
    document.getElementById('downloadButton').disabled = busy;
    document.getElementById('compareButton').disabled = busy;
    document.getElementById('settingsButton').disabled = busy;
    document.getElementById('refreshSessionButton').disabled = busy;
    document.getElementById('saveSettingsButton').disabled = busy;
    document.getElementById('chooseDirButton').disabled = busy;
    document.getElementById('openDirButton').disabled = busy;
    document.getElementById('openDirInlineButton').disabled = busy;

    if (this.modalOpen) {
      this.fillSettingsForm(state.settings);
    }
  },

  syncOutputs(outputs) {
    const successLogPath = outputs.successLogPath || '';
    const errorLogPath = outputs.errorLogPath || '';
    const downloadedProjectNames = Array.isArray(outputs.downloadedProjectNames) ? outputs.downloadedProjectNames : [];
    const successProjectCodes = Array.isArray(outputs.successProjectCodes) ? outputs.successProjectCodes : [];
    const errorProjectCodes = Array.isArray(outputs.errorProjectCodes) ? outputs.errorProjectCodes : [];
    const errorReportPaths = Array.isArray(outputs.errorReportPaths) ? outputs.errorReportPaths : [];
    const updatedAt = outputs.updatedAt || '';
    const mode = outputs.mode || '';
    const successCount = Number(outputs.successCount || 0);
    const duplicateCount = Number(outputs.duplicateCount || 0);
    const failedCount = Number(outputs.failedCount || 0);
    const projectCount = Number(outputs.projectCount || 0);

    this.setBadge('projectCountBadge', `${projectCount} 个`, projectCount > 0 ? 'running' : 'idle');

    const outputMeta = document.getElementById('outputMeta');
    if (!mode) {
      outputMeta.innerText = '尚未生成比对结果';
    } else {
      const modeText = mode === 'batch' ? '最近一次批处理' : mode === 'compare' ? '最近一次本地比对' : '最近一次运行';
      const parts = [modeText];
      if (updatedAt) {
        parts.push(updatedAt);
      }
      outputMeta.innerText = parts.join(' | ');
    }

    const downloadedProjectList = document.getElementById('downloadedProjectList');
    if (downloadedProjectNames.length === 0) {
      downloadedProjectList.classList.add('empty');
      downloadedProjectList.textContent = '暂无已下载项目';
    } else {
      downloadedProjectList.classList.remove('empty');
      downloadedProjectList.innerHTML = downloadedProjectNames.map((name, index) => {
        const escaped = this.escapeHtml(name);
        return `<div class="output-item"><span class="output-item-index">${index + 1}.</span><span>${escaped}</span></div>`;
      }).join('');
    }

    const successProjectCodeList = document.getElementById('successProjectCodeList');
    if (successProjectCodes.length === 0) {
      successProjectCodeList.classList.add('empty');
      successProjectCodeList.textContent = successCount > 0 ? '本次已写入成功台账，但未拿到项目编号' : '暂无比对成功编号';
    } else {
      successProjectCodeList.classList.remove('empty');
      successProjectCodeList.innerHTML = successProjectCodes.map((code, index) => {
        const escaped = this.escapeHtml(code);
        return `<div class="output-item"><span class="output-item-index">${index + 1}.</span><span>${escaped}</span></div>`;
      }).join('');
    }
    this.setBadge('successWorkbookBadge', successCount > 0 ? `已追加 ${successCount}` : '未追加', successCount > 0 ? 'success' : 'idle');

    const errorProjectCodeList = document.getElementById('errorProjectCodeList');
    if (errorProjectCodes.length === 0) {
      errorProjectCodeList.classList.add('empty');
      errorProjectCodeList.textContent = duplicateCount > 0 || failedCount > 0 ? '本次应有失败项目，但未拿到项目编号' : '暂无比对失败编号';
    } else {
      errorProjectCodeList.classList.remove('empty');
      errorProjectCodeList.innerHTML = errorProjectCodes.map((code, index) => {
        const escaped = this.escapeHtml(code);
        return `<div class="output-item"><span class="output-item-index">${index + 1}.</span><span>${escaped}</span></div>`;
      }).join('');
    }
    const errorCount = errorProjectCodes.length || errorReportPaths.length || failedCount || duplicateCount;
    this.setBadge('errorReportBadge', `${errorCount} 个`, errorCount > 0 ? 'warning' : 'idle');

    document.getElementById('openSuccessWorkbookButton').disabled = !successLogPath;
    document.getElementById('openSuccessWorkbookDirButton').disabled = !outputs.successWorkbookExists;
    document.getElementById('clearSuccessLogButton').disabled = !successLogPath;
    document.getElementById('openFirstErrorReportButton').disabled = !errorLogPath;
    document.getElementById('openErrorDirButton').disabled = !errorLogPath;
    document.getElementById('clearErrorLogButton').disabled = !errorLogPath;
    document.getElementById('openDownloadedDirButton').disabled = !outputs.projectRoot;
  },

  setBadge(id, text, tone) {
    const el = document.getElementById(id);
    el.innerText = text;
    el.className = `status-pill ${tone}`;
  },

  async openOutputFile(key) {
    const path = this.state?.outputs?.[key] || '';
    await this.openOutputPathValue(path);
  },

  async openOutputDir(pathKey, fallbackKey) {
    const outputs = this.state?.outputs || {};
    const filePath = pathKey ? outputs[pathKey] || '' : '';
    const fallbackPath = outputs[fallbackKey] || '';
    if (filePath) {
      await window.pywebview.api.open_parent_path(filePath);
      return;
    }
    if (!fallbackPath) {
      return;
    }
    await window.pywebview.api.open_path(fallbackPath);
  },

  async openOutputPathValue(path) {
    if (!path) {
      return;
    }
    await window.pywebview.api.open_path(path);
  },

  escapeHtml(value) {
    return String(value)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  },

  async callApi(method, ...args) {
    try {
      const result = await window.pywebview.api[method](...args);
      if (result && result.state) {
        this.sync(result.state);
        return result;
      }
      this.sync(result);
      return result;
    } catch (error) {
      console.error(error);
    }
  },

  openSettings() {
    this.modalOpen = true;
    document.getElementById('settingsModal').classList.add('open');
    if (this.state) {
      this.fillSettingsForm(this.state.settings);
    }
  },

  closeSettings() {
    this.modalOpen = false;
    document.getElementById('settingsModal').classList.remove('open');
  },

  fillSettingsForm(settings) {
    document.getElementById('fileRootInput').value = settings.lastFileRoot;
    document.getElementById('aiEnabledInput').checked = settings.aiEnabled;
    document.getElementById('aiBaseUrlInput').value = settings.aiBaseUrl;
    document.getElementById('aiApiKeyInput').value = settings.aiApiKey;
    document.getElementById('aiModelInput').value = settings.aiModel;
    document.getElementById('ocrBaseUrlInput').value = settings.ocrBaseUrl;
    document.getElementById('ocrApiKeyInput').value = settings.ocrApiKey;
    document.getElementById('requestTimeoutInput').value = settings.requestTimeoutSeconds;
    document.getElementById('imageMaxKbInput').value = settings.imageMaxKb;
  },

  async chooseFileRoot() {
    const result = await window.pywebview.api.choose_file_root();
    if (result && result.selected) {
      document.getElementById('fileRootInput').value = result.selected;
    }
  },

  async saveSettings() {
    const payload = {
      lastFileRoot: document.getElementById('fileRootInput').value.trim(),
      aiEnabled: document.getElementById('aiEnabledInput').checked,
      aiBaseUrl: document.getElementById('aiBaseUrlInput').value.trim(),
      aiApiKey: document.getElementById('aiApiKeyInput').value.trim(),
      aiModel: document.getElementById('aiModelInput').value.trim(),
      ocrBaseUrl: document.getElementById('ocrBaseUrlInput').value.trim(),
      ocrApiKey: document.getElementById('ocrApiKeyInput').value.trim(),
      requestTimeoutSeconds: document.getElementById('requestTimeoutInput').value,
      imageMaxKb: document.getElementById('imageMaxKbInput').value
    };

    const state = await window.pywebview.api.save_settings(payload);
    this.sync(state);
    this.closeSettings();
  }
};

window.appBridge = appBridge;
window.addEventListener('pywebviewready', () => appBridge.init());
