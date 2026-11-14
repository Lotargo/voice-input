/* Local visualizer: scalar levels only; no webview microphone access. */
window.VoiceWave = class VoiceWave {
  constructor(surface, front, back, ribbon, onSignal = () => {}) {
    this.surface = surface;
    this.paths = [front, back, ribbon];
    this.onSignal = onSignal;
    this.enabled = false;
    this.visible = true;
    this.hasData = false;
    this.frame = this.timer = this.phase = this.level = this.lastFrame = 0;
    this.bands = Array(8).fill(0);
    this.target = { level: 0, bands: Array(8).fill(0) };
    this.signal = 'normal';
    this.surface.setAttribute('data-signal', 'normal');
    this.candidate = 'normal';
    this.candidateAt = this.holdUntil = 0;
    this.media = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    this.media?.addEventListener?.('change', () => { this.pause(); this.resume(); });
    this.render(0, this.bands, 0);
    this.resume();
  }

  static bounded(value) { return Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0; }

  pause() {
    if (this.frame) window.cancelAnimationFrame(this.frame);
    if (this.timer) window.clearTimeout(this.timer);
    this.frame = this.timer = this.lastFrame = 0;
  }

  setSignal(signal) {
    if (signal === this.signal) return;
    this.signal = signal;
    this.surface.setAttribute('data-signal', signal);
    this.onSignal(signal);
  }

  setRecording(enabled) {
    if (enabled === this.enabled) return;
    this.enabled = enabled;
    this.pause();
    this.hasData = false;
    this.level = 0;
    this.bands.fill(0);
    this.target = { level: 0, bands: Array(8).fill(0) };
    this.candidate = 'normal';
    this.candidateAt = this.holdUntil = 0;
    this.setSignal('normal');
    this.render(0, this.bands, this.media?.matches ? 0 : this.phase);
    this.resume();
  }

  setVisible(visible) {
    if (visible === this.visible) return;
    this.visible = visible;
    if (!visible) this.pause();
    else this.resume();
  }

  classify(payload, now) {
    const rms = VoiceWave.bounded(payload.rms);
    const peak = VoiceWave.bounded(Number.isFinite(payload.input_peak) ? payload.input_peak : payload.peak);
    const clipped = VoiceWave.bounded(payload.clipped_ratio);
    const next = clipped >= .005 || peak >= .985 || rms >= .42 ? 'overload' :
      peak >= .8 || rms >= .2 ? 'high' : 'normal';
    if (next !== this.candidate) { this.candidate = next; this.candidateAt = now; }
    const rank = { normal: 0, high: 1, overload: 2 };
    if (next === this.signal) { this.holdUntil = now + 700; return; }
    const rising = rank[next] > rank[this.signal];
    if ((rising && (clipped >= .005 || now - this.candidateAt >= 140)) ||
        (!rising && now >= this.holdUntil && now - this.candidateAt >= 400)) {
      this.setSignal(next);
      this.holdUntil = now + 700;
    }
  }

  update(payload) {
    if (!this.enabled) return;
    const now = performance.now();
    this.classify(payload, now);
    const amplitude = Math.max(VoiceWave.bounded(payload.rms), VoiceWave.bounded(payload.peak) * .2);
    const db = 20 * Math.log10(Math.max(amplitude, .000001));
    const energy = Math.pow(VoiceWave.bounded((db + 55) / 49), 1.55);
    this.target = {
      level: Math.max(energy, VoiceWave.bounded(payload.clipped_ratio) >= .005 ? .95 : 0),
      bands: Array.from({ length: 8 }, (_, index) => VoiceWave.bounded(payload.bands?.[index])),
    };
    this.hasData = true;
    this.lastData = now;
    // Audio should wake the slow idle loop immediately.
    if (this.timer) { window.clearTimeout(this.timer); this.timer = 0; }
    this.resume();
  }

  fresh(now) { return this.enabled && this.hasData && now - this.lastData <= 360; }

  resume() {
    if (!this.visible) return;
    const now = performance.now();
    const fresh = this.fresh(now);
    if (!fresh) { this.candidate = 'normal'; this.holdUntil = 0; this.setSignal('normal'); }
    if (this.media?.matches) {
      this.level = fresh ? this.target.level : 0;
      this.bands = fresh ? [...this.target.bands] : Array(8).fill(0);
      this.render(this.level, this.bands, 0);
      if (this.timer) window.clearTimeout(this.timer);
      this.timer = fresh ? window.setTimeout(() => { this.timer = 0; this.resume(); }, 380) : 0;
    } else if (!this.frame && !this.timer) {
      this.frame = window.requestAnimationFrame(time => this.tick(time));
    }
  }

  tick(now) {
    this.frame = 0;
    if (!this.visible) return;
    const dt = this.lastFrame ? Math.min(160, Math.max(1, now - this.lastFrame)) : 16;
    this.lastFrame = now;
    const fresh = this.fresh(now);
    const target = fresh ? this.target.level : 0;
    if (!fresh) { this.candidate = 'normal'; this.holdUntil = 0; this.setSignal('normal'); }
    const mix = 1 - Math.exp(-dt / (target > this.level ? 38 : 200));
    this.level += (target - this.level) * mix;
    if (this.level < .001) this.level = 0;
    this.bands = this.bands.map((value, index) => value + ((fresh ? this.target.bands[index] : 0) - value) * mix);
    this.phase = (this.phase + dt * (.00065 + this.level * .0035)) % (Math.PI * 2);
    const active = this.level > .012 || target > .012;
    if (!this.lastRender || now - this.lastRender >= (active ? 30 : 95)) {
      this.render(this.level, this.bands, this.phase);
      this.lastRender = now;
    }
    if (active) this.frame = window.requestAnimationFrame(time => this.tick(time));
    else this.timer = window.setTimeout(() => { this.timer = 0; this.resume(); }, 100);
  }

  render(level, bands, phase) {
    let weight = 0, centroid = 0;
    bands.forEach((value, index) => { weight += value; centroid += value * index; });
    centroid = weight ? centroid / weight / 7 : 0;
    const low = (bands[0] + bands[1] + bands[2]) / 3;
    const mid = (bands[3] + bands[4] + bands[5]) / 3;
    const high = (bands[6] + bands[7]) / 2;
    const idleAmplitude = 16 + 3 * Math.sin(phase * .5);
    const amplitude = idleAmplitude + 62 * level;
    const front = [], back = [];
    for (let x = 0; x <= 560; x += 8) {
      const t = x / 560;
      const envelope = Math.pow(Math.sin(Math.PI * t), .65);
      const carrier = Math.sin(t * Math.PI * 2 * (1.65 + centroid * 3.6) - phase);
      const detail = low * Math.sin(t * 9 - phase * .8) + mid * Math.sin(t * 25 + phase * 1.2) + high * Math.sin(t * 48 - phase * 1.4);
      const shape = Math.max(-1, Math.min(1, carrier * .72 + detail * .28));
      const y = Math.max(6, Math.min(154, 80 + amplitude * envelope * shape));
      const z = Math.max(6, Math.min(154, 80 + amplitude * .74 * envelope * Math.sin(t * Math.PI * 2 * (1.85 + centroid * 3.1) + phase * .85 + 1.3)));
      front.push(`${x},${y.toFixed(2)}`);
      back.push(`${x},${z.toFixed(2)}`);
    }
    const values = [`M${front.join('L')}`, `M${back.join('L')}`, `M${front.join('L')}L${back.slice().reverse().join('L')}Z`];
    this.paths.forEach((path, index) => path.setAttribute('d', values[index]));
    this.paths[0].style.strokeWidth = String(1.15 + level * 1.5);
    this.paths[1].style.strokeWidth = String(.8 + level * .8);
    this.surface.style.opacity = String(.42 + level * .58);
    this.surface.setAttribute('data-level', level.toFixed(3));
    this.surface.setAttribute('data-motion', level > .012 ? 'reactive' : 'idle');
  }
};
