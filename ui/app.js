/* Static local webview. Update networking is restricted to native commands. */
const runtime = window.__TAURI__;
const invoke = runtime?.core?.invoke;
const listen = runtime?.event?.listen;
const $ = (id) => document.getElementById(id);
const model = $('model');
const transcript = $('transcribed');
let recording = false;
let currentSession = null;
let transcriptBusy = false;
let transcriptRevision = 0;
let claimChain = Promise.resolve();
const transcriptIds = new Set();
const recordBook = new window.DictationRecords();
let recordTransitionBusy = false;
let lastDelivery = { pending: [], uncertain: [], last_message: '' };
const tabNames = ['dictation', 'model', 'settings', 'queue'];
let selectedTab = 'dictation';
let editorOpen = false;
let latestFinal = '';
let hotkeyView = { shortcut: 'Ctrl+Alt+Space', default_shortcut: 'Ctrl+Alt+Space', active: false };
let hotkeySaving = false;
let hotkeyCapturing = false;
let hotkeyCaptureStarting = false;
let hotkeyCaptureGeneration = 0;
let hotkeyCaptureTimer;
const wave = new window.VoiceWave($('voice-waves'), $('wave-front'), $('wave-back'), $('wave-ribbon'), renderSignalStatus);
const updates = new window.VoiceUpdates($, (name, args) => call(name, args), listen,
  (busy) => { $('record').disabled = busy; });

function renderSignalStatus(signal) {
  const warning = $('input-warning');
  warning.hidden = signal === 'normal';
  warning.textContent = signal === 'overload' ? 'Слишком громко' : signal === 'high' ? 'Высокий уровень' : '';
  warning.setAttribute('data-signal', signal);
  warning.title = signal === 'overload' ? 'Сигнал близок к перегрузке. Отодвиньте микрофон или уменьшите его усиление.' :
    signal === 'high' ? 'Высокий уровень сигнала микрофона.' : '';
}
renderSignalStatus('normal');

function waveVisibility() { wave.setVisible(selectedTab === 'dictation' && !document.hidden); }
document.addEventListener?.('visibilitychange', waveVisibility);

function editorVisibility(open, focus = false) {
  editorOpen = open;
  $('transcript-editor').hidden = !open;
  $('panel-dictation').classList.toggle('editor-open', open);
  $('toggle-editor').setAttribute('aria-expanded', String(open));
  $('toggle-editor').textContent = open ? 'Скрыть редактор' : 'Открыть редактор';
  if (focus) (open ? transcript : $('toggle-editor')).focus();
}
$('toggle-editor').addEventListener('click', () => editorVisibility(!editorOpen, true));
editorVisibility(false);

function compactPreview(text) {
  $('compact-preview').textContent = text || 'Поставьте курсор в нужное поле и начните говорить.';
}

function selectTab(name, focus = false) {
  if (!tabNames.includes(name)) return;
  if (name !== 'settings' && (hotkeyCapturing || hotkeyCaptureStarting)) stopHotkeyCapture();
  selectedTab = name;
  for (const item of tabNames) {
    const selected = item === name;
    const tab = $(`tab-${item}`);
    tab.setAttribute('aria-selected', String(selected));
    tab.tabIndex = selected ? 0 : -1;
    $(`panel-${item}`).hidden = !selected;
  }
  if (focus) $(`tab-${name}`).focus();
  waveVisibility();
}

for (const [index, name] of tabNames.entries()) {
  $(`tab-${name}`).addEventListener('click', () => selectTab(name));
  $(`tab-${name}`).addEventListener('keydown', (event) => {
    let target;
    if (event.key === 'ArrowRight') target = tabNames[(index + 1) % tabNames.length];
    if (event.key === 'ArrowLeft') target = tabNames[(index + tabNames.length - 1) % tabNames.length];
    if (event.key === 'Home') target = tabNames[0];
    if (event.key === 'End') target = tabNames[tabNames.length - 1];
    if (target) { event.preventDefault(); selectTab(target, true); }
  });
}
selectTab('dictation');

const nativeWindow = runtime?.window?.getCurrentWindow?.();
for (const [id, action] of [['window-minimize', 'minimize'], ['window-maximize', 'toggleMaximize'], ['window-close', 'close']]) {
  $(id).disabled = !nativeWindow;
  $(id).addEventListener('click', async () => {
    try { await nativeWindow?.[action](); }
    catch (error) { status(String(error), recording, true); }
  });
}

