(() => {
  const messages = {
    localNote: ['本地运行 · Cookie 不离开设备', 'Local app · cookies stay on this device'], languageLabel: ['语言', 'Language'], chinese: ['中文', 'Chinese'], english: ['English', 'English'], queueCount: ['同时处理数量', 'Number processed at once'], uploadQueue: ['补传队列', 'Batch upload queue'],
    eyebrow: ['PROJECT TRANSFER', 'PROJECT TRANSFER'], heroTitle: ['把项目搬到', 'Move your projects to'], heroCopy: ['在内置 OAuth 窗口完成两次登录，选择项目后逐个导出、检查并导入。不会批量压垮服务，也不会读取系统浏览器。', 'Sign in twice in the built-in OAuth windows, then export, check, and import projects one at a time. No bulk load on the service, and your system browser is never accessed.'], zipLimitSingle: ['单个 ZIP 上传上限', 'ZIP upload limit per file'],
    connectSystems: ['连接两个系统', 'Connect both systems'], addressPrefilled: ['地址可修改，默认值已预填', 'Addresses are editable; defaults are prefilled'], sourceLabel: ['老系统 · 导出来源', 'Legacy system · export source'], targetLabel: ['新系统 · 导入目标', 'New system · import target'], serviceAddress: ['服务地址', 'Service address'], openOAuth: ['打开 OAuth 登录', 'Open OAuth sign-in'], notConnected: ['尚未连接', 'Not connected'],
    selectProjects: ['选择要迁移的项目', 'Select projects to migrate'], loadProjects: ['读取项目列表', 'Load project list'], loadAfterSignIn: ['完成登录后读取项目', 'Load projects after signing in'], selectAll: ['全选', 'Select all'], selectNone: ['清空', 'Clear'], projectsNotLoaded: ['项目列表尚未加载', 'Project list not loaded'], projectsHint: ['老系统登录成功后，点击「读取项目列表」。', 'After signing in to the legacy system, click “Load project list.”'],
    zipWorkdir: ['ZIP 工作目录', 'ZIP work directory'], chooseDirectory: ['选择目录', 'Choose directory'], downloadCheckUpload: ['下载 → 检查大小 → 上传', 'Download → check size → upload'], oversizePending: ['超限 ZIP 跳过并保留在「待处理」', 'Oversized ZIPs are skipped and kept in “待处理”'], ready: ['准备就绪', 'Ready'], concurrency: ['并发数', 'Concurrency'], startMigration: ['开始迁移', 'Start migration'], titleProduct: ['LaTeX Pro。', 'LaTeX Pro.'],
    batchUpload: ['批量补传 ZIP', 'Batch-upload ZIPs'], batchUploadDescription: ['只需登录新系统。一次选择多个处理好的 ZIP，每个不超过 50 MB（50,000,000 字节）。若上次上传断线，请先核对新系统，避免重复创建。', 'Only the new system needs to be signed in. Select multiple prepared ZIPs at once; each must be no larger than 50 MB (50,000,000 bytes). If a previous upload was interrupted, check the new system first to avoid duplicates.'], chooseZip: ['选择 ZIP 文件（可多选）', 'Choose ZIP files (multiple allowed)'], notSelected: ['尚未选择', 'None selected'], startBatchUpload: ['开始批量补传', 'Start batch upload'],
    overallProgress: ['整体进度', 'Overall progress'], pauseAll: ['全部暂停', 'Pause all'], clearAll: ['全部清除', 'Clear all'], continueAll: ['全部继续', 'Resume all'], retryFailed: ['重试失败项目', 'Retry failed projects'], preparingMigration: ['准备迁移', 'Preparing migration'], oversizeTitle: ['超限项目需要处理', 'Oversized projects need attention'], oversizeDescription: ['以下项目已跳过。将这段提示词复制给你的 Agent，处理后使用「批量补传 ZIP」。', 'The following projects were skipped. Copy this prompt to your Agent, then use “Batch-upload ZIPs.”'], agentPrompt: ['复制给 Agent 的提示词', 'Prompt to copy to Agent'], cancel: ['取消', 'Cancel'], continue: ['继续', 'Continue'], migrationLog: ['迁移记录', 'Migration log'], securityNote: ['安全提示：登录窗口仅用于对应域名，Cookie 只保存在本次应用进程内。', 'Security: sign-in windows are only used for the corresponding domain; cookies stay in this app process.']
  };
  const normalizeLocale = locale => locale === 'en' || String(locale).toLowerCase().startsWith('en') ? 'en' : 'zh-CN';
  let current = normalizeLocale(localStorage.getItem('locale') || navigator.language);
  function t(key, params = {}) {
    const value = messages[key] ? messages[key][current === 'en' ? 1 : 0] : key;
    return String(value).replace(/\{(\w+)\}/g, (_, name) => params[name] == null ? `{${name}}` : params[name]);
  }
  function translateBackend(text) { return text; }
  function apply() {
    document.documentElement.lang = current;
    document.querySelectorAll('[data-i18n]').forEach(node => { node.textContent = t(node.dataset.i18n); });
    document.querySelectorAll('[data-i18n-aria]').forEach(node => { node.setAttribute('aria-label', t(node.dataset.i18nAria)); });
    const select = document.getElementById('language-select'); if (select) select.value = current;
  }
  function setLocale(locale) { const next = normalizeLocale(locale); if (next === current) { apply(); return; } current = next; localStorage.setItem('locale', current); apply(); window.dispatchEvent(new CustomEvent('localechange', { detail: { locale: current } })); }
  window.i18n = { get locale() { return current; }, setLocale, t, translateBackend };
  document.addEventListener('DOMContentLoaded', () => { apply(); const select = document.getElementById('language-select'); if (select) select.addEventListener('change', event => setLocale(event.target.value)); });
})();
