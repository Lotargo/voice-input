/* Microphone preferences and a bounded level check; no WebView microphone API. */
window.VoiceAudio = class VoiceAudio {
  constructor(element, invoke, changed = () => {}) {
    this.element = element; this.invoke = invoke; this.changed = changed;
    this.view = null; this.busy = false; this.active = false; this.testing = false; this.session = -1;
    this.label = 'Микрофон по умолчанию';
    this.visible = false; this.starting = false; this.cancelled = false;
    element('input-device').addEventListener('change', () => this.save());
    element('noise-suppression').addEventListener('change', () => this.save());
    element('refresh-inputs').addEventListener('click', () => this.refresh());
    element('test-input').addEventListener('click', () => this.toggleTest());
  }
  message(text, error = false) {
    this.element('audio-feedback').textContent = text;
    this.element('audio-feedback').classList.toggle('error', error);
  }
  render() {
    const locked = this.busy || this.active;
    this.element('input-device').disabled = locked || !this.view;
    this.element('noise-suppression').disabled = locked || !this.view;
    this.element('refresh-inputs').disabled = this.busy;
    this.element('test-input').disabled = this.busy || (this.active && !this.testing) || (!this.active && !this.view?.available);
    this.element('test-input').textContent = this.testing ? 'Остановить проверку' : 'Проверить микрофон';
    this.element('test-input').setAttribute('aria-pressed', String(this.testing));
    this.element('audio-lock').hidden = !this.active;
    this.element('audio-settings').setAttribute('aria-busy', String(this.busy));
    this.changed();
  }
  apply(view) {
    this.view = view;
    const option = (value, text) => { const item = document.createElement('option'); item.value = value; item.textContent = text; return item; };
    const entries = [option('default', 'По умолчанию в Windows'), ...view.devices.map(d => option(d.id, d.name + (d.is_default ? ' · по умолчанию' : '')))];
    const id = view.options.device_id;
    if (id && !view.devices.some(d => d.id === id)) {
      const missing = option(id, `${view.options.device_name || 'Сохранённый микрофон'} · недоступен`); missing.disabled = true; entries.push(missing);
    }
    this.element('input-device').replaceChildren(...entries);
    this.element('input-device').value = id || 'default';
    this.element('noise-suppression').checked = view.options.denoise;
    this.label = view.name;
    this.element('input-device-name').textContent = view.available ? `Выбран: ${view.name}` : `Недоступен: ${view.name}. Подключите его или выберите другой.`;
    this.element('input-device-name').classList.toggle('error', !view.available);
    this.render();
  }
  async refresh() {
    if (this.busy) return;
    this.busy = true; this.render();
    try { const view = await this.invoke('input_settings'); this.apply(view); if (view.error) this.message(view.error, true); }
    catch (error) { this.message(String(error), true); }
    finally { this.busy = false; this.render(); }
  }
  async save() {
    if (this.busy || this.active || !this.view) return;
    const previous = this.view;
    const value = this.element('input-device').value;
    const denoise = this.element('noise-suppression').checked;
    this.busy = true; this.render();
    try {
      this.apply(await this.invoke('set_input_settings', { deviceId: value === 'default' ? null : value, denoise }));
      this.message(denoise ? 'Сохранено. RNNoise включён: локальная обработка на CPU.' : 'Сохранено. Шумоподавление выключено.');
    } catch (error) { this.apply(previous); this.message(String(error), true); }
    finally { this.busy = false; this.render(); }
  }
  setStatus(payload) {
    if (payload.session < this.session) return;
    const wasTesting = this.testing;
    this.session = payload.session; this.active = payload.recording; this.testing = Boolean(payload.recording && payload.testing);
    if (payload.input_name) this.label = payload.input_name;
    if (payload.testing) {
      this.message(payload.message, /отключ|недоступ|не найден|не удалось|перестал|переполн|прерван/i.test(payload.message));
      if (!this.testing) this.meters(null);
    }
    this.render();
    if (this.testing && !wasTesting) this.element('input-quality').scrollIntoView?.({ block: 'nearest' });
    if (!this.active && (wasTesting || payload.testing)) this.refresh();
  }
  levels(payload) {
    if (this.testing && payload.testing && payload.session === this.session) this.meters(payload);
  }
  meters(levels) {
    const finite = value => Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0;
    const source = finite(levels?.raw_rms), cleaned = finite(levels?.rms);
    const display = (id, value) => {
      const db = value > .00001 ? Math.max(-60, 20 * Math.log10(value)) : -60;
      this.element(id).value = db;
      this.element(id).setAttribute('aria-valuetext', value > .00001 ? `${db.toFixed(0)} дБFS` : 'Тишина');
      this.element(`${id}-value`).textContent = value > .00001 ? `${db.toFixed(0)} дБFS` : '—';
    };
    display('input-level', source); display('clean-level', cleaned);
    const overloaded = finite(levels?.input_peak) >= .985 || finite(levels?.clipped_ratio) >= .005;
    const high = finite(levels?.input_peak) >= .8 || source >= .2;
    const label = !levels ? 'Проверка остановлена' : overloaded ? 'Перегрузка — уменьшите уровень микрофона' : high ? 'Высокий уровень' : source < .004 ? 'Низкий уровень или тишина — скажите несколько слов' : 'Нормальный уровень';
    this.element('input-quality').textContent = label;
    this.element('input-quality').setAttribute('data-signal', overloaded ? 'overload' : high ? 'high' : 'normal');
  }
  async toggleTest() {
    if (this.busy || (this.active && !this.testing)) return;
    const start = !this.testing;
    this.starting = start; this.cancelled = false;
    this.busy = true; this.render();
    try {
      const command = this.testing ? 'end_microphone_test' : 'begin_microphone_test';
      this.message(this.testing ? 'Останавливаем проверку…' : 'Открываем микрофон…');
      await this.invoke(command);
      if (start && (this.cancelled || !this.visible)) await this.invoke('end_microphone_test');
    } catch (error) { this.message(String(error), true); }
    finally { this.starting = false; this.busy = false; this.render(); }
  }
  setVisible(visible) { this.visible = visible; if (!visible) this.stopTest(); }
  async stopTest() {
    this.cancelled = true;
    if (!this.testing && !this.starting) return;
    try { await this.invoke('end_microphone_test'); } catch (error) { this.message(String(error), true); }
  }
};
