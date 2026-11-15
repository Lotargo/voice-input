const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');
function setup() {
  const elements = new Map(), calls = [], events = new Map();
  class Element {
    constructor() { this.children = []; this.attributes = {}; this.classList = { toggle() {} }; }
    append(...children) { this.children.push(...children); }
    setAttribute(key, value) { this.attributes[key] = value; }
    addEventListener(name, callback) { this[name] = callback; }
    focus() { document.activeElement = this; }
  }
  const document = { createElement: () => new Element(), activeElement: null };
  const $ = id => { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id); };
  const timers = new Map(); let nextTimer = 0;
  const sandbox = { window: { setTimeout: callback => { const id = ++nextTimer; timers.set(id, callback); return id; }, clearTimeout: id => timers.delete(id) }, document };
  vm.runInNewContext(readFileSync(require('node:path').join(__dirname, '../ui/models.js'), 'utf8'), sandbox);
  const view = { version: 1, selected: { engine: 'vosk', path: 'builtin' }, active: false, entries: [
    { id: 'base', name: 'Vosk', path: 'builtin', bundled: true, installed: true, primary: true, size: 0, downloaded: 0, state: 'installed' },
    { id: 'giga', name: 'GigaAM', path: 'giga-path', bundled: false, installed: false, primary: false, size: 231898272, downloaded: 0, state: 'available', license: 'MIT' },
  ] };
  let response = view;
  const controller = new sandbox.window.VoiceModels($, async (name, args) => { calls.push({ name, args }); return response; }, async (name, fn) => events.set(name, fn), async () => {}, async path => calls.push({ name: 'select', path }));
  const update = patch => { response = { ...response, ...patch, version: response.version + 1 }; return response; };
  const entry = patch => update({ entries: response.entries.map(e => e.id === 'giga' ? { ...e, ...patch } : e) });
  return { controller, $, calls, events, view, entry, update, timers, card: () => controller.cards.get('giga') };
}
test('catalog startup only reads local state and displays final model size', async () => {
  const app = setup(); await app.controller.init();
  assert.deepEqual(app.calls.map(call => call.name), ['model_catalog']);
  assert.match(app.card().bytes.textContent, /231,9 МБ/); assert.match(app.card().bytes.title, /231/);
  assert.equal(app.card().buttons.download.hidden, false); assert.equal(app.card().buttons.resume.hidden, true);
});
test('an unsupported old selection produces no custom card or download completion', async () => {
  const app = setup(); await app.controller.init();
  const changed = app.update({ selected: { path: 'old-custom', engine: 'vosk' }, error: 'Доступны только модели из каталога' });
  app.controller.render(changed); assert.equal(app.controller.cards.has('custom'), false);
  assert.equal(app.$('catalog-feedback').textContent, changed.error);
  app.controller.render(app.update({ selected: app.view.selected, error: null }));
  assert.equal(app.$('catalog-feedback').textContent, '');
});
test('interrupted download is resumable after startup without starting a network request', async () => {
  const app = setup(); app.entry({ state: 'paused', downloaded: 45000000 }); await app.controller.init();
  assert.equal(app.card().buttons.resume.hidden, false); assert.equal(app.card().progress.value, 45000000);
  assert.equal(app.calls.some(call => call.name === 'download_model'), false);
});
test('pause preserves progress and stale events cannot rewind native state', async () => {
  const app = setup(); app.entry({ state: 'downloading', downloaded: 5000000 }); app.update({ active: true }); await app.controller.init();
  const card = app.card(); app.entry({ state: 'pausing', downloaded: 5000000 }); await app.controller.action('giga', 'pause');
  assert.equal(app.calls.at(-1).name, 'pause_model_download');
  const paused = app.entry({ state: 'paused', downloaded: 6000000 }); app.update({ active: false });
  app.controller.render({ ...paused, version: paused.version + 1, active: false });
  app.controller.render(app.view);
  assert.equal(app.card(), card); assert.equal(card.progress.value, 6000000); assert.equal(card.buttons.resume.hidden, false);
});
test('selection and deletion respect capture locks and primary model ownership', async () => {
  const app = setup(); app.entry({ state: 'installed', installed: true, downloaded: 231898272 }); await app.controller.init();
  app.controller.setLocked(true); assert.equal(app.card().buttons.select.disabled, true); assert.equal(app.card().buttons.delete.disabled, true);
  app.controller.setLocked(false); assert.equal(app.card().buttons.select.disabled, false);
  app.controller.render(app.entry({ primary: true })); assert.equal(app.card().buttons.delete.disabled, true); assert.equal(app.card().buttons.select.hidden, true);
});
test('resume and cancel use native commands and never reset local bytes optimistically', async () => {
  const app = setup(); app.entry({ state: 'paused', downloaded: 100 }); await app.controller.init();
  await app.controller.action('giga', 'resume'); assert.equal(app.calls.at(-1).name, 'download_model'); assert.equal(app.card().progress.value, 100);
  app.entry({ state: 'cancelling', downloaded: 100 }); await app.controller.action('giga', 'cancel');
  assert.equal(app.calls.at(-1).name, 'cancel_model_download'); assert.equal(app.card().progress.value, 100);
});