function status(message, isRecording = recording, error = false) {
  recording = isRecording;
  wave.setRecording(recording);
  $('status-label').textContent = message;
  $('status-label').classList.toggle('error', error);
  $('status-dot').classList.toggle('recording', recording);
  $('record').classList.toggle('active', recording);
  $('record').setAttribute('aria-pressed', String(recording));
  $('record-label').textContent = recording ? 'Закончить диктовку' : 'Начать диктовку';
  for (const id of ['model', 'refresh-models', 'custom-path', 'apply-path']) $(id).disabled = recording;
  $('model-lock').hidden = !recording;
  updateFooter();
  transcriptControls();
}

async function call(name, args = {}) {
  if (!invoke) throw new Error('Запустите приложение через Tauri, не браузер');
  return invoke(name, args);
}

function prettyHotkey(value) { return value.split('+').join(' + '); }

function updateFooter() {
  $('footer-status').textContent = recording ?
    `Диктовка включена${hotkeyView.active ? ` · ${prettyHotkey(hotkeyView.shortcut)}` : ''}` :
    'Микрофон по умолчанию';
  $('footer-status').title = $('footer-status').textContent;
}

function hotkeyFeedback(message, error = false) {
  $('hotkey-feedback').textContent = message;
  $('hotkey-feedback').classList.toggle('error', error);
}

function hotkeyControls() {
  const capturing = hotkeyCapturing || hotkeyCaptureStarting;
  $('hotkey-input').readOnly = capturing || hotkeySaving;
  $('capture-hotkey').disabled = hotkeySaving || hotkeyCaptureStarting;
  $('capture-hotkey').textContent = hotkeyCapturing ? 'Отмена' : 'Записать';
  $('capture-hotkey').setAttribute('aria-pressed', String(hotkeyCapturing));
  $('apply-hotkey').disabled = capturing || hotkeySaving;
  $('reset-hotkey').disabled = capturing || hotkeySaving;
}

function renderHotkey(view) {
  hotkeyView = view;
  $('hotkey-input').value = view.shortcut;
  $('hotkey-input').setAttribute('aria-invalid', 'false');
  $('shortcut-label').textContent = view.active ? prettyHotkey(view.shortcut) : 'Сочетание недоступно';
  $('shortcut-label').title = view.error || 'Начать или остановить диктовку';
  $('hotkey-active').textContent = view.active ? 'Активно' : 'Не активно';
  $('hotkey-active').classList.toggle('error', !view.active);
  hotkeyFeedback(view.error || 'Сохраняется после выхода. Работает при скрытом окне.', !!view.error);
  updateFooter();
}

async function stopHotkeyCapture(value, message) {
  const wasCapturing = hotkeyCapturing || hotkeyCaptureStarting;
  hotkeyCaptureGeneration += 1;
  hotkeyCapturing = false;
  hotkeyCaptureStarting = false;
  window.clearTimeout(hotkeyCaptureTimer);
  if (value) $('hotkey-input').value = value;
  hotkeyControls();
  if (!wasCapturing) return;
  hotkeyFeedback(message || (value ? 'Сочетание записано. Нажмите «Применить».' : 'Запись сочетания отменена.'));
  try { await call('set_hotkey_capture', { enabled: false }); }
  catch (error) { hotkeyFeedback(String(error), true); }
}

async function startHotkeyCapture() {
  if (hotkeyCapturing) { await stopHotkeyCapture(); return; }
  if (hotkeySaving || hotkeyCaptureStarting) return;
  const generation = ++hotkeyCaptureGeneration;
  hotkeyCaptureStarting = true;
  hotkeyControls();
  try {
    await call('set_hotkey_capture', { enabled: true });
    if (generation !== hotkeyCaptureGeneration || selectedTab !== 'settings' || document.hidden) {
      await call('set_hotkey_capture', { enabled: false });
      return;
    }
    hotkeyCaptureStarting = false;
    hotkeyCapturing = true;
    hotkeyControls();
    $('hotkey-input').focus();
    hotkeyFeedback('Нажмите сочетание. Esc — отмена. Диктовка от этого нажатия не начнётся.');
    hotkeyCaptureTimer = window.setTimeout(() => stopHotkeyCapture(undefined, 'Время записи истекло. Нажмите «Записать» ещё раз.'), 15000);
  } catch (error) {
    hotkeyFeedback(String(error), true);
  } finally {
    if (generation === hotkeyCaptureGeneration) {
      hotkeyCaptureStarting = false;
      hotkeyControls();
    }
  }
}

