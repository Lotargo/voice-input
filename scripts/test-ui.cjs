const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const vm = require('node:vm');

async function openApp(commands = {}, clipboardWrite, options = {}) {
  const elements = new Map();
  const events = new Map();
  const calls = [];
  const clipboard = [];
  const storageWrites = [];
  const windowCalls = [];
  let focusedId = null;
  let now = 100;
  let nextAnimation = 1;
  const frames = new Map();
  const timers = new Map();
  const documentEvents = new Map();
  const windowEvents = new Map();
  let nativeRecords = { version: 0, current: { id: 1, generation: 0, text: '', partial: '', fragments: [] }, history: [] };
  const clone = value => JSON.parse(JSON.stringify(value));
  const addEvent = (events, name, callback) => events.set(name, [...(events.get(name) || []), callback]);
  class Element {
    constructor() {
      this.textContent = '';
      this.value = '';
      this.options = [];
      this.handlers = new Map();
      this.attributes = new Map();
      this.style = {};
      this.scrollTop = 0; this.scrollHeight = 0; this.clientHeight = 0;
      this.classList = { toggle() {} };
    }
    addEventListener(name, callback) { this.handlers.set(name, callback); }
    focus() { focusedId = this.id; }
    setAttribute(name, value) { this.attributes.set(name, value); }
    removeAttribute(name) { this.attributes.delete(name); }
    getAttribute(name) { return this.attributes.get(name); }
    replaceChildren(...children) { this.children = children; }
    append(...children) { this.children = [...(this.children || []), ...children]; }
    add(option) {
      this.options.push(option);
      if (this.options.length === 1) this.value = option.value;
    }
  }
  const element = (id) => {
    if (!elements.has(id)) { const entry = new Element(); entry.id = id; elements.set(id, entry); }
    return elements.get(id);
  };
  const context = {
    document: { getElementById: element, createElement: () => new Element(), hidden: false, addEventListener: (name, callback) => addEvent(documentEvents, name, callback) },
    performance: { now: () => now },
    localStorage: { getItem: () => null, setItem: (...args) => storageWrites.push(args) },
    navigator: { clipboard: { writeText: async (text) => {
      if (clipboardWrite) await clipboardWrite(text);
      clipboard.push(text);
    } } },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    window: {
      addEventListener: (name, callback) => addEvent(windowEvents, name, callback),
      matchMedia: () => ({ matches: !!options.reducedMotion, addEventListener() {} }),
      requestAnimationFrame: callback => { const id = nextAnimation++; frames.set(id, callback); return id; },
      cancelAnimationFrame: id => frames.delete(id),
      setTimeout: (callback, delay) => { const id = nextAnimation++; timers.set(id, { callback, at: now + delay }); return id; },
      clearTimeout: id => timers.delete(id),
      __TAURI__: {
      window: { getCurrentWindow: () => ({
        minimize: async () => windowCalls.push('minimize'),
        toggleMaximize: async () => windowCalls.push('toggleMaximize'),
        close: async () => windowCalls.push('close'),
      }) },
      core: { invoke: async (name, args) => {
        calls.push({ name, args });
        if (commands[name]) { const value = await commands[name](args); if (value !== undefined) return value; }
        if (name === 'available_models') return [{ name: 'test', path: 'C:\\test' }];
        if (name === 'recording_status') return { session: 0, recording: false, message: 'Ожидание' };
        if (name === 'update_info') return { version: '0.2.0', mode: 'portable', supported: true, last_result: null };
        if (name === 'input_settings' || name === 'set_input_settings') return { options: { device_id: args?.deviceId ?? null, device_name: null, denoise: args?.denoise ?? false }, devices: [{ id: 'wasapi:test', name: 'Микрофон по умолчанию', is_default: true }], name: 'Микрофон по умолчанию', available: true, error: null };
        if (name === 'delivery_status') return { pending: [], uncertain: [], last_message: '' };
        if (name === 'hotkey_status') return { shortcut: 'Ctrl+Alt+Space', default_shortcut: 'Ctrl+Alt+Space', active: true, error: null };
        if (name === 'set_hotkey') return { shortcut: args.shortcut, default_shortcut: 'Ctrl+Alt+Space', active: true, error: null };
        if (name === 'record_snapshot') return clone(nativeRecords);
        if (name === 'edit_record') {
          const record = nativeRecords.current.id === args.recordId ? nativeRecords.current : nativeRecords.history.find(entry => entry.id === args.recordId);
          if (!record || record.generation !== args.generation) throw new Error('Запись изменилась');
          record.text = args.text;
          for (const part of record.fragments) if (!args.baseIds.includes(part.id)) record.text += (record.text && !record.text.endsWith('\n') ? '\n' : '') + part.text;
          return clone(record);
        }
        if (name === 'change_record') {
          if (args.recordId !== nativeRecords.current.id || args.generation !== nativeRecords.current.generation) throw new Error('Запись изменилась');
          nativeRecords.version++;
          if (args.clear) nativeRecords.current = { id: args.recordId, generation: args.generation + 1, text: '', partial: '', fragments: [] };
          else {
            if (nativeRecords.current.text) nativeRecords.history.push(nativeRecords.current);
            nativeRecords.current = { id: args.recordId + 1, generation: 0, text: '', partial: '', fragments: [] };
          }
          return clone(nativeRecords);
        }
      } },
      event: { listen: async (name, callback) => { events.set(name, callback); } },
    } },
  };
  const sandbox = vm.createContext(context);
  vm.runInContext(readFileSync(join(__dirname, '../ui/wave.js'), 'utf8'), sandbox);
  vm.runInContext(readFileSync(join(__dirname, '../ui/records.js'), 'utf8'), sandbox);
  vm.runInContext(readFileSync(join(__dirname, '../ui/updates.js'), 'utf8'), sandbox);
  vm.runInContext(readFileSync(join(__dirname, '../ui/audio.js'), 'utf8'), sandbox);
  vm.runInContext(readFileSync(join(__dirname, '../ui/models.js'), 'utf8'), sandbox);
  vm.runInContext(readFileSync(join(__dirname, '../ui/app.js'), 'utf8'), sandbox);
  await new Promise(setImmediate);
  return {
    element, calls, clipboard, storageWrites, windowCalls,
    focused: () => focusedId,
    pendingFrames: () => frames.size,
    step: (milliseconds = 16) => {
      now += milliseconds;
      for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.callback(); }
      const batch = [...frames.values()]; frames.clear();
      for (const callback of batch) callback(now);
    },
    visibility: (hidden) => { context.document.hidden = hidden; for (const callback of documentEvents.get('visibilitychange') || []) callback(); },
    blur: () => { for (const callback of windowEvents.get('blur') || []) callback(); },
    captureKey: async (details) => {
      let prevented = false;
      for (const callback of documentEvents.get('keydown') || []) callback({ ...details, preventDefault: () => { prevented = true; } });
      await new Promise(setImmediate);
      return prevented;
    },
    emit: (name, payload) => {
      if (name === 'record-changed') nativeRecords = clone(payload);
      if (name === 'transcription-final' && (!payload.record_id || payload.record_id === nativeRecords.current.id) && (payload.generation ?? nativeRecords.current.generation) === nativeRecords.current.generation) {
        if (!payload.id || !nativeRecords.current.fragments.some(part => part.id === payload.id)) {
          nativeRecords.current.text += (nativeRecords.current.text && !nativeRecords.current.text.endsWith('\n') ? '\n' : '') + payload.text.trim();
          if (payload.id) nativeRecords.current.fragments.push({ id: payload.id, text: payload.text.trim() });
        }
      }
      return events.get(name)({ payload });
    },
    click: (id) => element(id).handlers.get('click')(),
    keydown: (id, key) => {
      let prevented = false;
      element(id).handlers.get('keydown')({ key, preventDefault: () => { prevented = true; } });
      return prevented;
    },
    input: (value) => {
      element('transcribed').value = value;
      return element('transcribed').handlers.get('input')();
    },
  };
}

