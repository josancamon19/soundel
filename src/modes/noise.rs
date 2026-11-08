//! Colored noise. Endel's version is fully decorrelated between ears (L/R correlation ≈ 0),
//! steady (~1.3 dB short-term variation) and, at the center of its pad, darker than brown noise.

use clap::ValueEnum;

use crate::dsp::*;
use crate::engine::{Generator, Status};
use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum NoiseColor {
    Deep,
    Brown,
    Pink,
    Grey,
    White,
    Blue,
    Violet,
    Green,
}

impl NoiseColor {
    /// Position on the dark→bright continuum the arrow keys move along (None = off-continuum color).
    pub fn position(self) -> Option<f32> {
        match self {
            NoiseColor::Deep => Some(0.0),
            NoiseColor::Brown => Some(0.2),
            NoiseColor::Pink => Some(0.4),
            NoiseColor::Grey => Some(0.6),
            NoiseColor::White => Some(0.8),
            NoiseColor::Blue => Some(1.0),
            NoiseColor::Violet | NoiseColor::Green => None,
        }
    }
}

const CONTINUUM: [&str; 6] = ["deep", "brown", "pink", "grey", "white", "blue"];
const CTRL: u32 = 64;
const TARGET_RMS: f32 = 0.068;

struct Chan {
    w: Noise,
    pink: Pink,
    brown: Brown,
    deep: Svf,
    grey_lo: Biquad,
    grey_hi: Biquad,
    green: Svf,
    prev_p: f32,
    prev_w: f32,
}

impl Chan {
    fn new(sr: f32, seed: u32) -> Self {
        let mut grey_lo = Biquad::default();
        grey_lo.set_low_shelf(250.0, 6.0, sr);
        let mut grey_hi = Biquad::default();
        grey_hi.set_high_shelf(6000.0, 4.0, sr);
        Chan {
            w: Noise::new(seed),
            pink: Pink::default(),
            brown: Brown::default(),
            deep: Svf::new(1000.0, 0.7, sr),
            grey_lo,
            grey_hi,
            green: Svf::new(500.0, 0.5, sr),
            prev_p: 0.0,
            prev_w: 0.0,
        }
    }

    /// deep, brown, pink, grey, white, blue, violet, green
    #[inline]
    fn tick(&mut self) -> [f32; 8] {
        let w = self.w.white();
        let p = self.pink.tick(w);
        let b = self.brown.tick(w);
        let d = self.deep.lp(b);
        let g = self.grey_hi.tick(self.grey_lo.tick(p));
        let blue = p - self.prev_p;
        self.prev_p = p;
        let violet = w - self.prev_w;
        self.prev_w = w;
        let green = self.green.bp(p);
        [d, b, p, g, w, blue, violet, green]
    }
}

pub struct NoiseGen {
    color: NoiseColor,
    pos: Smooth,
    y: Smooth,
    x_target: f32,
    ch: [Chan; 2],
    gains: [f32; 8],
    lfo: [f32; 3],
    lfo_inc: [f32; 3],
    mod_gain: [f32; 2],
    master: Ramp,
    ctrl: u32,
}

impl NoiseGen {
    pub fn new(sr: f32, seed: u64, color: NoiseColor, x: f32, y: f32) -> Self {
        let mut rng = Rng::new(seed);
        let s = rng.next_u64();
        let gains = Self::calibrate(sr, s as u32 | 1);
        let pos = color.position().map(|_| x).unwrap_or(0.0);
        let ctrl_rate = sr / CTRL as f32;
        let mut n = NoiseGen {
            color,
            pos: Smooth::new(pos, 0.8, ctrl_rate),
            y: Smooth::new(y, 1.0, ctrl_rate),
            x_target: x,
            ch: [Chan::new(sr, s as u32 | 1), Chan::new(sr, (s >> 32) as u32 | 1)],
            gains,
            lfo: [rng.f32(), rng.f32(), rng.f32()],
            lfo_inc: [
                CTRL as f32 / (sr * rng.range(14.0, 22.0)),
                CTRL as f32 / (sr * rng.range(17.0, 27.0)),
                CTRL as f32 / (sr * rng.range(40.0, 70.0)),
            ],
            mod_gain: [1.0, 1.0],
            master: Ramp::new(0.0),
            ctrl: 0,
        };
        n.master.go(1.0, 4.0, sr);
        n
    }

    /// Normalize every color to the same RMS so moving across the pad doesn't jump in level.
    fn calibrate(sr: f32, seed: u32) -> [f32; 8] {
        let mut c = Chan::new(sr, seed);
        let mut acc = [0f64; 8];
        let n = sr as usize;
        for _ in 0..n / 2 {
            c.tick();
        }
        for _ in 0..n {
            let s = c.tick();
            for i in 0..8 {
                acc[i] += (s[i] as f64) * (s[i] as f64);
            }
        }
        let mut g = [1f32; 8];
        for i in 0..8 {
            let rms = (acc[i] / n as f64).sqrt() as f32;
            g[i] = if rms > 1e-9 { TARGET_RMS / rms } else { 0.0 };
        }
        g
    }

    fn control(&mut self) {
        self.pos.next();
        let y = self.y.next();
        let depth = 0.4 + 2.6 * y;
        for i in 0..3 {
            self.lfo[i] = wrap(self.lfo[i] + self.lfo_inc[i]);
        }
        let swell = sine(self.lfo[2]) * depth * 0.5;
        self.mod_gain[0] = db(sine(self.lfo[0]) * depth + swell);
        self.mod_gain[1] = db(sine(self.lfo[1]) * depth + swell);
    }

    fn color_name(&self) -> &'static str {
        match self.color.position() {
            Some(_) => CONTINUUM[(self.pos.value * 5.0).round().clamp(0.0, 5.0) as usize],
            None => match self.color {
                NoiseColor::Violet => "violet",
                _ => "green",
            },
        }
    }
}

impl Generator for NoiseGen {
    fn render(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            if self.ctrl == 0 {
                self.control();
            }
            self.ctrl = (self.ctrl + 1) % CTRL;
            let g = self.master.next();
            let pos = self.pos.value.clamp(0.0, 1.0) * 5.0;
            let i = (pos.floor() as usize).min(4);
            let f = pos - i as f32;
            for (c, out) in [a, b].into_iter().enumerate() {
                let s = self.ch[c].tick();
                let v = match self.color {
                    NoiseColor::Violet => s[6] * self.gains[6],
                    NoiseColor::Green => s[7] * self.gains[7],
                    _ => s[i] * self.gains[i] * (1.0 - f) + s[i + 1] * self.gains[i + 1] * f,
                };
                *out = v * self.mod_gain[c] * g;
            }
        }
    }

    fn tune(&mut self, x: f32, y: f32) {
        self.x_target = x.clamp(0.0, 1.0);
        if self.color.position().is_some() {
            self.pos.set(self.x_target);
        }
        self.y.set(y.clamp(0.0, 1.0));
    }

    fn status(&self, st: &mut Status) {
        st.mode = "noise";
        st.key = "";
        st.root = "";
        st.section = self.color_name();
        st.beat = "";
        st.phase = "";
        st.bpm = 0.0;
        st.x = self.x_target;
        st.y = self.y.target;
        st.x_labels = ("Dark", "Bright");
        st.y_labels = ("Still", "Moving");
    }
}