test('download completion shows separate verification progress and keeps selection unavailable', async () => {
  const app = setup(); app.entry({ state: 'downloading', downloaded: 231898272 }); app.update({ active: true }); await app.controller.init();
  assert.equal(app.card().state.textContent, 'Загружается');
  app.controller.render(app.entry({ state: 'verifying', verified: 100000000 }));
  assert.equal(app.card().state.textContent, 'Проверка файлов');
  assert.equal(app.card().progress.value, 100000000);
  assert.match(app.card().progress.attributes['aria-valuetext'], /Проверено 100 МБ/);
  assert.match(app.card().note.textContent, /После проверки модель станет доступна/);
  assert.equal(app.card().buttons.select.hidden, true); assert.equal(app.card().buttons.pause.hidden, false);
  app.controller.render(app.entry({ state: 'installed', installed: true, verified: 231898272 }));
  assert.equal(app.card().progress.hidden, true); assert.equal(app.card().buttons.select.hidden, false);
  assert.match(app.$('catalog-feedback').textContent, /скачаны и проверены/);
});

test('fully saved download after restart offers local verification instead of model availability', async () => {
  const app = setup(); app.entry({ state: 'verification_pending', downloaded: 231898272 }); await app.controller.init();
  assert.equal(app.card().state.textContent, 'Ожидает проверки');
  assert.equal(app.card().buttons.resume.hidden, false); assert.equal(app.card().buttons.resume.textContent, 'Проверить');
  assert.equal(app.card().buttons.select.hidden, true);
  await app.controller.action('giga', 'resume'); assert.equal(app.calls.at(-1).name, 'download_model');
  assert.match(app.$('catalog-feedback').textContent, /Проверяем скачанные файлы/);
});

test('visible catalog reconciles a lost completion event via local snapshots and stops when hidden', async () => {
  const app = setup(); app.entry({ state: 'verifying', downloaded: 231898272, verified: 150000000 }); await app.controller.init();
  app.controller.setVisible(true); await app.controller.loading; await Promise.resolve();
  assert.equal(app.timers.size, 1);
  app.entry({ state: 'installed', installed: true, verified: 231898272 });
  await [...app.timers.values()][0]();
  assert.equal(app.card().state.textContent, 'Установлена');
  assert.equal(app.card().buttons.select.hidden, false);
  assert.equal(app.calls.every(call => call.name === 'model_catalog'), true);
  app.controller.setVisible(false); assert.equal(app.timers.size, 0);
  const count = app.calls.length; app.controller.setVisible(true); await app.controller.loading;
  assert.equal(app.calls.length, count + 1); app.controller.setVisible(false);
});

test('concurrent local refreshes share one request', async () => {
  const app = setup(); await app.controller.init(); const count = app.calls.length;
  await Promise.all([app.controller.load(), app.controller.load()]);
  assert.equal(app.calls.length, count + 1);
});

test('a delayed local snapshot cannot return older model choices after a completion event', async () => {
  const app = setup(); await app.controller.init();
  const stale = app.controller.view; let respond;
  app.controller.call = () => new Promise(resolve => { respond = resolve; });
  const loading = app.controller.load(); await Promise.resolve();
  const installed = app.entry({ state: 'installed', installed: true, downloaded: 231898272 });
  app.controller.render(installed);
  respond(stale);
  const snapshot = await loading;
  assert.equal(snapshot, installed);
  assert.equal(snapshot.entries.find(entry => entry.id === 'giga').installed, true);
});
