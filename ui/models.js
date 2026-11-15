/* Native state owns downloads. This view never fetches models or starts them automatically. */
(() => {
  const size = bytes => new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 1 }).format(bytes / 1e6) + ' МБ';
  class VoiceModels {
    constructor($, call, listen, changed, select) {
      Object.assign(this, { $, call, listen, changed, select });
      this.version = -1; this.cards = new Map(); this.busy = new Set(); this.ready = false;
    }
    async init() { await this.listen('model-catalog', ({ payload }) => this.render(payload)); await this.load(); }
    load() {
      if (!this.loading) this.loading = Promise.resolve().then(() => this.call('model_catalog')).then(view => {
        if (view?.entries) { this.render(view); return this.view; } return view;
      }).finally(() => { this.loading = null; });
      return this.loading;
    }
    setVisible(visible) {
      if (this.visible === visible) return;
      this.visible = visible; window.clearTimeout(this.refreshTimer);
      if (visible) this.refresh();
    }
    async refresh() {
      try { await this.load(); } catch (error) { this.feedback(`Не удалось обновить состояние моделей: ${error}`, true); }
      finally {
        window.clearTimeout(this.refreshTimer);
        if (this.visible) this.refreshTimer = window.setTimeout(() => this.refresh(), 1000);
      }
    }
    feedback(text, error = false) { this.$('catalog-feedback').textContent = text; this.$('catalog-feedback').classList.toggle('error', error); }
    setLocked(locked, updating = false) { this.locked = locked || updating; this.updating = updating; if (this.view) this.render(this.view, true); }
    makeCard(entry) {
      const node = (tag, className, text) => { const el = document.createElement(tag); el.className = className; if (text) el.textContent = text; return el; };
      const card = node('article', 'model-card');
      const heading = node('div', 'model-card-heading'); const state = node('span', 'model-state'); heading.append(node('h3', '', entry.name), state);
      const description = node('p', 'model-description', entry.description); const bytes = node('p', 'model-size'); const progress = node('progress', 'model-progress');
      progress.setAttribute('aria-label', `Загрузка ${entry.name}`);
      const note = node('p', 'model-transfer-note'); const actions = node('div', 'model-actions'); const buttons = {};
      for (const [action, label] of [['download', 'Загрузить'], ['resume', 'Продолжить'], ['pause', 'Пауза'], ['cancel', 'Отменить'], ['select', 'Сделать основной'], ['delete', 'Удалить']]) {
        const button = node('button', ['delete', 'cancel'].includes(action) ? 'mini model-remove' : 'mini', label);
        button.type = 'button'; button.setAttribute('aria-label', `${label}: ${entry.name}`);
        button.addEventListener('click', () => this.action(entry.id, action)); buttons[action] = button; actions.append(button);
      }
      card.append(heading, description, bytes, progress, note, actions);
      const result = { card, state, description, bytes, progress, note, buttons }; this.cards.set(entry.id, result); this.$('model-catalog-items').append(card); return result;
    }
    render(view, repeat = false) {
      if (!view?.entries || (!repeat && view.version <= this.version)) return;
      const old = this.view; this.view = view; this.version = view.version; this.ready = true;
      for (const entry of view.entries) {
        const card = this.cards.get(entry.id) || this.makeCard(entry); card.card.hidden = false;
        const verifying = entry.state === 'verifying';
        const readyToVerify = !entry.installed && entry.downloaded === entry.size && entry.size > 0;
        const resumable = ['paused', 'verification_pending'].includes(entry.state);
        const active = ['downloading', 'verifying', 'pausing', 'cancelling'].includes(entry.state);
        const stopping = ['pausing', 'cancelling'].includes(entry.state);
        card.state.textContent = entry.state === 'damaged' ? 'Требует восстановления' : entry.installed ? (entry.primary ? 'Основная' : 'Установлена') : entry.state === 'missing' ? 'Недоступна' : stopping ? 'Останавливаем…' : verifying ? 'Проверка файлов' : active ? 'Загружается' : readyToVerify ? 'Ожидает проверки' : entry.state === 'paused' ? 'Пауза' : 'Не установлена';
        card.card.classList.toggle('model-primary', entry.primary);
        card.card.classList.toggle('model-bundled', entry.bundled);
        card.description.hidden = entry.bundled; card.bytes.hidden = entry.bundled;
        card.bytes.textContent = entry.bundled ? 'В комплекте приложения' : `Файлы модели: ${size(entry.size)} · ${entry.license}`;
        card.bytes.title = entry.size ? `${entry.size.toLocaleString('ru-RU')} байт` : '';
        const checked = Math.min(entry.verified || 0, entry.size);
        card.progress.hidden = entry.installed || (!active && !resumable);
        card.progress.max = entry.size || 1; card.progress.value = verifying ? checked : entry.downloaded;
        card.progress.setAttribute('aria-label', `${verifying ? 'Проверка файлов' : 'Загрузка'} ${entry.name}`);
        card.progress.setAttribute('aria-valuetext', `${verifying ? 'Проверено' : 'Скачано'} ${size(verifying ? checked : entry.downloaded)} из ${size(entry.size)}`);
        const transferNote = verifying ? `Скачано ${size(entry.size)}. Проверено ${size(checked)} из ${size(entry.size)}. После проверки модель станет доступна.` : readyToVerify && resumable ? 'Все файлы скачаны. Нажмите «Проверить», чтобы завершить установку.' : !entry.installed && entry.downloaded ? `${size(entry.downloaded)} из ${size(entry.size)} сохранено` : '';
        card.note.textContent = verifying ? transferNote : [transferNote, entry.message || (entry.state === 'damaged' ? 'Удалите неполную модель и загрузите её заново. Остальные модели сохранятся.' : '')].filter((text, index, all) => text && all.indexOf(text) === index).join(' '); card.note.hidden = !card.note.textContent;
        card.buttons.resume.textContent = readyToVerify ? 'Проверить' : 'Продолжить';
        card.buttons.resume.setAttribute('aria-label', `${card.buttons.resume.textContent}: ${entry.name}`);
        const visible = { download: !entry.installed && entry.state === 'available', resume: !entry.installed && resumable, pause: active && !stopping, cancel: !entry.installed && (active || resumable), select: entry.installed && !entry.primary, delete: (entry.installed || entry.state === 'damaged') && !entry.bundled };
        for (const [action, button] of Object.entries(card.buttons)) {
          button.hidden = !visible[action];
          button.disabled = this.busy.has(entry.id) || (['select', 'delete'].includes(action) && this.locked) || (action === 'delete' && (entry.primary || view.active)) || (['download', 'resume'].includes(action) && (view.active || this.updating)) || (action === 'cancel' && entry.state === 'cancelling');
        }
        const focused = Object.values(card.buttons).find(button => document.activeElement === button);
        if (focused?.hidden) Object.values(card.buttons).find(button => !button.hidden && !button.disabled)?.focus();
        card.buttons.delete.title = entry.primary ? 'Сначала назначьте другую основную модель' : '';
      }
      for (const [id, card] of this.cards) if (!view.entries.some(entry => entry.id === id)) card.card.hidden = true;
      this.$('engine-status').textContent = `${view.selected?.engine === 'gigaam' ? 'GigaAM' : view.selected?.engine === 'nemotron' ? 'Nemotron' : 'Vosk'} · CPU · RU`;
      if (view.error) this.feedback(view.error, true);
      else if (old?.error && this.$('catalog-feedback').textContent === old.error) this.feedback('');
      const completed = view.entries.find(entry => !entry.bundled && entry.installed && !old?.entries.some(previous => previous.id === entry.id && previous.installed));
      if (old && completed) { this.feedback(`${completed.name}: файлы скачаны и проверены. Можно назначить основной.`); this.changed?.(); }
    }
    async action(id, action) {
      if (this.busy.has(id)) return;
      const entry = this.view.entries.find(item => item.id === id); if (!entry) return;
      this.busy.add(id); this.render(this.view, true);
      try {
        if (action === 'select') { await this.select(entry.path); await this.load(); this.feedback(`${entry.name} назначена основной.`); }
        else {
          const command = { download: 'download_model', resume: 'download_model', pause: 'pause_model_download', cancel: 'cancel_model_download', delete: 'delete_model' }[action];
          this.render(await this.call(command, { id }));
          this.feedback(action === 'cancel' ? 'Отменяем операцию и удаляем незавершённую загрузку.' : action === 'delete' ? `${entry.name} удалена.` : action === 'pause' ? 'Приостанавливаем операцию. Скачанные файлы сохраняются.' : entry.downloaded === entry.size ? 'Проверяем скачанные файлы. Модель станет доступна после успешной проверки.' : 'Загрузка началась. Можно приостановить или закрыть приложение и продолжить позже.');
          if (action === 'delete') await this.changed?.();
        }
      } catch (error) { this.feedback(String(error), true); }
      finally { this.busy.delete(id); this.render(this.view, true); }
    }
  }
  window.VoiceModels = VoiceModels;
})();
