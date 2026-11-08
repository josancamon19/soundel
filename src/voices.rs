//! Instruments. Each is a small, allocation-free pool of synth voices that renders into a `Mix`.

use crate::dsp::*;
use crate::rng::Rng;

/// Per-sample stereo accumulator with reverb and delay sends.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mix {
    pub dry: (f32, f32),
    pub rev: (f32, f32),
    pub dly: (f32, f32),
}

impl Mix {
    #[inline]
    pub fn add(&mut self, l: f32, r: f32, rev: f32, dly: f32) {
        self.dry.0 += l;
        self.dry.1 += r;
        self.rev.0 += l * rev;
        self.rev.1 += r * rev;
        self.dly.0 += l * dly;
        self.dly.1 += r * dly;
    }
}

// ───────────────────────────── Kick ─────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
struct KickVoice {
    on: bool,
    wait: u32,
    phase: f32,
    penv: f32,
    pmul: f32,
    amp: f32,
    amul: f32,
    att: f32,
    click: f32,
    cmul: f32,
    f0: f32,
    f1: f32,
    vel: f32,
}

/// Soft sine kick with a pitch drop (≈190 → 52 Hz in Focus) and a faint click.
pub struct Kick {
    sr: f32,
    v: [KickVoice; 3],
    next: usize,
    noise: Noise,
    hp: OnePole,
    /// Current amplitude envelope, used for sidechain ducking.
    pub env: f32,
}

impl Kick {
    pub fn new(sr: f32, seed: u32) -> Self {
        Kick { sr, v: [KickVoice::default(); 3], next: 0, noise: Noise::new(seed), hp: OnePole::new(2500.0, sr), env: 0.0 }
    }

    pub fn trigger(&mut self, vel: f32, f_start: f32, f_end: f32, decay: f32, click: f32, wait: u32) {
        let sr = self.sr;
        self.v[self.next] = KickVoice {
            on: true,
            wait,
            phase: 0.0,
            penv: 1.0,
            pmul: decay_mul(0.03, sr),
            amp: 1.0,
            amul: decay_mul(decay, sr),
            att: 0.0,
            click,
            cmul: decay_mul(0.0018, sr),
            f0: f_start,
            f1: f_end,
            vel,
        };
        self.next = (self.next + 1) % self.v.len();
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        let sr = self.sr;
        let w = self.hp.hp(self.noise.white());
        let mut out = 0.0;
        let mut env = 0.0f32;
        for v in self.v.iter_mut() {
            if !v.on {
                continue;
            }
            if v.wait > 0 {
                v.wait -= 1;
                continue;
            }
            let f = v.f1 + (v.f0 - v.f1) * v.penv;
            v.penv *= v.pmul;
            v.phase = wrap(v.phase + f / sr);
            if v.att < 1.0 {
                v.att = (v.att + 1.0 / (0.0015 * sr)).min(1.0);
            }
            let a = v.amp * v.att;
            let body = sine(v.phase) * a;
            out += (body + w * v.click * 0.5) * v.vel;
            v.click *= v.cmul;
            v.amp *= v.amul;
            env = env.max(a * v.vel);
            if v.amp < 1e-4 {
                v.on = false;
            }
        }
        self.env = env;
        out
    }
}

// ───────────────────────────── Sub ─────────────────────────────

#[derive(Clone, Copy, Debug)]
struct SubOsc {
    phase: f32,
    freq: f32,
    gain: Ramp,
}

/// Sustained sub-bass root; crossfades between two oscillators on root changes. The ears get a
/// gentle, slowly drifting phase offset (0.6–0.9 rad) so the low end isn't dead mono.
pub struct Sub {
    sr: f32,
    osc: [SubOsc; 2],
    cur: usize,
    spread: f32,
    pub level: Smooth,
}

impl Sub {
    pub fn new(sr: f32) -> Self {
        let o = SubOsc { phase: 0.0, freq: 55.0, gain: Ramp::new(0.0) };
        Sub { sr, osc: [o, o], cur: 0, spread: 0.0, level: Smooth::new(1.0, 1.5, sr) }
    }