test('audio settings save the device identity and suppression choice', async () => {
  const app = await openApp();
  await app.click('microphone-settings');
  await new Promise(setImmediate);
  assert.equal(app.element('panel-settings').hidden, false);
  assert.equal(app.element('settings-audio').hidden, false);
  app.element('input-device').value = 'wasapi:test';
  app.element('noise-suppression').checked = true;
  await app.element('input-device').handlers.get('change')();
  const call = app.calls.find(call => call.name === 'set_input_settings');
  assert.equal(call.args.deviceId, 'wasapi:test');
  assert.equal(call.args.denoise, true);
  assert.equal(app.element('noise-suppression').checked, true);
  assert.equal(app.calls.some(call => call.name === 'begin_microphone_test' || call.name === 'begin_recording'), false);
});

test('a missing pinned microphone stays selected until the user chooses a replacement', async () => {
  const app = await openApp({ input_settings: () => ({ options: { device_id: 'wasapi:missing', device_name: 'USB mic', denoise: false }, devices: [], name: 'USB mic', available: false }) });
  assert.equal(app.element('input-device').value, 'wasapi:missing');
  assert.equal(app.element('test-input').disabled, true);
  assert.match(app.element('input-device-name').textContent, /Недоступен/);
  assert.equal(app.calls.some(call => call.name === 'set_input_settings'), false);
});

test('a failed audio preference save restores the previous controls', async () => {
  const app = await openApp({ set_input_settings: () => { throw new Error('Read only'); } });
  app.element('noise-suppression').checked = true;
  await app.element('noise-suppression').handlers.get('change')();
  assert.equal(app.element('noise-suppression').checked, false);
  assert.match(app.element('audio-feedback').textContent, /Read only/);
});

test('microphone level check leaves transcripts and delivery ownership untouched', async () => {
  const app = await openApp();
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'Сохранённый текст' });
  await app.click('microphone-settings');
  await new Promise(setImmediate);
  await app.click('test-input');
  app.emit('recording-status', { session: 2, recording: true, testing: true, message: 'Проверка' });
  app.emit('audio-levels', { session: 2, testing: true, raw_rms: .5, rms: .005, input_peak: 1, clipped_ratio: .02, bands: [] });
  assert.equal(app.element('record').disabled, true);
  assert.equal(app.element('noise-suppression').disabled, true);
  assert.match(app.element('input-quality').textContent, /Перегрузка/);
  assert.ok(app.element('input-level').value > app.element('clean-level').value);
  assert.equal(app.element('transcribed').value, 'Сохранённый текст');
  assert.equal(app.calls.some(call => ['begin_recording', 'take_pending', 'edit_record', 'discard_pending'].includes(call.name)), false);
  assert.equal(app.clipboard.length, 0);
  await app.click('settings-tab-dictation');
  assert.ok(app.calls.some(call => call.name === 'end_microphone_test'));
  app.emit('recording-status', { session: 2, recording: false, testing: true, message: 'Проверка завершена' });
  await new Promise(setImmediate);
  assert.equal(app.element('record').disabled, false);
});

test('leaving audio settings during a delayed start cannot leave a microphone check running', async () => {
  let finish;
  const app = await openApp({ begin_microphone_test: () => new Promise(resolve => { finish = resolve; }) });
  await app.click('microphone-settings');
  await new Promise(setImmediate);
  const start = app.click('test-input');
  await app.click('tab-dictation');
  finish(); await start;
  assert.ok(app.calls.some(call => call.name === 'end_microphone_test'));
  assert.equal(app.calls.some(call => call.name === 'begin_recording'), false);
});

test('settings sections support keyboard navigation with one active tab stop', async () => {
  const app = await openApp();
  await app.click('tab-settings');
  assert.equal(app.keydown('settings-tab-audio', 'ArrowRight'), true);
  assert.equal(app.focused(), 'settings-tab-dictation');
  assert.equal(app.element('settings-dictation').hidden, false);
  assert.equal(app.keydown('settings-tab-dictation', 'End'), true);
  assert.equal(app.focused(), 'settings-tab-updates');
  assert.equal(app.element('settings-updates').hidden, false);
  assert.equal(app.element('settings-tab-audio').tabIndex, -1);
});

