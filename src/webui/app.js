const appBridge = {
  state: null,
  modalOpen: false,

  bindEvents() {
    document.getElementById('startButton').addEventListener('click', () => this.callApi('handle_start_stop'));
    document.getElementById('downloadButton').addEventListener('click', () => this.callApi('run_download_only'));
    document.getElementById('compareButton').addEventListener('click', () => this.callApi('run_compare_only'));
    document.getElementById('refreshSessionButton').addEventListener('click', () => this.callApi('refresh_session'));
    document.getElementById('clearLogsButton').addEventListener('click', () => this.callApi('clear_logs'));
    document.getElementById('settingsButton').addEventListener('click', () => this.openSettings());
    document.getElementById('closeSettingsButton').addEventListener('click', () => this.closeSettings());
    document.getElementById('cancelSettingsButton').addEventListener('click', () => this.closeSettings());
    document.getElementById('saveSettingsButton').addEventListener('click', () => this.saveSettings());
    document.getElementById('chooseDirButton').addEventListener('click', () => this.chooseFileRoot());
    document.getElementById('openDirButton').addEventListener('click', () => this.callApi('open_file_root'));
    document.getElementById('openDirInlineButton').addEventListener('click', () => this.callApi('open_file_root'));
    document.getElementById('settingsModal').addEventListener('click', (event) => {
      if (event.target.id === 'settingsModal') {
        this.closeSettings();
      }
    });
  },

  async init() {
    this.bindEvents();
    const state = await window.pywebview.api.bootstrap();
    this.sync(state);
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