    pub fn set_note(&mut self, midi: f32, xfade: f32) {
        let f = midi_hz(midi);
        let sr = self.sr;
        if (self.osc[self.cur].freq - f).abs() < 0.01 && self.osc[self.cur].gain.target() > 0.5 {
            return;
        }
        self.osc[self.cur].gain.go(0.0, xfade, sr);
        self.cur ^= 1;
        let o = &mut self.osc[self.cur];
        o.freq = f;
        o.gain.go(1.0, xfade * 1.3, sr);
    }

    #[inline]
    pub fn tick(&mut self) -> (f32, f32) {
        let lvl = self.level.next();
        let sr = self.sr;
        self.spread = wrap(self.spread + 0.011 / sr);
        let half = (0.75 + 0.15 * sine(self.spread)) / TAU * 0.5;
        let (mut l, mut r) = (0.0, 0.0);
        for o in self.osc.iter_mut() {
            let g = o.gain.next();
            if g <= 0.0 {
                continue;
            }
            o.phase = wrap(o.phase + o.freq / sr);
            let tone = |p: f32| sine(p) + 0.22 * sine(wrap(p * 2.0)) + 0.06 * sine(wrap(p * 3.0));
            l += g * tone(wrap(o.phase - half));
            r += g * tone(wrap(o.phase + half));
        }
        (l * lvl, r * lvl)
    }
}

// ───────────────────────────── Pads ─────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
struct PadVoice {
    note: f32,
    freq: f32,
    held: bool,
    env: Env,
    ph: [f32; 4],
    fl: Svf,
    fr: Svf,
    gl: f32,
    gr: f32,
    lfo: f32,
    lfo_inc: f32,
    det: f32,
}

/// Warm detuned saw/triangle pad, low-passed. Notes are held until released (smooth voice leading).
pub struct Pads {
    sr: f32,
    v: Vec<PadVoice>,
    pub cutoff: Smooth,
    pub level: Smooth,
    pub attack: f32,
    pub release: f32,
    pub width: f32,
    pub q: f32,
    /// 0 = all saw (rich), 1 = all triangle (soft).
    pub soft: f32,
    rng: Rng,
    ctrl: u32,
}

impl Pads {
    pub fn new(sr: f32, voices: usize, seed: u64) -> Self {
        Pads {
            sr,
            v: vec![PadVoice::default(); voices],
            cutoff: Smooth::new(1000.0, 1.5, sr),
            level: Smooth::new(1.0, 1.5, sr),
            attack: 4.0,
            release: 1.4,
            width: 0.8,
            q: 0.75,
            soft: 0.45,
            rng: Rng::new(seed),
            ctrl: 0,
        }
    }

    pub fn note_on(&mut self, note: f32, gain: f32) {
        if self.v.iter().any(|v| v.held && (v.note - note).abs() < 0.01) {
            return;
        }
        let idx = match self.v.iter().position(|v| !v.env.active()) {
            Some(i) => i,
            None => {
                let mut best = 0;
                let mut lvl = f32::MAX;
                for (i, v) in self.v.iter().enumerate() {
                    let l = v.env.level + if v.held { 10.0 } else { 0.0 };
                    if l < lvl {
                        lvl = l;
                        best = i;
                    }
                }
                best
            }
        };
        let pan = self.rng.range(-0.8, 0.8);
        let det = self.rng.range(0.0025, 0.0045);
        let lfo_inc = self.rng.range(0.05, 0.17) / self.sr;
        let phases = [self.rng.f32(), self.rng.f32(), self.rng.f32(), self.rng.f32()];
        let lfo = self.rng.f32();
        let (sr, attack, release) = (self.sr, self.attack, self.release);
        let v = &mut self.v[idx];
        if !v.env.active() {
            v.ph = phases;
            v.lfo = lfo;
        }
        v.note = note;
        v.freq = midi_hz(note);
        v.held = true;
        v.det = det;
        v.lfo_inc = lfo_inc;
        let (gl, gr) = pan_gains(pan);
        v.gl = gl * gain * std::f32::consts::SQRT_2;
        v.gr = gr * gain * std::f32::consts::SQRT_2;
        v.env.trigger(attack, 1.0, 1.0, release, sr);
    }