test('a hardware error restores idle controls while preserving the current record', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'Текст до отключения' });
  app.emit('recording-status', { session: 1, recording: false, message: 'Микрофон отключён' });
  assert.equal(app.element('transcribed').value, 'Текст до отключения');
  assert.equal(app.element('record').getAttribute('aria-pressed'), 'false');
  assert.equal(app.element('input-device').disabled, false);
  assert.equal(app.calls.some(call => call.name === 'begin_recording'), false);
});

test('startup reads local update info without checking the network', async () => {
  const app = await openApp();
  assert.equal(app.element('app-version').textContent, '0.2.0 · Portable');
  assert.equal(app.calls.some(call => call.name === 'check_updates'), false);
  assert.equal(app.calls.some(call => call.name === 'install_update'), false);
});

test('a checked update requires a separate install click and waits for dictation to stop', async () => {
  const app = await openApp({ check_updates: async () => ({ version: '0.2.1', notes: 'Исправления' }) });
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  await app.click('check-updates');
  assert.equal(app.element('install-update').hidden, false);
  assert.equal(app.element('install-update').disabled, true);
  assert.equal(app.element('update-stop-note').hidden, false);
  assert.equal(app.element('update-restart-note').hidden, false);
  assert.equal(app.element('update-notes').textContent, 'Исправления');
  await app.click('install-update');
  assert.equal(app.calls.some(call => call.name === 'install_update'), false);
  app.emit('recording-status', { session: 1, recording: false, message: 'Стоп' });
  await app.click('install-update');
  const install = app.calls.find(call => call.name === 'install_update');
  assert.equal(install.args.version, '0.2.1');
  assert.equal(app.element('record').disabled, true);
});

test('a failed check clears the old offer and an installation failure leaves the app usable', async () => {
  let failCheck = false;
  const app = await openApp({
    check_updates: async () => { if (failCheck) throw new Error('Offline'); return { version: '0.2.1' }; },
    install_update: async () => { throw new Error('Invalid signature'); },
  });
  await app.click('check-updates');
  await app.click('install-update');
  assert.match(app.element('update-feedback').textContent, /Invalid signature/);
  assert.equal(app.element('record').disabled, false);
  assert.equal(app.element('install-update').disabled, false);
  failCheck = true;
  await app.click('check-updates');
  assert.match(app.element('update-feedback').textContent, /Offline/);
  assert.equal(app.element('install-update').hidden, true);
});

test('update progress is bounded and a repeated install cannot launch a second request', async () => {
  let complete;
  const app = await openApp({
    check_updates: async () => ({ version: '0.2.1' }),
    install_update: () => new Promise(resolve => { complete = resolve; }),
  });
  await app.click('check-updates');
  const installing = app.click('install-update');
  await app.click('install-update');
  app.emit('update-progress', { phase: 'downloading', downloaded: 300, total: 200 });
  assert.equal(app.element('update-download').value, 1);
  app.emit('update-progress', { phase: 'installing', downloaded: 300, total: 300 });
  assert.match(app.element('update-feedback').textContent, /Подпись проверена/);
  assert.equal(app.calls.filter(call => call.name === 'install_update').length, 1);
  complete(); await installing;
});

test('pauses start a new line within the same record in compact preview and the editor', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 1, id: '1:1', record_id: 1, generation: 0, version: 0, text: 'первая часть' });
  app.emit('transcription-partial', { session: 1, record_id: 1, generation: 0, version: 0, text: '' });
  app.step(10000);
  app.emit('transcription-partial', { session: 1, record_id: 1, generation: 0, version: 0, text: 'после паузы' });
  assert.equal(app.element('compact-preview').textContent, 'первая часть\nпосле паузы');
  app.emit('transcription-final', { session: 1, id: '1:2', record_id: 1, generation: 0, version: 0, text: 'после паузы' });
  assert.equal(app.element('transcribed').value, 'первая часть\nпосле паузы');
  assert.equal(app.element('record-number').textContent, 'Запись 1');
  assert.equal(app.calls.some(call => call.name === 'change_record'), false);
});

test('right button starts a fresh record while retaining history and microphone state', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 1, id: '1:1', record_id: 1, generation: 0, version: 0, text: 'первое сообщение' });
  await app.click('new-record');
  assert.equal(app.element('record').getAttribute('aria-pressed'), 'true');
  assert.equal(app.element('record-number').textContent, 'Запись 2');
  assert.equal(app.element('transcribed').value, '');
  assert.equal(app.element('history-items').children[0].children[1].textContent, 'первое сообщение');
  app.emit('transcription-final', { session: 1, id: '1:2', record_id: 2, generation: 0, version: 1, text: 'второе сообщение' });
  assert.equal(app.element('compact-preview').textContent, 'второе сообщение');
  assert.equal(app.calls.some(call => ['begin_recording', 'end_recording'].includes(call.name)), false);
});

test('left button clears during dictation and rejects late hypotheses from the old generation', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 1, id: '1:1', record_id: 1, generation: 0, version: 0, text: 'передумал' });
  await app.click('restart-record');
  app.emit('transcription-partial', { session: 1, record_id: 1, generation: 0, version: 0, text: 'старая гипотеза' });
  app.emit('transcription-final', { session: 1, id: '1:2', record_id: 1, generation: 0, version: 0, text: 'старый хвост' });
  assert.equal(app.element('transcribed').value, '');
  app.emit('transcription-final', { session: 1, id: '1:3', record_id: 1, generation: 1, version: 1, text: 'новый вариант' });
  assert.equal(app.element('transcribed').value, 'новый вариант');
  assert.equal(app.element('record-number').textContent, 'Запись 1');
  assert.equal(app.element('record').getAttribute('aria-pressed'), 'true');
  assert.equal(app.calls.some(call => ['begin_recording', 'end_recording'].includes(call.name)), false);
  assert.deepEqual(app.clipboard, []);
});

test('a stale native snapshot cannot erase final segments received after it was captured', async () => {
  const app = await openApp();
  const first = { id: 1, generation: 0, text: 'первая', partial: '', fragments: [{ id: '1:1', text: 'первая' }] };
  app.emit('record-changed', { version: 0, current: first, history: [] });
  app.emit('transcription-final', { session: 1, id: '1:2', record_id: 1, generation: 0, version: 0, text: 'вторая' });
  app.emit('record-changed', { version: 0, current: first, history: [] });
  assert.equal(app.element('transcribed').value, 'первая\nвторая');
});