document.addEventListener?.('keydown', (event) => {
  if (!hotkeyCapturing) return;
  event.preventDefault();
  event.stopPropagation?.();
  if (event.code === 'Escape' && !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) {
    stopHotkeyCapture(); return;
  }
  if (/^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/.test(event.code)) return;
  if (event.repeat || !event.code || event.code === 'Unidentified') return;
  if (!event.ctrlKey && !event.altKey && !event.metaKey && !/^F(?:[1-9]|1\d|2[0-4])$/.test(event.code)) {
    hotkeyFeedback('Используйте Ctrl, Alt или Win вместе с клавишей либо одну из F1–F24.');
    return;
  }
  const parts = [];
  if (event.ctrlKey) parts.push('Ctrl');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  if (event.metaKey) parts.push('Win');
  parts.push(event.code.replace(/^Key(?=[A-Z]$)|^Digit(?=\d$)/, ''));
  stopHotkeyCapture(parts.join('+'));
});
window.addEventListener?.('blur', () => stopHotkeyCapture());
document.addEventListener?.('visibilitychange', () => { if (document.hidden) stopHotkeyCapture(); });

async function applyHotkey(shortcut) {
  if (hotkeySaving || hotkeyCapturing || hotkeyCaptureStarting) return;
  hotkeySaving = true;
  hotkeyControls();
  hotkeyFeedback('Проверяем сочетание…');
  try {
    const view = await call('set_hotkey', { shortcut });
    renderHotkey(view);
    if (!view.error) hotkeyFeedback(`Сохранено: ${prettyHotkey(view.shortcut)}. Начинает и останавливает диктовку.`);
  } catch (error) {
    $('hotkey-input').setAttribute('aria-invalid', 'true');
    hotkeyFeedback(String(error), true);
  } finally {
    hotkeySaving = false;
    hotkeyControls();
  }
}
$('capture-hotkey').addEventListener('click', startHotkeyCapture);
$('apply-hotkey').addEventListener('click', () => applyHotkey($('hotkey-input').value.trim()));
$('reset-hotkey').addEventListener('click', () => applyHotkey(hotkeyView.default_shortcut));
$('hotkey-input').addEventListener('input', () => {
  $('hotkey-input').setAttribute('aria-invalid', 'false');
  hotkeyFeedback('Нажмите «Применить», чтобы сохранить сочетание.');
});
$('hotkey-input').addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && !hotkeyCapturing) {
    event.preventDefault();
    applyHotkey($('hotkey-input').value.trim());
  }
});

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

function transcriptControls() {
  transcript.readOnly = recording || transcriptBusy || recordTransitionBusy;
  $('copy-transcript').disabled = recording || transcriptBusy || recordTransitionBusy || !transcript.value.trim();
  $('clear-transcript').disabled = recording || transcriptBusy || recordTransitionBusy || (!transcript.value && !transcriptIds.size);
  $('restart-record').disabled = transcriptBusy || recordTransitionBusy || (!recording && !transcript.value && !recordBook.current.partial);
  $('new-record').disabled = transcriptBusy || recordTransitionBusy;
  $('record').disabled = recordTransitionBusy && !recording;
  $('transcript-help').textContent = recording ?
    'Идёт диктовка. Остановите запись, чтобы поправить или скопировать текст.' :
    'Правка относится к текущей записи. Вставленный во внешнее поле текст измените вручную.';
}

function transcriptFeedback(message, error = false) {
  $('transcript-feedback').textContent = message;
  $('transcript-feedback').classList.toggle('error', error);
}

function takeTranscript(ids) {
  // Serialize ownership changes. Keep IDs after failures so Copy can retry the
  // handover, and remove only the captured IDs when newer dictation arrives.
  const operation = claimChain.then(async () => {
    const remaining = ids.filter(id => [...recordBook.entries.values()].some(entry => entry.pending.has(id)));
    if (!remaining.length) return;
    await call('take_pending', { ids: remaining });
    for (const id of remaining) transcriptIds.delete(id);
    recordBook.take(remaining);
  });
  claimChain = operation.catch(() => {});
  return operation;
}