    pub fn note_off(&mut self, note: f32) {
        for v in self.v.iter_mut() {
            if v.held && (v.note - note).abs() < 0.01 {
                v.held = false;
                v.env.release();
            }
        }
    }

    /// Hold exactly `notes`: common tones keep sounding, others release, new ones fade in.
    pub fn set_chord(&mut self, notes: &[f32], gain: f32) {
        for v in self.v.iter_mut() {
            if v.held && !notes.iter().any(|&n| (n - v.note).abs() < 0.01) {
                v.held = false;
                v.env.release();
            }
        }
        for &n in notes {
            self.note_on(n, gain);
        }
    }

    #[inline]
    pub fn tick(&mut self, mix: &mut Mix, rev: f32) {
        self.ctrl = self.ctrl.wrapping_add(1);
        let update = self.ctrl % 32 == 0;
        let cutoff = self.cutoff.next();
        let level = self.level.next();
        let (sr, q, width, soft) = (self.sr, self.q, self.width, self.soft);
        for v in self.v.iter_mut() {
            if !v.env.active() {
                continue;
            }
            v.lfo = wrap(v.lfo + v.lfo_inc);
            let lfo = sine(v.lfo);
            let f = v.freq * (1.0 + 0.0017 * lfo);
            let dt0 = f * (1.0 - v.det) / sr;
            let dt1 = f * (1.0 + v.det) / sr;
            v.ph[0] = wrap(v.ph[0] + dt0);
            v.ph[1] = wrap(v.ph[1] + dt1);
            v.ph[2] = wrap(v.ph[2] + f * (1.0 - 0.4 * v.det) / sr);
            v.ph[3] = wrap(v.ph[3] + f * (1.0 + 0.4 * v.det) / sr);
            let a = saw(v.ph[0], dt0) * (1.0 - soft) + tri(v.ph[2]) * soft;
            let b = saw(v.ph[1], dt1) * (1.0 - soft) + tri(v.ph[3]) * soft;
            if update {
                let c = cutoff * (1.0 + 0.22 * lfo);
                v.fl.set(c, q, sr);
                v.fr.set(c * 1.04, q, sr);
            }
            let e = v.env.next() * level;
            let yl = v.fl.lp(a) * e;
            let yr = v.fr.lp(b) * e;
            let mid = (yl + yr) * 0.5;
            let side = (yl - yr) * 0.5 * width;
            mix.add((mid + side) * v.gl, (mid - side) * v.gr, rev, 0.0);
        }
    }
}

// ───────────────────────────── Bells (FM) ─────────────────────────────

/// Two-operator FM timbre.
#[derive(Clone, Copy, Debug)]
pub struct BellParams {
    pub ratio: f32,
    pub index: f32,
    pub index_tau: f32,
    pub attack: f32,
    pub decay: f32,
}

impl BellParams {
    pub const GLASS: Self = BellParams { ratio: 1.0, index: 0.55, index_tau: 1.2, attack: 0.012, decay: 2.6 };
    pub const BELL: Self = BellParams { ratio: 3.5, index: 0.9, index_tau: 0.4, attack: 0.004, decay: 2.4 };
    pub const KEYS: Self = BellParams { ratio: 1.0, index: 1.2, index_tau: 0.5, attack: 0.003, decay: 2.4 };
    pub const MALLET: Self = BellParams { ratio: 1.0, index: 0.45, index_tau: 0.1, attack: 0.006, decay: 0.7 };
    pub const PLUCK: Self = BellParams { ratio: 2.0, index: 0.55, index_tau: 0.07, attack: 0.004, decay: 0.45 };
}

#[derive(Clone, Copy, Debug, Default)]
struct BellVoice {
    wait: u32,
    freq: f32,
    pc: f32,
    pm: f32,
    ratio: f32,
    index: f32,
    idx_env: f32,
    idx_mul: f32,
    env: Env,
    vel: f32,
    gl: f32,
    gr: f32,
}

pub struct Bells {
    sr: f32,
    v: Vec<BellVoice>,
    /// Scales FM index: higher is brighter.
    pub bright: f32,
}