test('queue groups endpoint fragments as one record without losing segment ownership', async () => {
  const app = await openApp();
  app.emit('delivery-status', { pending: [
    { id: '1:1', record_id: 1, generation: 0, text: 'первая' },
    { id: '1:2', record_id: 1, generation: 0, text: '\nвторая' },
  ], uncertain: [], last_message: '' });
  assert.equal(app.element('queue-badge').textContent, '1');
  assert.equal(app.element('queue-items').children.length, 1);
  const card = app.element('queue-items').children[0];
  assert.equal(card.children[1].textContent, 'первая\nвторая');
  await card.children[2].children[0].handlers.get('click')();
  assert.deepEqual(Array.from(app.calls.filter(call => call.name === 'take_pending').at(-1).args.ids), ['1:1', '1:2']);
  assert.deepEqual(app.clipboard, ['первая\nвторая']);
});

test('automatic split settings default to manual mode and validate 1 through 60 seconds', async () => {
  const app = await openApp();
  assert.equal(app.element('auto-new-record').checked, false);
  assert.equal(app.element('silence-seconds').value, '3');
  assert.equal(app.element('silence-seconds').disabled, true);
  app.element('auto-new-record').checked = true;
  await app.element('auto-new-record').handlers.get('change')();
  const before = app.calls.filter(call => call.name === 'set_record_options').length;
  for (const value of ['0', '61', '2.5', '']) {
    app.element('silence-seconds').value = value;
    await app.element('silence-seconds').handlers.get('change')();
    assert.equal(app.element('silence-seconds').getAttribute('aria-invalid'), 'true');
  }
  assert.equal(app.calls.filter(call => call.name === 'set_record_options').length, before);
  for (const value of ['1', '60']) {
    app.element('silence-seconds').value = value;
    await app.element('silence-seconds').handlers.get('change')();
    assert.equal(app.calls.filter(call => call.name === 'set_record_options').at(-1).args.silenceSeconds, Number(value));
  }
  assert.deepEqual(app.storageWrites.at(-1), ['silence-seconds', '60']);
  assert.equal(app.calls.some(call => call.name === 'set_auto_stop'), false);
});

test('saved native shortcut updates settings, recording hint and footer', async () => {
  const app = await openApp({ hotkey_status: () => ({ shortcut: 'Ctrl+Shift+F9', default_shortcut: 'Ctrl+Alt+Space', active: true }) });
  assert.equal(app.element('hotkey-input').value, 'Ctrl+Shift+F9');
  assert.equal(app.element('shortcut-label').textContent, 'Ctrl + Shift + F9');
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  assert.match(app.element('footer-status').textContent, /Ctrl \+ Shift \+ F9/);
});

test('physical key capture works with a Russian layout and applies only on explicit save', async () => {
  const app = await openApp();
  app.click('tab-settings');
  await app.click('capture-hotkey');
  assert.equal(app.element('hotkey-input').readOnly, true);
  assert.equal(await app.captureKey({ key: 'К', code: 'KeyR', ctrlKey: true, shiftKey: true }), true);
  assert.equal(app.element('hotkey-input').value, 'Ctrl+Shift+R');
  assert.equal(app.element('hotkey-input').readOnly, false);
  assert.equal(app.calls.some(call => call.name === 'set_hotkey'), false);
  assert.equal(app.calls.some(call => call.name === 'begin_recording'), false);
  await app.click('apply-hotkey');
  assert.equal(app.element('shortcut-label').textContent, 'Ctrl + Shift + R');
  assert.equal(app.calls.filter(call => call.name === 'set_hotkey').at(-1).args.shortcut, 'Ctrl+Shift+R');
  assert.deepEqual(app.calls.filter(call => call.name === 'set_hotkey_capture').map(call => call.args.enabled), [true, false]);
});

test('capturing the currently registered global shortcut consumes it without dictation', async () => {
  const app = await openApp();
  app.click('tab-settings');
  await app.click('capture-hotkey');
  app.emit('hotkey-captured', 'Ctrl+Alt+Space');
  await new Promise(setImmediate);
  assert.equal(app.element('hotkey-input').value, 'Ctrl+Alt+Space');
  assert.equal(app.element('capture-hotkey').attributes.get('aria-pressed'), 'false');
  assert.equal(app.calls.some(call => ['begin_recording', 'end_recording', 'set_hotkey'].includes(call.name)), false);
});

test('failed rebind retains active shortcut and reset explicitly saves the default', async () => {
  let blocked = true;
  const app = await openApp({ set_hotkey: ({ shortcut }) => {
    if (blocked) throw new Error('Сочетание занято');
    return { shortcut, default_shortcut: 'Ctrl+Alt+Space', active: true };
  } });
  app.element('hotkey-input').value = 'Ctrl+Shift+F9';
  await app.click('apply-hotkey');
  assert.equal(app.element('hotkey-input').value, 'Ctrl+Shift+F9');
  assert.equal(app.element('hotkey-input').getAttribute('aria-invalid'), 'true');
  assert.match(app.element('hotkey-feedback').textContent, /занято/);
  assert.equal(app.element('shortcut-label').textContent, 'Ctrl + Alt + Space');
  blocked = false;
  await app.click('reset-hotkey');
  assert.equal(app.element('hotkey-input').value, 'Ctrl+Alt+Space');
  assert.equal(app.element('hotkey-input').getAttribute('aria-invalid'), 'false');
});

test('escape, tab changes, window blur and hiding all release shortcut capture', async () => {
  for (const cancel of [app => app.captureKey({ code: 'Escape' }), app => app.click('tab-dictation'), app => app.blur(), app => app.visibility(true)]) {
    const app = await openApp();
    app.click('tab-settings');
    await app.click('capture-hotkey');
    await cancel(app);
    await new Promise(setImmediate);
    assert.equal(app.element('capture-hotkey').getAttribute('aria-pressed'), 'false');
    assert.equal(app.element('hotkey-input').readOnly, false);
    assert.equal(app.calls.filter(call => call.name === 'set_hotkey_capture').at(-1).args.enabled, false);
    assert.equal(app.calls.some(call => call.name === 'set_hotkey'), false);
  }
});