function persistDraft(draft) {
  const operation = claimChain.then(() => call('edit_record', { recordId: draft.recordId, generation: draft.generation, text: draft.text, baseIds: draft.baseIds }));
  claimChain = operation.catch(() => {});
  return operation.then(updated => {
    recordBook.acknowledge(updated, draft.revision);
    recordBook.take(draft.baseIds);
    for (const id of draft.baseIds) transcriptIds.delete(id);
    return updated;
  });
}

function appendTranscript(payload) {
  if (!recordBook.append(payload)) return;
  showCurrentRecord();
  transcriptFeedback('');
  transcriptControls();
}

function acceptSession(session) {
  if (currentSession != null && session < currentSession) return false;
  if (session !== currentSession) {
    currentSession = session;
    wave.setRecording(false);
    recordBook.current.partial = '';
    showCurrentRecord();
  }
  return true;
}

function recordingStatus(payload) {
  if (!acceptSession(payload.session)) return;
  updates.setRecording(payload.recording);
  status(payload.message, payload.recording, !payload.recording && /не найден|ошиб|не удалось|Vosk/.test(payload.message));
  if (!payload.recording) { recordBook.current.partial = ''; showCurrentRecord(); }
}

function showCurrentRecord() {
  const current = recordBook.current;
  if (transcript.value !== current.text) { transcript.value = current.text; transcriptRevision += 1; }
  transcriptIds.clear();
  for (const id of current.pending) transcriptIds.add(id);
  $('record-number').textContent = `Запись ${current.id}`;
  $('partial').textContent = current.partial;
  latestFinal = current.text;
  compactPreview([current.text, current.partial].filter(Boolean).join(' '));
  transcriptControls();
}

function recordSnapshot(snapshot) {
  if (!recordBook.snapshot(snapshot)) return;
  showCurrentRecord();
  renderDelivery(lastDelivery);
}

async function changeRecord(clear) {
  if (recordTransitionBusy || transcriptBusy) return;
  recordTransitionBusy = true;
  const { id: recordId, generation } = recordBook.current;
  const session = currentSession ?? 0;
  transcriptControls();
  try {
    await claimChain;
    const snapshot = await call('change_record', { clear, recordId, generation, session });
    recordSnapshot(snapshot);
    transcriptFeedback(clear ? 'Текущая запись очищена. Вставленный текст удалите во внешнем поле вручную.' : 'Новая запись готова.');
  } catch (error) { transcriptFeedback(String(error), true); }
  finally { recordTransitionBusy = false; transcriptControls(); }
}
$('restart-record').addEventListener('click', () => changeRecord(true));
$('new-record').addEventListener('click', () => changeRecord(false));

function makeQueueItem(entry, uncertain) {
  const div = document.createElement('div');
  div.className = 'queue-item';
  if (entry.record_id) {
    const label = document.createElement('span');
    label.className = 'queue-record-label';
    label.textContent = `Запись ${entry.record_id}`;
    div.append(label);
  }
  const p = document.createElement('p');
  p.textContent = entry.text;
  const actions = document.createElement('div');
  actions.className = 'queue-actions';
  const copy = document.createElement('button');
  copy.className = 'mini'; copy.textContent = 'Копировать';
  copy.type = 'button';
  copy.addEventListener('click', async () => {
    try {
      if (!uncertain) { await call('take_pending', { ids: entry.ids || [entry.id] }); recordBook.take(entry.ids || [entry.id]); }
      await navigator.clipboard.writeText(entry.text.trim()); copy.textContent = 'Скопировано';
    }
    catch (err) { status(String(err), recording, true); }
  });
  const clear = document.createElement('button');
  clear.className = 'mini danger'; clear.textContent = 'Убрать';
  clear.type = 'button';
  clear.addEventListener('click', async () => {
    try { for (const id of entry.ids || [entry.id]) await call('discard_pending', { id }); }
    catch (error) { status(String(error), recording, true); }
  });
  actions.append(copy, clear);
  div.append(p, actions);
  if (uncertain) div.title = 'Автоматического повтора не будет. Используйте ручное копирование.';
  return div;
}