impl Bells {
    pub fn new(sr: f32, voices: usize) -> Self {
        Bells { sr, v: vec![BellVoice::default(); voices], bright: 1.0 }
    }

    pub fn play(&mut self, midi: f32, vel: f32, pan: f32, p: &BellParams, wait: u32) {
        let idx = match self.v.iter().position(|v| !v.env.active()) {
            Some(i) => i,
            None => {
                let mut best = 0;
                for (i, v) in self.v.iter().enumerate() {
                    if v.env.level < self.v[best].env.level {
                        best = i;
                    }
                }
                best
            }
        };
        let sr = self.sr;
        // Higher notes ring a little shorter, like real bars and tines.
        let decay = p.decay * (1.0 - ((midi - 72.0) / 48.0)).clamp(0.6, 1.4);
        let (gl, gr) = pan_gains(pan);
        let v = &mut self.v[idx];
        *v = BellVoice {
            wait,
            freq: midi_hz(midi),
            pc: 0.0,
            pm: 0.0,
            ratio: p.ratio,
            index: p.index,
            idx_env: 1.0,
            idx_mul: decay_mul(p.index_tau, sr),
            env: Env::default(),
            vel,
            gl: gl * std::f32::consts::SQRT_2,
            gr: gr * std::f32::consts::SQRT_2,
        };
        v.env.trigger(p.attack, decay, 0.0, 0.3, sr);
    }

    #[inline]
    pub fn tick(&mut self, pitch_mul: f32, mix: &mut Mix, rev: f32, dly: f32) {
        let sr = self.sr;
        let bright = self.bright;
        for v in self.v.iter_mut() {
            if !v.env.active() {
                continue;
            }
            if v.wait > 0 {
                v.wait -= 1;
                continue;
            }
            let f = v.freq * pitch_mul;
            v.pc = wrap(v.pc + f / sr);
            v.pm = wrap(v.pm + f * v.ratio / sr);
            let idx = v.index * bright * v.idx_env + 0.04;
            v.idx_env *= v.idx_mul;
            let y = (TAU * v.pc + idx * sine(v.pm)).sin() * v.env.next() * v.vel;
            mix.add(y * v.gl, y * v.gr, rev, dly);
        }
    }
}

// ───────────────────────────── Plucks ─────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
struct PluckVoice {
    on: bool,
    wait: u32,
    freq: f32,
    ph: f32,
    f: Svf,
    cenv: f32,
    cmul: f32,
    cbase: f32,
    cpeak: f32,
    amp: f32,
    amul: f32,
    att: f32,
    vel: f32,
    gl: f32,
    gr: f32,
    ctrl: u32,
}

/// Short filtered-saw plucks for arpeggios and stabs.
pub struct Plucks {
    sr: f32,
    v: Vec<PluckVoice>,
    pub bright: f32,
}

impl Plucks {
    pub fn new(sr: f32, voices: usize) -> Self {
        Plucks { sr, v: vec![PluckVoice::default(); voices], bright: 1.0 }
    }

    pub fn play(&mut self, midi: f32, vel: f32, pan: f32, decay: f32, wait: u32) {
        let idx = match self.v.iter().position(|v| !v.on) {
            Some(i) => i,
            None => {
                let mut best = 0;
                for (i, v) in self.v.iter().enumerate() {
                    if v.amp < self.v[best].amp {
                        best = i;
                    }
                }
                best
            }
        };
        let sr = self.sr;
        let freq = midi_hz(midi);
        let (gl, gr) = pan_gains(pan);
        self.v[idx] = PluckVoice {
            on: true,
            wait,
            freq,
            ph: 0.0,
            f: Svf::default(),
            cenv: 1.0,
            cmul: decay_mul(0.07, sr),
            cbase: freq * 1.3,
            cpeak: (freq * 9.0 * self.bright).min(6500.0),
            amp: 1.0,
            amul: decay_mul(decay, sr),
            att: 0.0,
            vel,
            gl: gl * std::f32::consts::SQRT_2,
            gr: gr * std::f32::consts::SQRT_2,
            ctrl: 0,
        };
    }