test('leaving settings during a delayed capture request cannot leave capture armed', async () => {
  let release;
  const app = await openApp({ set_hotkey_capture: ({ enabled }) => enabled ? new Promise(resolve => { release = resolve; }) : undefined });
  app.click('tab-settings');
  const starting = app.click('capture-hotkey');
  app.click('tab-dictation');
  release();
  await starting;
  assert.equal(app.element('capture-hotkey').getAttribute('aria-pressed'), 'false');
  assert.equal(app.calls.filter(call => call.name === 'set_hotkey_capture').at(-1).args.enabled, false);
  assert.equal(app.calls.some(call => call.name === 'set_hotkey'), false);
});

test('a startup shortcut conflict leaves dictation and settings usable', async () => {
  const app = await openApp({ hotkey_status: () => ({ shortcut: 'Ctrl+Alt+Space', default_shortcut: 'Ctrl+Alt+Space', active: false, error: 'Сочетание недоступно' }) });
  assert.match(app.element('shortcut-label').textContent, /недоступно/);
  assert.equal(app.element('hotkey-active').textContent, 'Не активно');
  assert.equal(app.calls.some(call => call.name === 'select_model'), true);
  await app.click('record');
  assert.equal(app.calls.some(call => call.name === 'begin_recording'), true);
});

test('a second hotkey session appends to the retained transcript', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 1, text: 'первая фраза' });
  app.emit('recording-status', { session: 1, recording: false, message: 'Стоп' });
  app.emit('recording-status', { session: 2, recording: true, message: 'Запись' });
  app.emit('transcription-partial', { session: 2, text: 'вторая' });
  assert.equal(app.element('partial').textContent, 'вторая');
  app.emit('transcription-final', { session: 2, text: 'вторая фраза' });
  assert.equal(app.element('transcribed').value, 'первая фраза\nвторая фраза');
});

test('late events from an earlier session cannot overwrite the active session', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 2, recording: true, message: 'Новая запись' });
  app.emit('transcription-final', { session: 2, text: 'актуальный текст' });
  app.emit('transcription-partial', { session: 1, text: 'устаревшее' });
  app.emit('transcription-final', { session: 1, text: 'устаревшее' });
  app.emit('recording-status', { session: 1, recording: false, message: 'Старый стоп' });
  assert.equal(app.element('transcribed').value, 'актуальный текст');
  assert.equal(app.element('partial').textContent, '');
  assert.equal(app.element('status-label').textContent, 'Новая запись');
  assert.equal(app.element('record-label').textContent, 'Закончить диктовку');
});

test('identical final chunks with distinct segments remain in the preview', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'да' });
  app.emit('transcription-final', { session: 1, id: '1:2', text: 'да' });
  assert.equal(app.element('transcribed').value, 'да\nда');
});

test('partial hypotheses stay in the preview without delivery commands', async () => {
  const app = await openApp();
  const before = app.calls.length;
  app.emit('transcription-partial', { session: 1, text: 'только предположение' });
  assert.equal(app.element('partial').textContent, 'только предположение');
  assert.equal(app.calls.length, before);
  assert.equal(app.element('transcribed').value, '');
  assert.deepEqual(app.clipboard, []);
});

test('manual corrections survive later dictation and take pending originals out of auto delivery', async () => {
  const app = await openApp();
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'ошипка' });
  await app.input('Ошибка исправлена.');
  const handover = app.calls.find(call => call.name === 'edit_record');
  assert.deepEqual(Array.from(handover.args.baseIds), ['1:1']);
  assert.deepEqual(app.clipboard, []);
  app.emit('recording-status', { session: 2, recording: true, message: 'Запись' });
  assert.equal(app.element('transcribed').readOnly, true);
  assert.equal(app.element('copy-transcript').disabled, true);
  app.emit('transcription-final', { session: 2, id: '2:1', text: 'продолжение' });
  app.emit('recording-status', { session: 2, recording: false, message: 'Стоп' });
  assert.equal(app.element('transcribed').value, 'Ошибка исправлена.\nпродолжение');
  assert.equal(app.element('transcribed').readOnly, false);
  assert.deepEqual(app.storageWrites.filter(([key]) => /transcript|draft/.test(key)), []);
});

test('Copy transfers pending originals before writing the corrected text to the clipboard', async () => {
  const order = [];
  const app = await openApp({ edit_record: () => { order.push('take'); } }, () => order.push('copy'));
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'исходный текст' });
  await app.input('Исправленный\nтекст.');
  await app.click('copy-transcript');
  assert.deepEqual(order, ['take', 'copy']);
  assert.deepEqual(app.clipboard, ['Исправленный\nтекст.']);
  assert.equal(app.element('transcribed').value, 'Исправленный\nтекст.');
});

test('failed manual handover preserves the draft and does not write the clipboard', async () => {
  let fail = true;
  const app = await openApp({ edit_record: () => { if (fail) throw new Error('IPC failed'); } });
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'исходный' });
  await app.input('Исправленный');
  await app.click('copy-transcript');
  assert.deepEqual(app.clipboard, []);
  assert.equal(app.element('transcribed').value, 'Исправленный');
  fail = false;
  await app.click('copy-transcript');
  assert.deepEqual(app.clipboard, ['Исправленный']);
  assert.equal(app.calls.filter(call => call.name === 'edit_record').length, 3);
});

test('clipboard failure keeps the draft available for an explicit retry', async () => {
  let fail = true;
  const app = await openApp({}, () => { if (fail) throw new Error('Clipboard denied'); });
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'текст' });
  await app.click('copy-transcript');
  assert.equal(app.element('transcribed').value, 'текст');
  assert.deepEqual(app.clipboard, []);
  fail = false;
  await app.click('copy-transcript');
  assert.deepEqual(app.clipboard, ['текст']);
  assert.equal(app.calls.filter(call => call.name === 'take_pending').length, 1);
});

test('Clear requests the current record generation and never changes the clipboard', async () => {
  const app = await openApp();
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'текст' });
  await app.click('clear-transcript');
  assert.equal(app.element('transcribed').value, '');
  assert.equal(app.element('copy-transcript').disabled, true);
  assert.deepEqual(app.clipboard, []);
  const clear = app.calls.find(call => call.name === 'change_record');
  assert.equal(clear.args.recordId, 1);
  assert.equal(clear.args.generation, 0);
  assert.equal(clear.args.clear, true);
});

