let localeSync = Promise.resolve();
const invoke = (name, args = {}) => name === 'set_locale' ? window.__TAURI__.core.invoke(name, args) : localeSync.then(() => window.__TAURI__.core.invoke(name, args));
const listen = (name, callback) => window.__TAURI__.event.listen(name, callback);

const $ = id => document.getElementById(id);
const state = { sourceCookie: false, targetCookie: false, projects: [], busy: false };
const sides = ['source', 'target'];
const authEpoch = { source: 0, target: 0 };
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let loading = false;
let localFiles = [];
let currentQueue = null;

const en = {
  '已连接': 'Connected', '登录窗口已关闭，可重新打开登录': 'Login window closed. Open it again to retry.',
  '等待登录完成，返回项目页后将自动连接…': 'Waiting for login; connection will be established when you return to the projects page…',
  '无法确认登录，可重新打开登录': 'Could not confirm login. Open the login window again.', '无法打开登录窗口': 'Could not open the login window',
  '地址已更改，请连接此站点': 'Address changed. Connect to this site.', '正在读取老系统项目列表…': 'Loading projects from the source system…',
  '排队中': 'Queued', '处理中': 'Processing', '已暂停': 'Paused', '已完成': 'Completed', '失败': 'Failed', '已跳过': 'Skipped',
  '继续': 'Resume', '暂停': 'Pause', '重试': 'Retry', '任务结束，有项目失败': 'Finished with failed projects', '任务完成': 'Finished',
  '上传 ZIP': 'Uploading ZIP', '下载 ZIP': 'Downloading ZIP', '总大小未知': 'total size unknown', '上传': 'Upload', '下载': 'Download',
  '请查看任务记录': 'See migration log', '队列结束，有项目失败，可逐项重试。': 'Queue finished with failures; retry individual projects.', '队列完成。': 'Queue completed.',
  '正在处理': 'Processing', '正在准备任务…': 'Preparing tasks…', '正在准备': 'Preparing', '等待重试': 'Waiting to retry',
  '仅清除本次进度、迁移记录和提示词，不会删除任何 ZIP 或其他文件。清除后无法从当前队列直接重试，是否继续？': 'Clear current progress, migration log and prompt? No ZIPs or other files will be deleted. You cannot retry directly from this queue afterward.',
  '日志会显示下载和上传的实时状态。': 'Live download and upload activity appears here.', '等待任务': 'Waiting for tasks', '准备就绪': 'Ready',
  '超过上限': 'Over limit', '待处理': 'Pending', '本批已准备；完成后可在队列中重试失败项目。': 'Batch prepared; failed projects can be retried from the queue when finished.'
};
const tr = text => window.i18n.locale === 'en' ? (en[text] || text) : text;
const bi = (zh, english) => window.i18n.locale === 'en' ? english : zh;
let localFileMessage = '尚未选择';
en['尚未选择'] = 'None selected'; en['尚未连接'] = 'Not connected';
const authMessages = {};
let projectsLoaded = false;
function updateProjectCount() { $('project-count').textContent = projectsLoaded ? bi(`${state.projects.length} 个可迁移项目`, `${state.projects.length} projects available`) : bi('完成登录后读取项目', 'Load projects after signing in'); }
function renderLocalFiles() {
  const rows = localFiles.map((file, i) => { const row = document.createElement('label'); row.className = 'zip-row'; const source = document.createElement('span'); source.textContent = bi(`ZIP 文件 ${i + 1}`, `ZIP file ${i + 1}`); const input = document.createElement('input'); input.value = file.name; input.setAttribute('aria-label', bi(`第 ${i + 1} 个项目名称`, `Project ${i + 1} name`)); input.addEventListener('input', () => { file.name = input.value; updateReady(); }); const status = document.createElement('small'); status.textContent = `${(file.bytes / 1e6).toFixed(2)} MB · ${tr(file.bytes > 50_000_000 ? '超过上限' : '待处理')}`; row.append(source, input, status); return row; });
  $('local-queue').replaceChildren(...rows); if (localFiles.length) $('local-file').textContent = bi(`已选 ${localFiles.length} 个，可分别修改项目名称`, `${localFiles.length} selected; project names can be edited`);
}
function renderPrompt(q) {
  const skipped = q.items.filter(item => item.status === 'skipped');
  if (!skipped.length) return;
  const files = skipped.map(item => `- ${item.name}: ${item.path || tr('请查看任务记录')}`).join('\n');
  $('agent-prompt').value = bi(`请处理以下超出上传限制的 LaTeX 项目 ZIP，分别生成不超过 50 MB（50,000,000 字节）的新 ZIP，不要覆盖原文件：\n${files}\n\n优先处理体积最大的图片。若图片过大，可考虑转为 JPG 并使用 quality 95；保留图像可读性与透明度要求，并同步修改 LaTeX 引用。不要删除源文件、字体或宏包配置。完成后检查 ZIP 路径和主文档引用，并报告新 ZIP 的路径及大小。`, `Please process the following LaTeX project ZIPs that exceed the upload limit. Create a new ZIP of at most 50 MB (50,000,000 bytes) for each without overwriting the originals:\n${files}\n\nStart with the largest images. If images are too large, consider converting them to JPEG at quality 95 while preserving readability and transparency requirements, and update LaTeX references accordingly. Do not delete source files, fonts, or package configuration. Check ZIP paths and main document references, and report the paths and sizes of the new ZIPs.`);
}