    #[inline]
    pub fn tick(&mut self, mix: &mut Mix, rev: f32, dly: f32) {
        let sr = self.sr;
        for v in self.v.iter_mut() {
            if !v.on {
                continue;
            }
            if v.wait > 0 {
                v.wait -= 1;
                continue;
            }
            let dt = v.freq / sr;
            v.ph = wrap(v.ph + dt);
            let osc = saw(v.ph, dt) * 0.7 + sine(v.ph) * 0.3;
            if v.ctrl % 16 == 0 {
                v.f.set(v.cbase + (v.cpeak - v.cbase) * v.cenv, 0.8, sr);
            }
            v.ctrl = v.ctrl.wrapping_add(1);
            v.cenv *= v.cmul;
            if v.att < 1.0 {
                v.att = (v.att + 1.0 / (0.002 * sr)).min(1.0);
            }
            let y = v.f.lp(osc) * v.amp * v.att * v.vel;
            v.amp *= v.amul;
            if v.amp < 1e-4 {
                v.on = false;
            }
            mix.add(y * v.gl, y * v.gr, rev, dly);
        }
    }
}

// ───────────────────────────── Hats ─────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
struct HatVoice {
    on: bool,
    wait: u32,
    amp: f32,
    mul: f32,
    vel: f32,
    gl: f32,
    gr: f32,
    bp: Svf,
    hp: OnePole,
    hp2: OnePole,
}

/// Band-passed noise ticks and hats.
pub struct Hats {
    sr: f32,
    v: [HatVoice; 8],
    next: usize,
    noise: Noise,
}

impl Hats {
    pub fn new(sr: f32, seed: u32) -> Self {
        Hats { sr, v: [HatVoice::default(); 8], next: 0, noise: Noise::new(seed) }
    }

    pub fn trigger(&mut self, vel: f32, decay: f32, pan: f32, center: f32, wait: u32) {
        let sr = self.sr;
        let (gl, gr) = pan_gains(pan);
        self.v[self.next] = HatVoice {
            on: true,
            wait,
            amp: 1.0,
            mul: decay_mul(decay, sr),
            vel,
            gl,
            gr,
            bp: Svf::new(center, 0.9, sr),
            hp: OnePole::new(center * 0.75, sr),
            hp2: OnePole::new(center * 0.75, sr),
        };
        self.next = (self.next + 1) % self.v.len();
    }

    #[inline]
    pub fn tick(&mut self, mix: &mut Mix, rev: f32) {
        let n = self.noise.white();
        for v in self.v.iter_mut() {
            if !v.on {
                continue;
            }
            if v.wait > 0 {
                v.wait -= 1;
                continue;
            }
            let y = v.hp2.hp(v.hp.hp(v.bp.bp(n))) * v.amp * v.vel;
            v.amp *= v.mul;
            if v.amp < 1e-4 {
                v.on = false;
            }
            mix.add(y * v.gl, y * v.gr, rev, 0.0);
        }
    }
}

// ───────────────────────────── Snap / rim / snare ─────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct SnapParams {
    pub center: f32,
    pub q: f32,
    pub decay: f32,
    pub tone: f32,
    pub tone_decay: f32,
    pub noise: f32,
}

impl SnapParams {
    pub const RIM: Self = SnapParams { center: 1800.0, q: 2.5, decay: 0.02, tone: 1250.0, tone_decay: 0.025, noise: 0.35 };
    pub const SNARE: Self = SnapParams { center: 1900.0, q: 0.8, decay: 0.1, tone: 190.0, tone_decay: 0.05, noise: 0.72 };
}

#[derive(Clone, Copy, Debug, Default)]
struct SnapVoice {
    on: bool,
    wait: u32,
    amp: f32,
    mul: f32,
    tamp: f32,
    tmul: f32,
    tph: f32,
    tone: f32,
    noise: f32,
    vel: f32,
    gl: f32,
    gr: f32,
    bp: Svf,
}

pub struct Snap {
    sr: f32,
    v: [SnapVoice; 4],
    next: usize,
    noise: Noise,
}

