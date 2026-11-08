//! Core DSP building blocks: oscillators, filters, envelopes, noise, smoothing.

pub mod fx;
pub mod reverb;

use std::f32::consts::PI;
pub const TAU: f32 = std::f32::consts::TAU;

#[inline]
pub fn midi_hz(m: f32) -> f32 {
    440.0 * (2.0f32).powf((m - 69.0) / 12.0)
}

#[inline]
pub fn hz_midi(f: f32) -> f32 {
    69.0 + 12.0 * (f / 440.0).log2()
}

#[inline]
pub fn db(d: f32) -> f32 {
    (10.0f32).powf(d / 20.0)
}

/// Equal-power pan gains for p in [-1, 1] (0.707 each at center).
#[inline]
pub fn pan_gains(p: f32) -> (f32, f32) {
    let a = (p.clamp(-1.0, 1.0) + 1.0) * PI * 0.25;
    (a.cos(), a.sin())
}

/// One-pole smoothing coefficient for a time constant.
#[inline]
pub fn tau_coef(tau_s: f32, rate: f32) -> f32 {
    if tau_s <= 0.0 {
        1.0
    } else {
        1.0 - (-1.0 / (tau_s * rate)).exp()
    }
}

/// Per-sample multiplier that realizes exp(-t / tau).
#[inline]
pub fn decay_mul(tau_s: f32, sr: f32) -> f32 {
    (-1.0 / (tau_s.max(1e-4) * sr)).exp()
}

#[inline]
pub fn wrap(p: f32) -> f32 {
    p - p.floor()
}

#[inline]
pub fn sine(phase: f32) -> f32 {
    (TAU * phase).sin()
}

#[inline]
pub fn tri(phase: f32) -> f32 {
    4.0 * (phase - 0.5).abs() - 1.0
}

#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Band-limited sawtooth (polyBLEP).
#[inline]
pub fn saw(phase: f32, dt: f32) -> f32 {
    2.0 * phase - 1.0 - poly_blep(phase, dt)
}

/// One-pole parameter smoother.
#[derive(Clone, Copy, Debug)]
pub struct Smooth {
    pub value: f32,
    pub target: f32,
    coef: f32,
}

impl Smooth {
    /// `rate` is how often `next` is called per second (sample rate, or control rate).
    pub fn new(value: f32, tau_s: f32, rate: f32) -> Self {
        Smooth { value, target: value, coef: tau_coef(tau_s, rate) }
    }
    pub fn set(&mut self, t: f32) {
        self.target = t;
    }
    pub fn snap(&mut self, v: f32) {
        self.value = v;
        self.target = v;
    }
    #[inline]
    pub fn next(&mut self) -> f32 {
        self.value += (self.target - self.value) * self.coef;
        self.value
    }
}

/// Linear ramp with an exact duration (fades).
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    pub value: f32,
    target: f32,
    step: f32,
}

impl Ramp {
    pub fn new(v: f32) -> Self {
        Ramp { value: v, target: v, step: 0.0 }
    }
    pub fn go(&mut self, target: f32, secs: f32, sr: f32) {
        self.target = target;
        let n = secs * sr;
        if n < 1.0 {
            self.value = target;
            self.step = 0.0;
        } else {
            self.step = (target - self.value) / n;
        }
    }
    pub fn target(&self) -> f32 {
        self.target
    }
    pub fn done(&self) -> bool {
        self.step == 0.0
    }
    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.step != 0.0 {
            self.value += self.step;
            if (self.step > 0.0 && self.value >= self.target) || (self.step < 0.0 && self.value <= self.target) {
                self.value = self.target;
                self.step = 0.0;
            }
        }
        self.value
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

/// Exponential envelope: curved attack, decay toward sustain, exponential release.
#[derive(Clone, Copy, Debug)]
pub struct Env {
    pub stage: Stage,
    pub level: f32,
    att_coef: f32,
    dec_mul: f32,
    sustain: f32,
    rel_mul: f32,
}

impl Default for Env {
    fn default() -> Self {
        Env { stage: Stage::Idle, level: 0.0, att_coef: 1.0, dec_mul: 0.999, sustain: 0.0, rel_mul: 0.999 }
    }
}

impl Env {
    /// `decay_tau` and `release_tau` are exponential time constants (T60 ≈ 6.9 × tau).
    pub fn trigger(&mut self, attack: f32, decay_tau: f32, sustain: f32, release_tau: f32, sr: f32) {
        self.dec_mul = decay_mul(decay_tau, sr);
        self.sustain = sustain;
        self.rel_mul = decay_mul(release_tau, sr);
        if attack <= 0.0005 {
            self.level = 1.0;
            self.stage = Stage::Decay;
        } else {
            // Approach 1.25 so the curve reaches 1.0 at `attack` seconds.
            self.att_coef = tau_coef(attack / 1.609, sr);
            self.stage = Stage::Attack;
        }
    }
    pub fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }
    #[inline]
    pub fn active(&self) -> bool {
        self.stage != Stage::Idle
    }
    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => 0.0,
            Stage::Attack => {
                self.level += (1.25 - self.level) * self.att_coef;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
                self.level
            }
            Stage::Decay => {
                self.level = self.sustain + (self.level - self.sustain) * self.dec_mul;
                if self.sustain <= 0.0 && self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
                self.level
            }
            Stage::Release => {
                self.level *= self.rel_mul;
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
                self.level
            }
        }
    }
}

/// Zavalishin topology-preserving-transform state variable filter.
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

pub struct SvfOut {
    pub lp: f32,
    pub bp: f32,
}