function groupQueue(entries) {
  const groups = new Map();
  for (const entry of entries) {
    const key = entry.record_id ? `${entry.record_id}:${entry.generation}` : entry.id;
    if (!groups.has(key)) groups.set(key, { ...entry, ids: [], text: '' });
    const group = groups.get(key);
    group.ids.push(entry.id);
    group.text += entry.text;
  }
  return [...groups.values()];
}

function renderRecordHistory() {
  const history = recordBook.history();
  $('record-history').hidden = history.length === 0;
  $('history-items').replaceChildren(...history.map(record => {
    const card = document.createElement('div'); card.className = 'queue-item';
    const label = document.createElement('span'); label.className = 'queue-record-label'; label.textContent = `Запись ${record.id}`;
    const text = document.createElement('p'); text.textContent = record.text;
    const actions = document.createElement('div'); actions.className = 'queue-actions';
    const copy = document.createElement('button'); copy.type = 'button'; copy.className = 'mini'; copy.textContent = 'Копировать';
    copy.addEventListener('click', async () => {
      try {
        const ids = record.fragments.map(part => part.id);
        await call('take_pending', { ids }); recordBook.take(ids);
        await navigator.clipboard.writeText(record.text); copy.textContent = 'Скопировано';
      } catch (error) { status(String(error), recording, true); }
    });
    actions.append(copy); card.append(label, text, actions); return card;
  }));
  return history.length;
}

function renderDelivery(state) {
  lastDelivery = state;
  const pending = groupQueue(state.pending);
  const uncertain = groupQueue(state.uncertain);
  const total = new Set([...pending, ...uncertain].map(entry => entry.record_id ? `${entry.record_id}:${entry.generation}` : entry.id)).size;
  $('queue-badge').textContent = String(total);
  $('queue-badge').hidden = total === 0;
  const historyCount = renderRecordHistory();
  $('queue-empty').hidden = total !== 0 || historyCount > 0;
  $('uncertain-heading').hidden = state.uncertain.length === 0;
  $('queue-size').textContent = String(pending.length);
  $('queue-note').textContent = state.pending.length ?
    'Ожидает фокуса или наведения на редактируемое поле:' :
    'Нет ожидающих фрагментов.';
  $('queue-items').replaceChildren(...pending.map(p => makeQueueItem(p, false)));
  $('uncertain-items').replaceChildren(...uncertain.map(p => makeQueueItem(p, true)));
  $('delivery-message').textContent = state.last_message;
}

transcript.addEventListener('input', async () => {
  if (recording || transcriptBusy || recordTransitionBusy) return;
  transcriptRevision += 1;
  const draft = recordBook.edit(transcript.value);
  compactPreview(draft.text);
  transcriptControls();
  try {
    await persistDraft(draft);
    transcriptFeedback('Текст для ручной вставки. Нажмите «Копировать», когда закончите правку.');
  } catch (err) {
    transcriptFeedback(`Не удалось снять исходный текст с автовставки: ${err}. Повторите через «Копировать».`, true);
  }
});

$('copy-transcript').addEventListener('click', async () => {
  if (recording || transcriptBusy || !transcript.value.trim()) return;
  const text = transcript.value;
  const revision = transcriptRevision;
  const entry = recordBook.current;
  const ids = [...transcriptIds];
  const draft = entry.dirty ? { recordId: entry.id, generation: entry.generation, text, baseIds: entry.fragments.map(part => part.id), revision: entry.revision } : null;
  transcriptBusy = true;
  transcriptControls();
  try {
    if (draft) await persistDraft(draft);
    await takeTranscript(ids);
    await navigator.clipboard.writeText(text);
    transcriptFeedback(revision === transcriptRevision ?
      'Скопировано. Вставьте исправленный текст вручную.' :
      'Скопирован текст на момент нажатия. Новая диктовка остаётся в редакторе.');
  } catch (err) {
    transcriptFeedback(`Не удалось скопировать: ${err}. Текст остаётся в редакторе.`, true);
  } finally {
    transcriptBusy = false;
    transcriptControls();
  }
});

$('clear-transcript').addEventListener('click', async () => {
  if (recording) return;
  await changeRecord(true);
});