impl Snap {
    pub fn new(sr: f32, seed: u32) -> Self {
        Snap { sr, v: [SnapVoice::default(); 4], next: 0, noise: Noise::new(seed) }
    }

    pub fn trigger(&mut self, vel: f32, p: &SnapParams, pan: f32, wait: u32) {
        let sr = self.sr;
        let (gl, gr) = pan_gains(pan);
        self.v[self.next] = SnapVoice {
            on: true,
            wait,
            amp: 1.0,
            mul: decay_mul(p.decay, sr),
            tamp: 1.0,
            tmul: decay_mul(p.tone_decay, sr),
            tph: 0.0,
            tone: p.tone,
            noise: p.noise,
            vel,
            gl,
            gr,
            bp: Svf::new(p.center, p.q, sr),
        };
        self.next = (self.next + 1) % self.v.len();
    }

    #[inline]
    pub fn tick(&mut self, mix: &mut Mix, rev: f32, dly: f32) {
        let sr = self.sr;
        let n = self.noise.white();
        for v in self.v.iter_mut() {
            if !v.on {
                continue;
            }
            if v.wait > 0 {
                v.wait -= 1;
                continue;
            }
            v.tph = wrap(v.tph + v.tone / sr);
            let y = (v.bp.bp(n) * 1.6 * v.amp * v.noise + sine(v.tph) * v.tamp * (1.0 - v.noise)) * v.vel;
            v.amp *= v.mul;
            v.tamp *= v.tmul;
            if v.amp < 1e-4 && v.tamp < 1e-4 {
                v.on = false;
            }
            mix.add(y * v.gl, y * v.gr, rev, dly);
        }
    }
}

// ───────────────────────────── Bass ─────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
struct BassVoice {
    on: bool,
    wait: u32,
    freq: f32,
    ph: f32,
    ph_r: f32,
    f: Svf,
    f_r: Svf,
    cenv: f32,
    cmul: f32,
    amp: f32,
    amul: f32,
    att: f32,
    vel: f32,
    ctrl: u32,
}

/// Plucked mono-ish bass (saw + sine through an enveloped low-pass).
pub struct Bass {
    sr: f32,
    v: [BassVoice; 4],
    next: usize,
    pub cutoff: f32,
    /// 0 = pure sine, 1 = full saw.
    pub grit: f32,
    /// Relative detune between ears (0 = mono).
    pub detune: f32,
}

impl Bass {
    pub fn new(sr: f32) -> Self {
        Bass { sr, v: [BassVoice::default(); 4], next: 0, cutoff: 260.0, grit: 0.5, detune: 0.0 }
    }

    pub fn play(&mut self, midi: f32, vel: f32, decay: f32, wait: u32) {
        let sr = self.sr;
        self.v[self.next] = BassVoice {
            on: true,
            wait,
            freq: midi_hz(midi),
            ph: 0.0,
            ph_r: 0.0,
            f: Svf::default(),
            f_r: Svf::default(),
            cenv: 1.0,
            cmul: decay_mul(0.06, sr),
            amp: 1.0,
            amul: decay_mul(decay, sr),
            att: 0.0,
            vel,
            ctrl: 0,
        };
        self.next = (self.next + 1) % self.v.len();
    }

    #[inline]
    pub fn tick(&mut self) -> (f32, f32) {
        let sr = self.sr;
        let (cut, grit, det) = (self.cutoff, self.grit, self.detune);
        let (mut l, mut r) = (0.0, 0.0);
        for v in self.v.iter_mut() {
            if !v.on {
                continue;
            }
            if v.wait > 0 {
                v.wait -= 1;
                continue;
            }
            let dt = v.freq * (1.0 - det) / sr;
            v.ph = wrap(v.ph + dt);
            let osc = saw(v.ph, dt) * grit + sine(v.ph) * (1.0 - grit * 0.5);
            if v.ctrl % 16 == 0 {
                v.f.set(cut * (1.0 + 2.5 * v.cenv), 0.9, sr);
                v.f_r.set(cut * (1.0 + 2.5 * v.cenv), 0.9, sr);
            }
            v.ctrl = v.ctrl.wrapping_add(1);
            v.cenv *= v.cmul;
            if v.att < 1.0 {
                v.att = (v.att + 1.0 / (0.004 * sr)).min(1.0);
            }
            let g = v.amp * v.att * v.vel;
            let yl = v.f.lp(osc) * g;
            let yr = if det == 0.0 {
                yl
            } else {
                let dtr = v.freq * (1.0 + det) / sr;
                v.ph_r = wrap(v.ph_r + dtr);
                v.f_r.lp(saw(v.ph_r, dtr) * grit + sine(v.ph_r) * (1.0 - grit * 0.5)) * g
            };
            l += yl;
            r += yr;
            v.amp *= v.amul;
            if v.amp < 1e-4 {
                v.on = false;
            }
        }
        (l, r)
    }
}

