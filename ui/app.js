/* Static Tauri webview. No remote assets, JS build pipeline, or API calls. */
const runtime = window.__TAURI__;
const invoke = runtime?.core?.invoke;
const listen = runtime?.event?.listen;
const $ = (id) => document.getElementById(id);
const model = $('model');
let recording = false;
let completed = [];
let currentSession = null;

function status(message, isRecording = recording, error = false) {
  recording = isRecording;
  $('status-label').textContent = message;
  $('status-label').classList.toggle('error', error);
  $('status-dot').classList.toggle('recording', recording);
  $('record').classList.toggle('active', recording);
  $('record-label').textContent = recording ? 'Закончить диктовку' : 'Начать диктовку';
}

async function call(name, args = {}) {
  if (!invoke) throw new Error('Запустите приложение через Tauri, не браузер');
  return invoke(name, args);
}

async function loadModels() {
  const available = await call('available_models');
  model.replaceChildren();
  if (!available.length) {
    model.add(new Option('Модели не найдены. Запустите prepare-assets.ps1', ''));
    return;
  }
  for (const entry of available) model.add(new Option(entry.name, entry.path));
  const wanted = localStorage.getItem('local-model');
  if (wanted && available.some(x => x.path === wanted)) model.value = wanted;
  await selectModel(model.value);
}

async function selectModel(path) {
  if (!path) return;
  await call('select_model', { path });
  localStorage.setItem('local-model', path);
}

function transcriptPreview() {
  $('transcribed').textContent = completed.join(' ') || 'Здесь появится распознанный текст…';
}

function makeQueueItem(entry, uncertain) {
  const div = document.createElement('div');
  div.className = 'queue-item';
  const p = document.createElement('p');
  p.textContent = entry.text;
  const actions = document.createElement('div');
  actions.className = 'queue-actions';
  const copy = document.createElement('button');
  copy.className = 'mini'; copy.textContent = 'Копировать';
  copy.addEventListener('click', async () => {
    try { await navigator.clipboard.writeText(entry.text); copy.textContent = 'Скопировано'; }
    catch (err) { status(String(err), recording, true); }
  });
  const clear = document.createElement('button');
  clear.className = 'mini danger'; clear.textContent = 'Убрать';
  clear.addEventListener('click', async () => {
    await call('discard_pending', { id: entry.id });
  });
  actions.append(copy, clear);
  div.append(p, actions);
  if (uncertain) div.title = 'Автоматического повтора не будет. Используйте ручное копирование.';
  return div;
}

function renderDelivery(state) {
  $('queue-size').textContent = String(state.pending.length);
  $('queue-note').textContent = state.pending.length ?
    'Ожидает фокуса или наведения на редактируемое поле:' :
    'Нет ожидающих фрагментов.';
  $('queue-items').replaceChildren(...state.pending.map(p => makeQueueItem(p, false)));
  $('uncertain-items').replaceChildren(...state.uncertain.map(p => makeQueueItem(p, true)));
  $('delivery-message').textContent = state.last_message;
}

$('record').addEventListener('click', async () => {
  try {
    if (recording) {
      await call('end_recording');
      status('Останавливаем микрофон…', true);
    } else {
      completed = []; currentSession = null; transcriptPreview(); $('partial').textContent = '';
      await call('begin_recording');
      status('Загружаем модель…', true);
    }
  } catch (err) { status(String(err), false, true); }
});
$('refresh-models').addEventListener('click', () => loadModels().catch(e => status(String(e), false, true)));
model.addEventListener('change', () => selectModel(model.value).catch(e => status(String(e), false, true)));
$('apply-path').addEventListener('click', async () => {
  try { await selectModel($('custom-path').value.trim()); await loadModels(); status('Локальная модель выбрана'); }
  catch (e) { status(String(e), recording, true); }
});
$('auto-stop').addEventListener('change', async (ev) => {
  localStorage.setItem('auto-stop', String(ev.target.checked));
  await call('set_auto_stop', { enabled: ev.target.checked });
});
$('hover-insert').addEventListener('change', async (ev) => {
  localStorage.setItem('hover-insert', String(ev.target.checked));
  await call('set_hover_delivery', { enabled: ev.target.checked });
});

async function init() {
  if (!runtime) { status('Откройте Voice Input как Windows-приложение', false, true); return; }
  $('auto-stop').checked = localStorage.getItem('auto-stop') === 'true';
  $('hover-insert').checked = localStorage.getItem('hover-insert') !== 'false';
  await call('set_auto_stop', { enabled: $('auto-stop').checked });
  await call('set_hover_delivery', { enabled: $('hover-insert').checked });
  await listen('recording-status', ({payload}) => {
    status(payload.message, payload.recording, !payload.recording && /не найден|ошиб|не удалось|Vosk/.test(payload.message));
    if (!payload.recording) $('partial').textContent = '';
  });
  await listen('transcription-partial', ({payload}) => {
    if (currentSession != null && currentSession !== payload.session) return;
    currentSession = payload.session;
    $('partial').textContent = payload.text;
  });
  await listen('transcription-final', ({payload}) => {
    if (currentSession != null && currentSession !== payload.session) return;
    currentSession = payload.session;
    completed.push(payload.text);
    $('partial').textContent = '';
    transcriptPreview();
  });
  await listen('delivery-status', ({payload}) => renderDelivery(payload));
  renderDelivery(await call('delivery_status'));
  const previous = await call('recording_status');
  status(previous.message, previous.recording);
  await loadModels();
}
init().catch(err => status(String(err), false, true));