test('editor Clear stays enabled while recording, discards partials and keeps the microphone running', async () => {
  let release;
  const app = await openApp({ change_record: () => new Promise(resolve => { release = resolve; }) });
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  // Even an unfinished phrase can be discarded before the first final result.
  app.emit('transcription-partial', { session: 1, record_id: 1, generation: 0, version: 0, text: 'передумал' });
  assert.equal(app.element('clear-transcript').disabled, false);
  const clearing = app.click('clear-transcript');
  await new Promise(setImmediate);
  assert.equal(app.element('clear-transcript').disabled, true);
  assert.equal(app.element('restart-record').disabled, true);
  assert.equal(app.element('transcribed').readOnly, true);
  const cleared = { version: 1, current: { id: 1, generation: 1, text: '', partial: '', fragments: [] }, history: [] };
  app.emit('record-changed', cleared);
  app.emit('transcription-partial', { session: 1, record_id: 1, generation: 0, version: 0, text: 'старое' });
  app.emit('transcription-final', { session: 1, id: '1:1', record_id: 1, generation: 0, version: 0, text: 'старый хвост' });
  assert.equal(app.element('transcribed').value, '');
  assert.equal(app.element('partial').textContent, '');
  app.emit('transcription-final', { session: 1, id: '1:2', record_id: 1, generation: 1, version: 1, text: 'новая мысль' });
  release(cleared);
  await clearing;
  assert.equal(app.element('transcribed').value, 'новая мысль');
  assert.equal(app.element('clear-transcript').disabled, false);
  assert.equal(app.element('copy-transcript').disabled, true);
  assert.equal(app.element('record').getAttribute('aria-pressed'), 'true');
  assert.equal(app.calls.some(call => ['begin_recording', 'end_recording', 'discard_pending'].includes(call.name)), false);
  assert.deepEqual(app.clipboard, []);
});

test('failed editor Clear leaves recording and text available for an explicit retry', async () => {
  const app = await openApp({ change_record: () => { throw new Error('IPC failed'); } });
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'сохранить' });
  await app.click('clear-transcript');
  assert.equal(app.element('transcribed').value, 'сохранить');
  assert.equal(app.element('clear-transcript').disabled, false);
  assert.equal(app.element('record').getAttribute('aria-pressed'), 'true');
  assert.match(app.element('transcript-feedback').textContent, /IPC failed/);
  assert.deepEqual(app.clipboard, []);
});

test('manual line breaks survive snapshots and resumed phrases without empty extra lines', async () => {
  const app = await openApp();
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'я тут подумал' });
  await app.input('Я тут подумал\n');
  app.emit('recording-status', { session: 2, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 2, id: '2:1', text: 'надо бы сходить прогуляться' });
  await app.click('new-record');
  assert.equal(app.element('history-items').children[0].children[1].textContent, 'Я тут подумал\nнадо бы сходить прогуляться');
});

test('a delayed Clear cannot erase a newer recording', async () => {
  let release;
  const app = await openApp({ change_record: () => new Promise(resolve => { release = resolve; }) });
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'старый' });
  const clearing = app.click('clear-transcript');
  await new Promise(setImmediate);
  const cleared = { version: 1, current: { id: 1, generation: 1, text: '', partial: '', fragments: [] }, history: [] };
  app.emit('record-changed', cleared);
  app.emit('recording-status', { session: 2, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 2, id: '2:1', record_id: 1, generation: 1, version: 1, text: 'новый' });
  release(cleared);
  await clearing;
  assert.equal(app.element('transcribed').value, 'новый');
});

test('repeated final event IDs do not restore cleared text', async () => {
  const app = await openApp();
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'текст' });
  await app.click('clear-transcript');
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'текст' });
  assert.equal(app.element('transcribed').value, '');
});

test('start and stop command replies cannot overwrite a completed session status', async () => {
  let app;
  app = await openApp({
    begin_recording: () => {
      app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
      app.emit('recording-status', { session: 1, recording: false, message: 'Микрофон не найден' });
    },
    end_recording: () => {
      app.emit('recording-status', { session: 2, recording: false, message: 'Запись завершена' });
    },
  });
  await app.click('record');
  assert.equal(app.element('record-label').textContent, 'Начать диктовку');
  assert.equal(app.element('status-label').textContent, 'Микрофон не найден');
  app.emit('recording-status', { session: 2, recording: true, message: 'Запись' });
  await app.click('record');
  assert.equal(app.element('record-label').textContent, 'Начать диктовку');
  assert.equal(app.element('status-label').textContent, 'Запись завершена');
});

test('model controls stay disabled for the entire recording session', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  for (const id of ['model', 'refresh-models', 'custom-path', 'apply-path']) {
    assert.equal(app.element(id).disabled, true);
  }
  app.emit('recording-status', { session: 1, recording: false, message: 'Стоп' });
  for (const id of ['model', 'refresh-models', 'custom-path', 'apply-path']) {
    assert.equal(app.element(id).disabled, false);
  }
});

test('switching tabs preserves the corrected draft and recording state', async () => {
  const app = await openApp();
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'исходный текст' });
  await app.input('Исправленный текст');
  app.emit('recording-status', { session: 2, recording: true, message: 'Запись идёт' });
  const before = app.calls.length;
  await app.click('tab-model');
  assert.equal(app.element('panel-model').hidden, false);
  assert.equal(app.element('panel-dictation').hidden, true);
  assert.equal(app.element('model-lock').hidden, false);
  await app.click('tab-settings');
  await app.click('tab-dictation');
  assert.equal(app.element('transcribed').value, 'Исправленный текст');
  assert.equal(app.element('transcribed').readOnly, true);
  assert.equal(app.element('record').getAttribute('aria-pressed'), 'true');
  assert.ok(app.calls.slice(before).every(call => ['input_settings', 'model_catalog'].includes(call.name)));
});