// ───────────────────────────── Air (noise texture) ─────────────────────────────

/// Very quiet band-passed pink noise "air" that keeps the stereo field alive.
pub struct Air {
    nl: Noise,
    nr: Noise,
    pl: Pink,
    pr: Pink,
    fl: Svf,
    fr: Svf,
    lfo: f32,
    lfo_inc: f32,
    pub level: Smooth,
}

impl Air {
    pub fn new(sr: f32, seed: u32, center: f32) -> Self {
        Air {
            nl: Noise::new(seed),
            nr: Noise::new(seed.wrapping_mul(2654435761).max(1)),
            pl: Pink::default(),
            pr: Pink::default(),
            fl: Svf::new(center, 0.45, sr),
            fr: Svf::new(center * 1.1, 0.45, sr),
            lfo: 0.0,
            lfo_inc: 0.031 / sr,
            level: Smooth::new(0.0, 3.0, sr),
        }
    }

    #[inline]
    pub fn tick(&mut self, mix: &mut Mix) {
        let lvl = self.level.next();
        if lvl < 1e-6 {
            return;
        }
        self.lfo = wrap(self.lfo + self.lfo_inc);
        let m = 0.75 + 0.25 * sine(self.lfo);
        let a = self.fl.bp(self.pl.tick(self.nl.white()));
        let b = self.fr.bp(self.pr.tick(self.nr.white()));
        let l = (a * 0.8 + b * 0.45) * lvl * m;
        let r = (b * 0.8 + a * 0.45) * lvl * (1.75 - m);
        mix.add(l, r, 0.1, 0.0);
    }
}

// ───────────────────────────── Clap ─────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
struct ClapVoice {
    on: bool,
    wait: u32,
    amp: f32,
    mul: f32,
    tail: f32,
    tail_mul: f32,
    vel: f32,
    bp: [Svf; 2],
    edge: [Svf; 2],
}

/// Short, wide backbeat snap: band-passed noise around ~1.2–1.5 kHz plus a 5 kHz noise edge,
/// independent per ear. Falls 10 dB in ~12 ms like the reference, then a faint tail.
pub struct Clap {
    sr: f32,
    v: [ClapVoice; 4],
    next: usize,
    noise: [Noise; 2],
}

impl Clap {
    pub fn new(sr: f32, seed: u32) -> Self {
        Clap {
            sr,
            v: [ClapVoice::default(); 4],
            next: 0,
            noise: [Noise::new(seed | 1), Noise::new(seed.rotate_left(16) | 1)],
        }
    }

    pub fn trigger(&mut self, vel: f32, center: f32, wait: u32) {
        let sr = self.sr;
        self.v[self.next] = ClapVoice {
            on: true,
            wait,
            amp: 1.0,
            mul: decay_mul(0.0105, sr),
            tail: 1.0,
            tail_mul: decay_mul(0.045, sr),
            vel,
            bp: [Svf::new(center, 0.8, sr), Svf::new(center * 1.06, 0.8, sr)],
            edge: [Svf::new(5000.0, 0.6, sr), Svf::new(5300.0, 0.6, sr)],
        };
        self.next = (self.next + 1) % self.v.len();
    }