function addLog(message, type = '') {
  const list = $('log-list');
  if (list.querySelector('.muted')) list.innerHTML = '';
  const row = document.createElement('div'); row.className = `log-line ${type}`;
  row.textContent = `[${new Date().toLocaleTimeString(window.i18n.locale)}] ${tr(message)}`;
  list.appendChild(row); list.scrollTop = list.scrollHeight;
  $('log-summary').textContent = bi(`${list.children.length} 条记录`, `${list.children.length} entries`);
  $('clear-history').disabled = state.busy;
}
function setAuth(side, ok, message) {
  authMessages[side] = { ok, message };
  const el = $(`${side}-state`); el.classList.toggle('ok', ok);
  const icon = document.createElement('span'); icon.className = 'state-icon'; icon.textContent = ok ? '✓' : side === 'source' ? '1' : '2';
  el.replaceChildren(icon, document.createTextNode(tr(message)));
}
function updateReady() {
  $('load-projects').disabled = !state.sourceCookie || state.busy || loading;
  $('start-migration').disabled = !state.sourceCookie || !state.targetCookie || loading || state.busy || !document.querySelector('.project-row input:checked');
  $('pick-zip').disabled = state.busy || loading;
  $('upload-local').disabled = !state.targetCookie || !localFiles.length || localFiles.some(f => !f.name.trim()) || loading || state.busy;
  for (const side of sides) { $(`${side}-url`).disabled = state.busy || loading; $(`${side}-login`).disabled = state.busy || loading; }
  $('clear-history').disabled = state.busy || (!currentQueue && !$('log-list').querySelector('.log-line:not(.muted)'));
  for (const id of ['workdir', 'pick-workdir', 'select-all', 'select-none']) $(id).disabled = state.busy;
  document.querySelectorAll('.project-row input').forEach(input => { input.disabled = state.busy; });
}
async function configure() { await invoke('configure', { input: { sourceUrl: $('source-url').value, targetUrl: $('target-url').value } }); }
async function watchLogin(side, epoch, opened) {
  try {
    while (epoch === authEpoch[side]) {
      const status = await invoke('auth_status', { side });
      if (epoch !== authEpoch[side]) return;
      if (status === 'connected') { state[`${side}Cookie`] = true; setAuth(side, true, '已连接'); updateReady(); return; }
      if (status === 'closed') { if (opened) setAuth(side, false, '登录窗口已关闭，可重新打开登录'); return; }
      setAuth(side, false, '等待登录完成，返回项目页后将自动连接…'); await delay(1000);
    }
  } catch (error) { if (epoch === authEpoch[side]) { setAuth(side, false, '无法确认登录，可重新打开登录'); addLog(String(error), 'failed'); } }
}
async function openLogin(side) {
  const epoch = ++authEpoch[side]; state[`${side}Cookie`] = false; updateReady();
  try { await configure(); await invoke('open_oauth', { side }); await watchLogin(side, epoch, true); }
  catch (error) { setAuth(side, false, '无法打开登录窗口'); addLog(String(error), 'failed'); }
}
function escapeHtml(value) { return String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])); }
function renderProjects() {
  const list = $('project-list'); list.innerHTML = ''; updateProjectCount();
  state.projects.forEach(project => {
    const row = document.createElement('label'); row.className = 'project-row';
    row.innerHTML = `<input type="checkbox" value="${escapeHtml(project.id)}" /><span>${escapeHtml(project.name)}</span><span class="project-meta">${escapeHtml(project.lastUpdated || '')}</span>`;
    row.querySelector('input').addEventListener('change', updateReady); list.appendChild(row);
  }); updateReady();
}
async function loadProjects() {
  loading = true; updateReady();
  try { await configure(); addLog('正在读取老系统项目列表…'); state.projects = await invoke('list_projects'); projectsLoaded = true; renderProjects(); addLog(bi(`已读取 ${state.projects.length} 个项目`, `Loaded ${state.projects.length} projects`), 'success'); }
  catch (error) { addLog(String(error), 'failed'); } finally { loading = false; updateReady(); }
}
function queueSummary(q) {
  const counts = { success: 0, failed: 0, skipped: 0, pending: 0 };
  q.items.forEach(item => { if (item.status in counts) counts[item.status]++; else counts.pending++; }); return counts;
}
function showBatch(q) {
  const counts = queueSummary(q); const done = counts.success + counts.failed + counts.skipped;
  $('batch-progress').max = q.items.length || 1; $('batch-progress').value = done;
  $('batch-count').textContent = bi(`${done} / ${q.items.length} 已处理 · 成功 ${counts.success} · 跳过 ${counts.skipped} · 失败 ${counts.failed} · 待处理 ${counts.pending}`, `${done} / ${q.items.length} processed · ${counts.success} succeeded · ${counts.skipped} skipped · ${counts.failed} failed · ${counts.pending} pending`);
}
function phase(title) { $('phase-name').textContent = tr(title); $('file-count').textContent = ''; $('file-progress').removeAttribute('value'); }
function statusText(item) { return tr(({ queued: '排队中', running: '处理中', paused: '已暂停', success: '已完成', failed: '失败', skipped: '已跳过' })[item.status] || item.status); }
function renderQueue(q) {
  const list = $('queue-list'); if (!list || !q) return;
  const rebuild = list.children.length !== q.items.length || q.items.some((item, index) => { const row = list.children[index]; return !row || Number(row.dataset.id) !== item.id || row.dataset.status !== item.status; });
  if (rebuild) {
    list.replaceChildren();
    q.items.forEach(item => {
      const row = document.createElement('div'); row.className = `queue-item queue-${item.status}`; row.dataset.id = item.id; row.dataset.status = item.status;
      const title = document.createElement('span'); title.className = 'queue-name'; title.textContent = item.name;
      const detail = document.createElement('small'); detail.className = 'queue-detail';
      const meter = document.createElement('progress'); meter.className = 'queue-meter'; meter.max = 1; meter.value = 0;
      const stateEl = document.createElement('span'); stateEl.className = 'queue-status';
      const actions = document.createElement('span'); actions.className = 'queue-actions';
      row.append(title, detail, meter, stateEl, actions); list.appendChild(row);
    });
  }
  q.items.forEach(item => {
    const row = [...list.children].find(candidate => Number(candidate.dataset.id) === item.id); if (!row) return;
    row.dataset.status = item.status; row.className = `queue-item queue-${item.status}`;
    row.querySelector('.queue-detail').textContent = tr(item.message) || statusText(item); row.querySelector('.queue-status').textContent = statusText(item);
    const meter = row.querySelector('.queue-meter'); const stage = item.focusStage || 'upload'; const total = item.totals?.[stage] || 0; const bytes = item.bytes?.[stage] || 0;
    if (item.status === 'skipped' || item.status === 'success') { meter.max = 1; meter.value = 1; } else if (total > 0) { meter.max = total; meter.value = bytes; } else if (item.status === 'queued' || item.status === 'failed' || item.status === 'paused') { meter.max = 1; meter.value = 0; } else meter.removeAttribute('value');
    const actions = row.querySelector('.queue-actions'); actions.replaceChildren();
    if (['queued', 'running', 'paused'].includes(item.status)) { const button = document.createElement('button'); button.className = 'text-button'; button.textContent = tr(item.status === 'paused' ? '继续' : '暂停'); button.addEventListener('click', () => toggleItemPause(item.id)); actions.appendChild(button); }
    if (item.status === 'failed') { const button = document.createElement('button'); button.className = 'text-button'; button.textContent = tr('重试'); button.addEventListener('click', () => retryItems([item.id])); actions.appendChild(button); }
  });
  showBatch(q); updateQueueControls(q);
}
function updateQueueControls(q) {
  $('pause-all').disabled = !q || q.finished || !q.items.some(item => ['queued', 'running', 'paused'].includes(item.status));
  $('continue-all').disabled = !q || q.finished || !q.items.some(item => item.status === 'paused');
  $('retry-failed').disabled = !q || !q.items.some(item => item.status === 'failed');
}
function updateRunPanel(q) {
  $('progress-panel').hidden = !q;
  if (!q) return;
  const counts = queueSummary(q); $('run-light').className = `status-light ${q.finished ? (counts.failed ? 'error' : 'done') : 'running'}`;
  $('run-status').textContent = q.finished ? tr(counts.failed ? '任务结束，有项目失败' : '任务完成') : bi(`正在处理（并发 ${q.limit}）`, `Processing (${q.limit} concurrent)`);
}
function mib(n) { return `${(n / 1048576).toFixed(2)} MiB`; }
function updateBytes(q, p) {
  const item = q.items.find(candidate => candidate.id === Number(p.id)); if (!item) return;
  item.bytes[p.stage] = Number(p.bytes) || 0; item.totals[p.stage] = Number(p.totalBytes) || 0; item.focusStage = p.stage; q.focusId = item.id;
  const bar = $('file-progress'); $('phase-name').textContent = tr(p.stage === 'upload' ? '上传 ZIP' : '下载 ZIP');
  const bytes = item.bytes[p.stage]; const total = item.totals[p.stage];
  if (total > 0) { bar.max = total; bar.value = bytes; $('file-count').textContent = `${mib(bytes)} / ${mib(total)} · ${Math.min(100, Math.floor(bytes * 100 / total))}%`; }
  else { bar.removeAttribute('value'); $('file-count').textContent = `${mib(bytes)} · ${tr('总大小未知')}`; }
  item.message = `${tr(p.stage === 'upload' ? '上传' : '下载')} ${$('file-count').textContent}`; renderQueue(q);
}
function itemFromEvent(q, p) {
  const id = p.kind === 'progress' || p.kind === 'result' ? Number(p.index) - 1 : Number(p.id); return q.items.find(item => item.id === id);
}
function applyResult(q, item, status, message, path) {
  if (!item || ['success', 'failed', 'skipped'].includes(item.status)) return;
  if (status === 'failed' && /任务已暂停，请先继续|Task is paused/.test(String(message || ''))) { item.status = 'paused'; item.paused = true; item.message = '已暂停'; renderQueue(q); return; }
  item.status = ['success', 'failed', 'skipped'].includes(status) ? status : 'failed'; item.message = message || statusText(item); item.path = path || item.path;
  if (item.status === 'failed' && item.path && !item.message.includes(item.path)) item.message += ` · ZIP：${item.path}`;
  addLog(`${item.name} · ${item.message}`, item.status); renderQueue(q); drainQueue(q);
}
function onStage({ payload: p }) {
  const q = currentQueue; if (!q || q.finished || p.batchId !== q.batchId) return;
  if (p.kind === 'progress') {
    const item = itemFromEvent(q, p); if (!item) return;
    item.message = p.message || item.message; item.focusStage = p.stage || item.focusStage; q.focusId = item.id; renderQueue(q); phase(item.message); $('run-status').textContent = `[${item.id + 1}/${q.items.length}] ${item.name}`;
  } else if (p.kind === 'log') addLog(`${p.project || ''} ${p.message || ''}`);
  else if (p.kind === 'result') applyResult(q, itemFromEvent(q, p), p.status, p.message, p.path);
  else if (p.kind === 'finished' && p.message) addLog(p.message);
}
function onBytes({ payload: p }) { const q = currentQueue; if (!q || q.finished || p.batchId !== q.batchId) return; updateBytes(q, p); }
async function attachListeners(q) {
  if (q.listeners.length) return; q.listeners.push(await listen('migration-event', onStage)); q.listeners.push(await listen('transfer-progress', onBytes));
}
function finishQueue(q) {
  if (q.finished || q.items.some(item => !['success', 'failed', 'skipped'].includes(item.status))) return;
  q.finished = true; q.listeners.splice(0).forEach(stop => stop()); state.busy = false; updateRunPanel(q); renderQueue(q); updateReady();
  $('current-stage').hidden = true;
  const skipped = q.items.filter(item => item.status === 'skipped');
  $('oversize-guidance').hidden = skipped.length === 0;
  if (skipped.length) renderPrompt(q);
  const failed = queueSummary(q).failed; addLog(failed ? '队列结束，有项目失败，可逐项重试。' : '队列完成。', failed ? 'failed' : 'success'); q.resolve?.();
}
function drainQueue(q) {
  if (currentQueue !== q || q.finished) return;
  while (q.active.size < q.limit) { const item = q.items.find(candidate => candidate.status === 'queued' && !candidate.paused); if (!item) break; runQueueItem(q, item); }
  const pending = q.items.some(item => !['success', 'failed', 'skipped'].includes(item.status));
  if (!q.active.size && !pending) finishQueue(q); renderQueue(q);
}
async function runQueueItem(q, item) {
  if (q.finished || item.status !== 'queued' || item.paused || q.active.has(item.id)) return;
  q.active.add(item.id); item.status = 'running'; item.message = '正在处理'; renderQueue(q);
  try { const result = await invoke('run_queue_item', { batchId: q.batchId, id: item.id }); applyResult(q, item, result.status, result.message, result.path); }
  catch (error) { applyResult(q, item, 'failed', String(error)); }
  finally { q.active.delete(item.id); drainQueue(q); }
}
async function prepareQueue(mode, names, input) {
  if (state.busy || (mode === 'local' && !names.length) || (mode === 'migration' && !input.projectIds?.length)) return;
  const q = { mode, names, input, limit: Number($('queue-limit').value) || 2, batchId: null, items: [], active: new Set(), listeners: [], finished: false, done: null };
  $('oversize-guidance').hidden = true;
  currentQueue = q; state.busy = true; updateReady(); updateRunPanel(q); phase('正在准备任务…'); $('run-status').textContent = tr('正在准备'); q.done = new Promise(resolve => { q.resolve = resolve; });
  try {
    await attachListeners(q); await configure(); const prepared = await invoke('prepare_queue', { mode, names, input, limit: q.limit });
    if (currentQueue !== q) return; q.batchId = prepared.batchId; q.items = prepared.items.map(item => ({ ...item, status: 'queued', paused: false, bytes: {}, totals: {} }));
    $('current-stage').hidden = false; renderQueue(q); addLog(bi(`已准备 ${q.items.length} 个任务（并发 ${q.limit}）`, `Prepared ${q.items.length} tasks (${q.limit} concurrent)`)); drainQueue(q); await q.done;
  } catch (error) { q.listeners.splice(0).forEach(stop => stop()); q.finished = true; state.busy = false; $('current-stage').hidden = true; updateRunPanel(q); updateReady(); addLog(String(error), 'failed'); }
}
async function setItemPaused(q, item, paused) {
  if (!item || q.finished || item.pausePromise) return;
  const wasActive = q.active.has(item.id);
  item.paused = paused; item.status = paused ? 'paused' : (wasActive ? 'running' : 'queued');
  renderQueue(q);
  item.pausePromise = invoke('pause_queue_item', { batchId: q.batchId, id: item.id, paused }).catch(error => {
    if (!['success', 'failed', 'skipped'].includes(item.status)) { item.paused = !paused; item.status = wasActive ? 'running' : 'queued'; }
    addLog(String(error), 'failed');
  }).finally(() => { item.pausePromise = null; renderQueue(q); if (!paused && !q.finished) drainQueue(q); });
  await item.pausePromise;
}
async function toggleItemPause(id) {
  const q = currentQueue; const item = q?.items.find(candidate => candidate.id === id); if (!item || q.finished) return;
  if (item.status === 'paused') await setItemPaused(q, item, false); else if (['queued', 'running'].includes(item.status)) await setItemPaused(q, item, true);
}
async function setAllPaused(paused) {
  const q = currentQueue; if (!q || q.finished) return;
  await Promise.all(q.items.filter(item => ['queued', 'running', 'paused'].includes(item.status) && item.paused !== paused).map(item => setItemPaused(q, item, paused)));
  renderQueue(q); if (!paused) drainQueue(q);
}
async function retryItems(ids) {
  const q = currentQueue; if (!q) return; const items = q.items.filter(item => ids.includes(item.id) && item.status === 'failed'); if (!items.length) return;
  if (!await confirmAction(items.length === 1 ? bi(`确定重试「${items[0].name}」吗？重复项目可能被再次创建。`, `Retry “${items[0].name}”? This may create a duplicate project.`) : bi(`确定重试这 ${items.length} 个失败项目吗？重复项目可能被再次创建。`, `Retry these ${items.length} failed projects? This may create duplicate projects.`))) return;
  await Promise.all(items.map(async item => { if (item.pausePromise) await item.pausePromise; try { await invoke('pause_queue_item', { batchId: q.batchId, id: item.id, paused: false }); } catch (error) { addLog(String(error), 'failed'); } }));
  $('current-stage').hidden = false;
  $('oversize-guidance').hidden = true;
  await attachListeners(q); q.finished = false; state.busy = true; items.forEach(item => { item.status = 'queued'; item.paused = false; item.message = '等待重试'; }); updateRunPanel(q); updateReady(); renderQueue(q); drainQueue(q);
}
async function clearHistory() {
  if (state.busy) return;
  if (!await confirmAction(tr('仅清除本次进度、迁移记录和提示词，不会删除任何 ZIP 或其他文件。清除后无法从当前队列直接重试，是否继续？'))) return;
  if (state.busy) return;
  currentQueue = null;
  $('progress-panel').hidden = true;
  $('current-stage').hidden = true;
  $('oversize-guidance').hidden = true;
  $('agent-prompt').value = '';
  $('queue-list').replaceChildren();
  $('log-list').innerHTML = ''; const placeholder = document.createElement('div'); placeholder.className = 'log-line muted'; placeholder.textContent = tr('日志会显示下载和上传的实时状态。'); $('log-list').appendChild(placeholder);
  $('log-summary').textContent = tr('等待任务');
  $('run-light').className = 'status-light';
  $('run-status').textContent = tr('准备就绪');
  updateReady();
}
function migrate() {
  const selected = [...document.querySelectorAll('.project-row input:checked')];
  return prepareQueue('migration', [], { projectIds: selected.map(input => input.value), workdir: $('workdir').value });
}
async function pickZip() {
  try {
    const files = await invoke('pick_local_zip'); if (!files) return; localFiles = files; localFileMessage = '尚未选择';
    renderLocalFiles(); updateReady();
  } catch (error) { addLog(String(error), 'failed'); }
}
function uploadLocal() {
  if (!localFiles.length || !state.targetCookie || state.busy) return;
  const names = localFiles.map(file => file.name.trim()); localFiles = []; $('local-queue').replaceChildren(); localFileMessage = '本批已准备；完成后可在队列中重试失败项目。'; $('local-file').textContent = tr(localFileMessage);
  return prepareQueue('local', names, { projectIds: [], workdir: '' });
}
function confirmAction(message) {
  const dialog = $('confirm-dialog'); if (!dialog) return Promise.resolve(false); $('confirm-message').textContent = message; dialog.showModal();
  return new Promise(resolve => {
    let settled = false; const onClose = () => { if (!settled) { settled = true; resolve(false); } }; const close = value => { if (settled) return; settled = true; dialog.removeEventListener('close', onClose); if (dialog.open) dialog.close(); resolve(value); };
    dialog.addEventListener('close', onClose, { once: true }); $('confirm-ok').onclick = () => close(true); $('confirm-cancel').onclick = () => close(false);
  });
}
$('source-login').addEventListener('click', () => openLogin('source')); $('target-login').addEventListener('click', () => openLogin('target'));
for (const side of sides) { const field = $(`${side}-url`); field.value = sessionStorage.getItem(`${side}-url`) || field.value; field.addEventListener('input', () => { authEpoch[side]++; state[`${side}Cookie`] = false; if (side === 'source') { state.projects = []; projectsLoaded = false; renderProjects(); } sessionStorage.setItem(`${side}-url`, field.value); setAuth(side, false, '地址已更改，请连接此站点'); updateReady(); }); }
$('load-projects').addEventListener('click', loadProjects); $('pick-workdir').addEventListener('click', async () => { try { const path = await invoke('pick_workdir'); if (path) $('workdir').value = path; } catch (error) { addLog(String(error), 'failed'); } }); $('start-migration').addEventListener('click', migrate); $('pick-zip').addEventListener('click', pickZip); $('upload-local').addEventListener('click', uploadLocal);
$('select-all').addEventListener('click', () => { document.querySelectorAll('.project-row input').forEach(input => { input.checked = true; }); updateReady(); }); $('select-none').addEventListener('click', () => { document.querySelectorAll('.project-row input').forEach(input => { input.checked = false; }); updateReady(); });
$('pause-all').addEventListener('click', () => setAllPaused(true)); $('continue-all').addEventListener('click', () => setAllPaused(false)); $('retry-failed').addEventListener('click', () => currentQueue && retryItems(currentQueue.items.filter(item => item.status === 'failed').map(item => item.id)));
$('clear-history').addEventListener('click', clearHistory);
window.addEventListener('localechange', () => {
  const locale = window.i18n.locale; localeSync = localeSync.then(() => invoke('set_locale', { locale })).catch(error => { addLog(String(error), 'failed'); });
  for (const side of sides) if (authMessages[side]) setAuth(side, authMessages[side].ok, authMessages[side].message);
  if (!localFiles.length) $('local-file').textContent = tr(localFileMessage);
  if (!currentQueue) $('run-status').textContent = tr('准备就绪');
  if (!$('log-list').querySelector('.log-line:not(.muted)')) { $('log-summary').textContent = tr('等待任务'); const placeholder = $('log-list').querySelector('.muted'); if (placeholder) placeholder.textContent = tr('日志会显示下载和上传的实时状态。'); }
  updateProjectCount();
  if (localFiles.length) renderLocalFiles();
  if (currentQueue) { renderQueue(currentQueue); updateRunPanel(currentQueue); if (currentQueue.finished) renderPrompt(currentQueue); }
  const logCount = $('log-list').querySelectorAll('.log-line:not(.muted)').length;
  if (logCount) $('log-summary').textContent = bi(`${logCount} 条记录`, `${logCount} entries`);
});
for (const side of sides) setAuth(side, false, '尚未连接');
window.dispatchEvent(new Event('localechange'));
configure().then(() => sides.forEach(side => watchLogin(side, authEpoch[side], false))).catch(error => addLog(String(error), 'failed'));