$('record').addEventListener('click', async () => {
  try {
    if (recording) {
      status('Останавливаем микрофон…', true);
      await call('end_recording');
    } else {
      status('Загружаем модель…', true);
      await call('begin_recording');
    }
  } catch (err) { status(String(err), false, true); }
});
$('refresh-models').addEventListener('click', () => loadModels().catch(e => status(String(e), recording, true)));
model.addEventListener('change', () => selectModel(model.value).catch(e => status(String(e), recording, true)));
$('apply-path').addEventListener('click', async () => {
  try {
    const path = $('custom-path').value.trim();
    await selectModel(path);
    if (![...model.options].some(o => o.value === path)) model.add(new Option('Пользовательская модель', path));
    model.value = path;
    status('Локальная модель выбрана');
  }
  catch (e) { status(String(e), recording, true); }
});
let recordOptions = { enabled: false, silence_seconds: 3 };
let recordOptionsBusy = false;
function recordOptionControls() {
  $('auto-new-record').disabled = recordOptionsBusy;
  $('silence-seconds').disabled = recordOptionsBusy || !$('auto-new-record').checked;
}
async function saveRecordOptions() {
  if (recordOptionsBusy) return;
  const seconds = Number($('silence-seconds').value);
  if (!Number.isInteger(seconds) || seconds < 1 || seconds > 60) {
    $('silence-seconds').setAttribute('aria-invalid', 'true');
    $('record-options-feedback').textContent = 'Укажите целое число от 1 до 60 секунд.';
    return;
  }
  recordOptionsBusy = true; recordOptionControls();
  try {
    const enabled = $('auto-new-record').checked;
    await call('set_record_options', { enabled, silenceSeconds: seconds });
    recordOptions = { enabled, silence_seconds: seconds };
    localStorage.setItem('auto-new-record', String(enabled));
    localStorage.setItem('silence-seconds', String(seconds));
    $('silence-seconds').setAttribute('aria-invalid', 'false');
    $('record-options-feedback').textContent = enabled ? `После ${seconds} с тишины следующая речь начнёт новую запись.` : 'Паузы любой длины остаются в одной записи.';
  } catch (error) {
    $('auto-new-record').checked = recordOptions.enabled;
    $('silence-seconds').value = String(recordOptions.silence_seconds);
    $('record-options-feedback').textContent = String(error);
  } finally { recordOptionsBusy = false; recordOptionControls(); }
}
$('auto-new-record').addEventListener('change', saveRecordOptions);
$('silence-seconds').addEventListener('change', saveRecordOptions);
$('hover-insert').addEventListener('change', async (ev) => {
  localStorage.setItem('hover-insert', String(ev.target.checked));
  await call('set_hover_delivery', { enabled: ev.target.checked });
});

async function init() {
  if (!runtime) { status('Откройте Voice Input как Windows-приложение', false, true); return; }
  await updates.init();
  $('auto-new-record').checked = localStorage.getItem('auto-new-record') === 'true';
  const silence = Number(localStorage.getItem('silence-seconds') || '3');
  $('silence-seconds').value = String(Number.isInteger(silence) && silence >= 1 && silence <= 60 ? silence : 3);
  $('hover-insert').checked = localStorage.getItem('hover-insert') !== 'false';
  await saveRecordOptions();
  await call('set_hover_delivery', { enabled: $('hover-insert').checked });
  await listen('recording-status', ({payload}) => {
    recordingStatus(payload);
  });
  await listen('transcription-partial', ({payload}) => {
    if (!acceptSession(payload.session)) return;
    if (recordBook.partial(payload)) showCurrentRecord();
  });
  await listen('transcription-final', ({payload}) => {
    if (!acceptSession(payload.session)) return;
    appendTranscript(payload);
    $('partial').textContent = '';
  });
  await listen('delivery-status', ({payload}) => renderDelivery(payload));
  await listen('audio-levels', ({payload}) => {
    if (recording && payload.session === currentSession) wave.update(payload);
  });
  await listen('hotkey-status', ({payload}) => renderHotkey(payload));
  await listen('hotkey-captured', ({payload}) => { if (hotkeyCapturing || hotkeyCaptureStarting) stopHotkeyCapture(payload); });
  await listen('record-changed', ({payload}) => recordSnapshot(payload));
  recordSnapshot(await call('record_snapshot'));
  renderHotkey(await call('hotkey_status'));
  renderDelivery(await call('delivery_status'));
  const previous = await call('recording_status');
  recordingStatus(previous);
  await loadModels();
}
init().catch(err => status(String(err), false, true));