    #[inline]
    pub fn tick(&mut self, mix: &mut Mix, rev: f32) {
        let n = [self.noise[0].white(), self.noise[1].white()];
        for v in self.v.iter_mut() {
            if !v.on {
                continue;
            }
            if v.wait > 0 {
                v.wait -= 1;
                continue;
            }
            let env = (v.amp + 0.18 * v.tail) * v.vel;
            let mut y = [0f32; 2];
            for c in 0..2 {
                y[c] = (v.bp[c].bp(n[c]) * 1.4 + v.edge[c].bp(n[c]) * 0.9) * env;
            }
            v.amp *= v.mul;
            v.tail *= v.tail_mul;
            if v.tail < 1e-4 {
                v.on = false;
            }
            mix.add(y[0], y[1], rev, 0.0);
        }
    }
}

// ───────────────────────────── Riser ─────────────────────────────

/// Filtered-noise swell leading into a section change.
pub struct Riser {
    sr: f32,
    on: bool,
    t: f32,
    dur: f32,
    vel: f32,
    nl: Noise,
    nr: Noise,
    fl: Svf,
    fr: Svf,
    ctrl: u32,
}

impl Riser {
    pub fn new(sr: f32, seed: u32) -> Self {
        Riser {
            sr,
            on: false,
            t: 0.0,
            dur: 1.0,
            vel: 0.0,
            nl: Noise::new(seed),
            nr: Noise::new(seed ^ 0x5bd1e995),
            fl: Svf::default(),
            fr: Svf::default(),
            ctrl: 0,
        }
    }

    pub fn start(&mut self, dur: f32, vel: f32) {
        self.on = true;
        self.t = 0.0;
        self.dur = dur.max(0.1);
        self.vel = vel;
    }

    #[inline]
    pub fn tick(&mut self, mix: &mut Mix) {
        if !self.on {
            return;
        }
        let x = self.t / self.dur;
        if x >= 1.0 {
            self.on = false;
            return;
        }
        if self.ctrl % 32 == 0 {
            let c = 300.0 * (20.0f32).powf(x);
            self.fl.set(c, 1.2, self.sr);
            self.fr.set(c * 1.07, 1.2, self.sr);
        }
        self.ctrl = self.ctrl.wrapping_add(1);
        let a = self.vel * x.powf(2.5) * (1.0 - ((x - 0.97) / 0.03).clamp(0.0, 1.0));
        let l = self.fl.bp(self.nl.white()) * a;
        let r = self.fr.bp(self.nr.white()) * a;
        self.t += 1.0 / self.sr;
        mix.add(l, r, 0.4, 0.0);
    }
}

// ───────────────────────────── Vinyl crackle ─────────────────────────────

/// Dust clicks plus faint hiss, for the lo-fi Study soundscape.
pub struct Crackle {
    sr: f32,
    noise: Noise,
    hiss_n: Noise,
    pink: Pink,
    hp: OnePole,
    lp: OnePole,
    pub rate: f32,
    pub level: f32,
    pub hiss: f32,
    pan: (f32, f32),
    rng: Rng,
}

impl Crackle {
    pub fn new(sr: f32, seed: u64) -> Self {
        Crackle {
            sr,
            noise: Noise::new(seed as u32 | 1),
            hiss_n: Noise::new((seed >> 32) as u32 | 1),
            pink: Pink::default(),
            hp: OnePole::new(1200.0, sr),
            lp: OnePole::new(7000.0, sr),
            rate: 6.0,
            level: 0.05,
            hiss: 0.003,
            pan: (0.7, 0.7),
            rng: Rng::new(seed),
        }
    }

    #[inline]
    pub fn tick(&mut self, mix: &mut Mix) {
        let mut imp = 0.0;
        if self.rng.f32() < self.rate / self.sr {
            let a = self.rng.f32();
            imp = a * a * a * if self.rng.chance(0.5) { 1.0 } else { -1.0 };
            self.pan = pan_gains(self.rng.range(-0.8, 0.8));
        }
        let _ = self.noise.white();
        let c = self.lp.lp(self.hp.hp(imp)) * self.level * 6.0;
        let h = self.pink.tick(self.hiss_n.white()) * self.hiss;
        mix.add(c * self.pan.0 + h, c * self.pan.1 + h, 0.0, 0.0);
    }
}