test('tabs support arrow keys, Home and End with one active tab stop', async () => {
  const app = await openApp();
  assert.equal(app.keydown('tab-dictation', 'ArrowRight'), true);
  assert.equal(app.focused(), 'tab-model');
  assert.equal(app.element('tab-model').getAttribute('aria-selected'), 'true');
  assert.equal(app.element('tab-dictation').tabIndex, -1);
  app.keydown('tab-model', 'End');
  assert.equal(app.focused(), 'tab-queue');
  app.keydown('tab-queue', 'ArrowRight');
  assert.equal(app.focused(), 'tab-dictation');
  app.keydown('tab-dictation', 'ArrowLeft');
  assert.equal(app.focused(), 'tab-queue');
  app.keydown('tab-queue', 'Home');
  assert.equal(app.focused(), 'tab-dictation');
  assert.equal(app.keydown('tab-dictation', 'Escape'), false);
});

test('queue tab badge includes uncertain attempts without enabling retries', async () => {
  const app = await openApp();
  const before = app.calls.length;
  app.emit('delivery-status', { pending: [], uncertain: [{ id: '1:1', text: 'текст' }], last_message: 'Автоповтора не будет' });
  assert.equal(app.element('queue-badge').textContent, '1');
  assert.equal(app.element('queue-badge').hidden, false);
  assert.equal(app.element('queue-empty').hidden, true);
  assert.equal(app.element('uncertain-heading').hidden, false);
  assert.equal(app.calls.length, before);
});

test('custom caption buttons call the native window actions', async () => {
  const app = await openApp();
  await app.click('window-minimize');
  await app.click('window-maximize');
  await app.click('window-close');
  assert.deepEqual(app.windowCalls, ['minimize', 'toggleMaximize', 'close']);
});

test('compact dictation does not require opening the editor or taking over delivery', async () => {
  const app = await openApp();
  assert.equal(app.element('transcript-editor').hidden, true);
  assert.equal(app.element('toggle-editor').getAttribute('aria-expanded'), 'false');
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'текст из компактного режима' });
  assert.equal(app.element('transcript-editor').hidden, true);
  assert.equal(app.element('compact-preview').textContent, 'текст из компактного режима');
  const before = app.calls.length;
  await app.click('toggle-editor');
  assert.equal(app.element('transcript-editor').hidden, false);
  assert.equal(app.element('transcribed').value, 'текст из компактного режима');
  await app.click('toggle-editor');
  assert.equal(app.element('transcript-editor').hidden, true);
  assert.equal(app.element('record').getAttribute('aria-pressed'), 'true');
  assert.equal(app.calls.length, before);
  assert.equal(app.calls.some(call => call.name === 'take_pending'), false);
  assert.deepEqual(app.clipboard, []);
});

test('compact preview follows new text and retains every line without claiming pending delivery', async () => {
  const app = await openApp(); const preview = app.element('compact-preview');
  preview.scrollHeight = 300; preview.clientHeight = 58;
  const before = app.calls.length;
  for (let segment = 1; segment <= 6; segment++) app.emit('transcription-final', { session: 1, id: `1:${segment}`, text: `строка ${segment}` });
  assert.equal(preview.scrollTop, 300);
  assert.equal(preview.textContent, 'строка 1\nстрока 2\nстрока 3\nстрока 4\nстрока 5\nстрока 6');
  assert.equal(app.element('transcript-editor').hidden, true);
  assert.equal(app.calls.slice(before).some(call => ['claim_pending', 'take_pending', 'copy_pending'].includes(call.name)), false);
});

test('reading earlier compact text holds the scroll position until returning to the bottom', async () => {
  const app = await openApp(); const preview = app.element('compact-preview');
  preview.scrollHeight = 300; preview.clientHeight = 58;
  app.emit('transcription-final', { session: 1, id: '1:1', text: 'начало' });
  preview.scrollTop = 40; preview.handlers.get('scroll')();
  app.emit('transcription-final', { session: 1, id: '1:2', text: 'новая фраза' });
  assert.equal(preview.scrollTop, 40);
  preview.scrollTop = 242; preview.handlers.get('scroll')(); preview.scrollHeight = 340;
  app.emit('transcription-final', { session: 1, id: '1:3', text: 'ещё фраза' });
  assert.equal(preview.scrollTop, 340);
  preview.scrollTop = 20; preview.handlers.get('scroll')();
  await app.click('new-record'); assert.equal(preview.scrollTop, 340);
});

test('audio level events increase wave height with loudness without delivering text', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  const before = app.calls.length;
  app.emit('audio-levels', { session: 1, rms: .004, peak: .008, bands: [0, 0, 0, 1, 0, 0, 0, 0] });
  for (let i = 0; i < 12; i++) app.step();
  const quiet = Number(app.element('voice-waves').getAttribute('data-level'));
  app.emit('audio-levels', { session: 1, rms: .3, peak: .6, bands: [0, 0, 0, 1, 0, 0, 0, 0] });
  for (let i = 0; i < 12; i++) app.step();
  const loud = Number(app.element('voice-waves').getAttribute('data-level'));
  assert.ok(loud > quiet + .4);
  assert.equal(app.calls.length, before);
  assert.equal(app.element('transcribed').value, '');
  assert.deepEqual(app.clipboard, []);
});

test('frequency bands change wave shape independently from volume in reduced motion', async () => {
  const app = await openApp({}, undefined, { reducedMotion: true });
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('audio-levels', { session: 1, rms: .1, peak: .2, bands: [0, 1, 0, 0, 0, 0, 0, 0] });
  const low = app.element('wave-front').getAttribute('d');
  const volume = app.element('voice-waves').getAttribute('data-level');
  app.emit('audio-levels', { session: 1, rms: .1, peak: .2, bands: [0, 0, 0, 0, 0, 0, 0, 1] });
  assert.notEqual(app.element('wave-front').getAttribute('d'), low);
  assert.equal(app.element('voice-waves').getAttribute('data-level'), volume);
  assert.equal(app.pendingFrames(), 0);
  app.step(400);
  assert.equal(Number(app.element('voice-waves').getAttribute('data-level')), 0);
});

