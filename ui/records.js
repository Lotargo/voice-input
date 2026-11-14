/* In-memory record views. Keep local edits while native transitions are in flight. */
(() => {
  const join = (left, right) => left ? left + (/\s$/.test(left) ? '' : ' ') + right.trim() : right.trim();
  class DictationRecords {
    constructor() { this.entries = new Map(); this.currentId = 1; this.version = 0; this.seen = new Set(); this.ensure(1, 0); }
    ensure(id, generation) {
      let entry = this.entries.get(id);
      if (entry && entry.generation > generation) return null;
      if (!entry || entry.generation < generation) {
        entry = { id, generation, text: '', partial: '', fragments: [], pending: new Set(), dirty: false, revision: 0 };
        this.entries.set(id, entry);
      }
      return entry;
    }
    get current() { return this.entries.get(this.currentId); }
    hydrate(view) {
      const entry = this.ensure(view.id, view.generation);
      if (!entry) return;
      const known = new Set(entry.fragments.map(part => part.id));
      const incoming = new Set((view.fragments || []).map(part => part.id));
      const stale = entry.fragments.some(part => !incoming.has(part.id));
      const replace = !entry.dirty && !stale;
      if (replace) entry.text = view.text;
      for (const part of view.fragments || []) {
        if (!known.has(part.id)) {
          if (!replace) entry.text = join(entry.text, part.text);
          entry.fragments.push(part);
          if (!this.seen.has(part.id)) entry.pending.add(part.id);
        }
        this.seen.add(part.id);
      }
      if (!stale) entry.partial = view.partial || '';
    }
    snapshot(snapshot) {
      if (snapshot.version < this.version) return false;
      this.version = snapshot.version;
      for (const view of [...snapshot.history, snapshot.current]) this.hydrate(view);
      this.currentId = snapshot.current.id;
      return true;
    }
    target(payload) {
      const id = payload.record_id ?? this.currentId;
      const generation = payload.generation ?? this.current.generation;
      if ((payload.version ?? this.version) < this.version || id < this.currentId) return null;
      const entry = this.ensure(id, generation);
      if (!entry) return null;
      this.version = payload.version ?? this.version;
      this.currentId = id;
      return entry;
    }
    append(payload) {
      const entry = this.target(payload);
      const text = payload.text.trim();
      if (!entry || !text || (payload.id && this.seen.has(payload.id))) return false;
      entry.text = join(entry.text, text);
      entry.partial = '';
      entry.revision += 1;
      if (payload.id) {
        this.seen.add(payload.id);
        entry.fragments.push({ id: payload.id, text });
        entry.pending.add(payload.id);
      }
      return true;
    }
    partial(payload) { const entry = this.target(payload); if (entry) entry.partial = payload.text; return !!entry; }
    edit(text) {
      const entry = this.current;
      entry.text = text;
      entry.dirty = true;
      entry.revision += 1;
      return { recordId: entry.id, generation: entry.generation, text, baseIds: entry.fragments.map(part => part.id), revision: entry.revision };
    }
    acknowledge(view, revision) {
      const entry = this.entries.get(view.id);
      if (entry?.generation !== view.generation || entry.revision !== revision) return;
      entry.dirty = false;
      this.hydrate(view);
    }
    take(ids) { for (const entry of this.entries.values()) for (const id of ids) entry.pending.delete(id); }
    history() { return [...this.entries.values()].filter(entry => entry.id !== this.currentId && entry.text).sort((a, b) => b.id - a.id); }
  }
  window.DictationRecords = DictationRecords;
})();
