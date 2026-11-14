/* Local state only at startup. GitHub is contacted only after Check is clicked. */
window.VoiceUpdates = class VoiceUpdates {
  constructor(element, invoke, listen, setBusy = () => {}) {
    this.element = element; this.invoke = invoke; this.listen = listen; this.setBusy = setBusy;
    this.recording = false; this.busy = false; this.available = null; this.supported = false;
    element('check-updates').addEventListener('click', () => this.check());
    element('install-update').addEventListener('click', () => this.install());
  }
  async init() {
    try {
      const info = await this.invoke('update_info');
      this.supported = info.supported;
      this.element('app-version').textContent = `${info.version} · ${info.mode === 'portable' ? 'Portable' : info.mode === 'installed' ? 'Установленная' : 'Разработка'}`;
      this.message(info.last_result || (this.supported ? 'Проверка запускается только по вашей команде.' : 'Обновления доступны в сборках из GitHub Releases.'));
      await this.listen('update-progress', ({ payload }) => this.progress(payload));
    } catch (error) { this.message(String(error)); }
    this.render();
  }
  message(text) { this.element('update-feedback').textContent = text; }
  setRecording(recording) { this.recording = recording; this.render(); }
  render() {
    this.element('check-updates').disabled = this.busy || !this.supported;
    this.element('install-update').hidden = !this.available;
    this.element('install-update').disabled = this.busy || this.recording;
    this.element('update-restart-note').hidden = !this.available;
    this.element('update-stop-note').hidden = !this.available || !this.recording;
    this.element('update-notes').hidden = !this.available?.notes;
    this.element('update-notes').textContent = this.available?.notes || '';
    this.element('update-download').hidden = !this.busy || !this.installing;
  }
  async check() {
    if (this.busy || !this.supported) return;
    this.busy = true; this.available = null; this.render();
    this.message('Проверяем GitHub Releases…');
    try {
      this.available = await this.invoke('check_updates');
      this.message(this.available ? `Доступна версия ${this.available.version}` : 'Установлена последняя версия.');
    } catch (error) { this.message(String(error)); }
    finally { this.busy = false; this.render(); }
  }
  async install() {
    if (this.busy || this.recording || !this.available) return;
    this.busy = true; this.installing = true; this.setBusy(true); this.render();
    this.element('update-download').removeAttribute('value');
    this.message('Загружаем и проверяем подпись…');
    try {
      await this.invoke('install_update', { version: this.available.version });
      this.message('Обновление подготовлено. Перезапускаем приложение…');
      // Success hands over to the installer/helper. Do not enable dictation again.
    } catch (error) {
      this.message(String(error)); this.busy = false; this.installing = false;
      this.setBusy(false); this.render();
    }
  }
  progress(payload) {
    if (!this.installing) return;
    if (payload.phase === 'installing') { this.message('Подпись проверена. Подготавливаем установку…'); return; }
    const progress = this.element('update-download');
    if (payload.total > 0) {
      progress.value = Math.min(1, payload.downloaded / payload.total);
      this.message(`Загружаем обновление: ${Math.round(progress.value * 100)}%`);
    } else { progress.removeAttribute('value'); this.message(`Загружено ${(payload.downloaded / 1048576).toFixed(1)} МБ…`); }
  }
};