test('stopping recording resets the wave and rejects late session audio', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('audio-levels', { session: 1, rms: .2, peak: .4, bands: [1, 0, 0, 0, 0, 0, 0, 0] });
  app.step();
  app.emit('recording-status', { session: 1, recording: false, message: 'Стоп' });
  const idle = app.element('wave-front').getAttribute('d');
  assert.equal(Number(app.element('voice-waves').getAttribute('data-level')), 0);
  app.emit('audio-levels', { session: 1, rms: 1, peak: 1, bands: [1, 1, 1] });
  assert.equal(app.element('wave-front').getAttribute('d'), idle);
  app.emit('recording-status', { session: 2, recording: true, message: 'Запись' });
  app.emit('audio-levels', { session: 1, rms: 1, peak: 1, bands: [1, 1, 1] });
  assert.equal(Number(app.element('voice-waves').getAttribute('data-level')), 0);
});

test('the wave pauses on other tabs and while the document is hidden', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  const levels = { session: 1, rms: .1, peak: .2, bands: [1, 0, 0, 0, 0, 0, 0, 0] };
  app.emit('audio-levels', levels);
  assert.equal(app.pendingFrames(), 1);
  await app.click('tab-model');
  assert.equal(app.pendingFrames(), 0);
  app.emit('audio-levels', levels);
  assert.equal(app.pendingFrames(), 0);
  await app.click('tab-dictation');
  assert.equal(app.pendingFrames(), 1);
  app.visibility(true);
  assert.equal(app.pendingFrames(), 0);
  app.visibility(false);
  assert.equal(app.pendingFrames(), 1);
});

test('stale audio decays to the calm idle state', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('audio-levels', { session: 1, rms: .1, peak: .3, bands: [1, 0, 0, 0, 0, 0, 0, 0] });
  for (let i = 0; i < 120; i++) app.step();
  assert.equal(Number(app.element('voice-waves').getAttribute('data-level')), 0);
  assert.equal(app.element('voice-waves').getAttribute('data-motion'), 'idle');
  const idle = app.element('wave-front').getAttribute('d');
  app.step(120);
  assert.notEqual(app.element('wave-front').getAttribute('d'), idle);
});

test('invalid audio levels remain finite and constrained to the SVG viewbox', async () => {
  const app = await openApp({}, undefined, { reducedMotion: true });
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('audio-levels', { session: 1, rms: NaN, peak: Infinity, bands: [NaN, Infinity, -1] });
  assert.equal(Number(app.element('voice-waves').getAttribute('data-level')), 0);
  app.emit('audio-levels', { session: 1, rms: 1, peak: 1, bands: Array(8).fill(1) });
  const path = app.element('wave-front').getAttribute('d');
  assert.doesNotMatch(path, /NaN|Infinity/);
  for (const match of path.matchAll(/\d+,(-?[\d.]+)/g)) assert.ok(Number(match[1]) >= 6 && Number(match[1]) <= 154);
});

test('idle waves move gently without opening the microphone and stop while hidden', async () => {
  const app = await openApp();
  const original = app.element('wave-front').getAttribute('d');
  for (let i = 0; i < 5; i++) app.step(120);
  assert.notEqual(app.element('wave-front').getAttribute('d'), original);
  assert.equal(app.calls.some(call => call.name === 'begin_recording'), false);
  const path = app.element('wave-front').getAttribute('d');
  app.visibility(true);
  for (let i = 0; i < 5; i++) app.step(120);
  assert.equal(app.element('wave-front').getAttribute('d'), path);
  app.visibility(false);
  app.step(120);
  assert.notEqual(app.element('wave-front').getAttribute('d'), path);
});

test('reduced motion keeps idle geometry still while audio still changes its amplitude', async () => {
  const app = await openApp({}, undefined, { reducedMotion: true });
  const idle = app.element('wave-front').getAttribute('d');
  for (let i = 0; i < 8; i++) app.step(120);
  assert.equal(app.element('wave-front').getAttribute('d'), idle);
  assert.equal(app.pendingFrames(), 0);
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('audio-levels', { session: 1, rms: .1, peak: .3, bands: [0, .2, .7, 0, 0, 0, 0, 0] });
  assert.notEqual(app.element('wave-front').getAttribute('d'), idle);
});

test('sustained loud input becomes amber, clipping becomes coral and normal input clears the warning', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  const send = (levels, count) => { for (let i = 0; i < count; i++) { app.emit('audio-levels', { session: 1, bands: Array(8).fill(.3), ...levels }); app.step(64); } };
  send({ rms: .25, peak: .7, input_peak: .7, clipped_ratio: 0 }, 5);
  assert.equal(app.element('voice-waves').getAttribute('data-signal'), 'high');
  assert.equal(app.element('input-warning').textContent, 'Высокий уровень');
  assert.equal(app.element('input-warning').hidden, false);
  send({ rms: .6, peak: 1, input_peak: 1, clipped_ratio: .12 }, 1);
  assert.equal(app.element('voice-waves').getAttribute('data-signal'), 'overload');
  assert.equal(app.element('input-warning').textContent, 'Слишком громко');
  send({ rms: .02, peak: .08, input_peak: .08, clipped_ratio: 0 }, 20);
  assert.equal(app.element('voice-waves').getAttribute('data-signal'), 'normal');
  assert.equal(app.element('input-warning').hidden, true);
  assert.equal(app.element('status-label').textContent, 'Запись');
});

test('one brief peak does not flash a clipping warning and stopping clears an active warning', async () => {
  const app = await openApp();
  app.emit('recording-status', { session: 1, recording: true, message: 'Запись' });
  app.emit('audio-levels', { session: 1, rms: .03, peak: 1, input_peak: 1, clipped_ratio: 0, bands: [] });
  app.step(64);
  app.emit('audio-levels', { session: 1, rms: .03, peak: .1, input_peak: .1, clipped_ratio: 0, bands: [] });
  assert.equal(app.element('voice-waves').getAttribute('data-signal'), 'normal');
  app.emit('audio-levels', { session: 1, rms: .5, peak: 1, input_peak: 1, clipped_ratio: .1, bands: [] });
  assert.equal(app.element('input-warning').hidden, false);
  app.emit('recording-status', { session: 1, recording: false, message: 'Стоп' });
  assert.equal(app.element('input-warning').hidden, true);
  assert.equal(app.element('voice-waves').getAttribute('data-signal'), 'normal');
});