impl Svf {
    pub fn new(cutoff: f32, q: f32, sr: f32) -> Self {
        let mut s = Svf::default();
        s.set(cutoff, q, sr);
        s
    }
    #[inline]
    pub fn set(&mut self, cutoff: f32, q: f32, sr: f32) {
        let fc = cutoff.clamp(5.0, sr * 0.45);
        let g = (PI * fc / sr).tan();
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    #[inline]
    pub fn tick(&mut self, v0: f32) -> SvfOut {
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut { lp: v2, bp: v1 }
    }
    #[inline]
    pub fn lp(&mut self, x: f32) -> f32 {
        self.tick(x).lp
    }
    #[inline]
    pub fn bp(&mut self, x: f32) -> f32 {
        self.tick(x).bp
    }
}

/// One-pole lowpass / highpass.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePole {
    z: f32,
    a: f32,
}

impl OnePole {
    pub fn new(cutoff: f32, sr: f32) -> Self {
        let mut o = OnePole::default();
        o.set(cutoff, sr);
        o
    }
    pub fn set(&mut self, cutoff: f32, sr: f32) {
        self.a = 1.0 - (-TAU * cutoff.clamp(1.0, sr * 0.49) / sr).exp();
    }
    #[inline]
    pub fn lp(&mut self, x: f32) -> f32 {
        self.z += self.a * (x - self.z);
        self.z
    }
    #[inline]
    pub fn hp(&mut self, x: f32) -> f32 {
        x - self.lp(x)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.9975 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// RBJ shelving biquad (transposed direct form II).
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }
}

impl Biquad {
    fn coeffs(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }
    pub fn set_high_shelf(&mut self, f: f32, gain_db: f32, sr: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let (sn, cs) = (TAU * f / sr).sin_cos();
        let sa = 2.0 * a.sqrt() * sn / 2.0 * std::f32::consts::SQRT_2;
        self.coeffs(
            a * ((a + 1.0) + (a - 1.0) * cs + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
            a * ((a + 1.0) + (a - 1.0) * cs - sa),
            (a + 1.0) - (a - 1.0) * cs + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * cs),
            (a + 1.0) - (a - 1.0) * cs - sa,
        );
    }
    pub fn set_low_shelf(&mut self, f: f32, gain_db: f32, sr: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let (sn, cs) = (TAU * f / sr).sin_cos();
        let sa = 2.0 * a.sqrt() * sn / 2.0 * std::f32::consts::SQRT_2;
        self.coeffs(
            a * ((a + 1.0) - (a - 1.0) * cs + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
            a * ((a + 1.0) - (a - 1.0) * cs - sa),
            (a + 1.0) + (a - 1.0) * cs + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cs),
            (a + 1.0) + (a - 1.0) * cs - sa,
        );
    }
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// xorshift32 white noise in [-1, 1).
#[derive(Clone, Copy, Debug)]
pub struct Noise {
    s: u32,
}

impl Noise {
    pub fn new(seed: u32) -> Self {
        Noise { s: seed.max(1) }
    }
    #[inline]
    pub fn white(&mut self) -> f32 {
        let mut x = self.s;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.s = x;
        (x as f32) * (2.0 / u32::MAX as f32) - 1.0
    }
}

/// Pink noise filter (Paul Kellet's refined method).
#[derive(Clone, Copy, Debug, Default)]
pub struct Pink {
    b: [f32; 7],
}

impl Pink {
    #[inline]
    pub fn tick(&mut self, w: f32) -> f32 {
        let b = &mut self.b;
        b[0] = 0.99886 * b[0] + w * 0.0555179;
        b[1] = 0.99332 * b[1] + w * 0.0750759;
        b[2] = 0.96900 * b[2] + w * 0.1538520;
        b[3] = 0.86650 * b[3] + w * 0.3104856;
        b[4] = 0.55000 * b[4] + w * 0.5329522;
        b[5] = -0.7616 * b[5] - w * 0.0168980;
        let out = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
        b[6] = w * 0.115926;
        out * 0.11
    }
}

/// Brown (red) noise: leaky integrated white noise.
#[derive(Clone, Copy, Debug, Default)]
pub struct Brown {
    z: f32,
}

impl Brown {
    #[inline]
    pub fn tick(&mut self, w: f32) -> f32 {
        self.z = (self.z + w * 0.02) * 0.998;
        self.z * 3.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svf_stays_bounded_under_sweeps() {
        let sr = 48_000.0;
        let mut f = Svf::new(100.0, 0.7, sr);
        let mut n = Noise::new(3);
        for i in 0..96_000 {
            let c = 20.0 + (i % 4800) as f32 * 4.0;
            f.set(c, 0.5 + (i % 7) as f32 * 0.3, sr);
            let y = f.lp(n.white());
            assert!(y.is_finite() && y.abs() < 10.0);
        }
    }

    #[test]
    fn envelope_attacks_and_releases() {
        let sr = 1000.0;
        let mut e = Env::default();
        e.trigger(0.1, 1.0, 1.0, 0.05, sr);
        let mut peak_at = 0;
        for i in 0..200 {
            if e.next() >= 1.0 && peak_at == 0 {
                peak_at = i;
            }
        }
        assert!((90..=110).contains(&peak_at), "attack reached 1.0 at {peak_at}");
        e.release();
        for _ in 0..2000 {
            e.next();
        }
        assert!(!e.active());
    }

    #[test]
    fn ramp_hits_target_exactly() {
        let mut r = Ramp::new(0.0);
        r.go(1.0, 0.01, 1000.0);
        for _ in 0..10 {
            r.next();
        }
        assert_eq!(r.value, 1.0);
        assert!(r.done());
    }

    #[test]
    fn midi_conversion_round_trips() {
        assert!((midi_hz(69.0) - 440.0).abs() < 1e-3);
        assert!((hz_midi(528.0) - 72.156).abs() < 0.01);
    }
}
