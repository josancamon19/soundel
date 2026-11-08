//! Binaural: a carrier tone in each ear, offset by the beat frequency (40 Hz gamma by default),
//! under a dark pentatonic pad and a faint pink-noise bed. Needs headphones to work.

use crate::dsp::reverb::Reverb;
use crate::dsp::*;
use crate::engine::{Generator, Status};
use crate::rng::Rng;
use crate::voices::*;

const CTRL: u32 = 64;
const CARRIER: f32 = 200.0;
const CHORDS: [[f32; 4]; 4] = [
    [48.0, 55.0, 62.0, 64.0],
    [45.0, 52.0, 55.0, 62.0],
    [43.0, 50.0, 57.0, 62.0],
    [50.0, 57.0, 60.0, 64.0],
];

pub struct Binaural {
    sr: f32,
    rng: Rng,
    beat: f32,
    ph: [f32; 2],
    pads: Pads,
    noise: [Noise; 2],
    pink: [Pink; 2],
    reverb: Reverb,
    master: Ramp,
    tone_gain: Ramp,
    x: Smooth,
    y: Smooth,
    chord: usize,
    chord_wait: i64,
    ctrl: u32,
}

impl Binaural {
    pub fn new(sr: f32, seed: u64, beat: f32, x: f32, y: f32) -> Self {
        let mut rng = Rng::new(seed);
        let ctrl_rate = sr / CTRL as f32;
        let s = rng.next_u64();
        let mut b = Binaural {
            sr,
            beat: beat.clamp(0.5, 60.0),
            ph: [0.0; 2],
            pads: Pads::new(sr, 8, rng.next_u64()),
            noise: [Noise::new(s as u32 | 1), Noise::new((s >> 32) as u32 | 1)],
            pink: [Pink::default(); 2],
            reverb: Reverb::new(sr, 3.5, 1.1, 3500.0),
            master: Ramp::new(0.0),
            tone_gain: Ramp::new(0.0),
            x: Smooth::new(x, 1.5, ctrl_rate),
            y: Smooth::new(y, 1.5, ctrl_rate),
            chord: 0,
            chord_wait: (sr * rng.range(30.0, 60.0)) as i64,
            ctrl: 0,
            rng,
        };
        b.pads.attack = 8.0;
        b.pads.release = 3.0;
        b.pads.soft = 0.8;
        b.pads.set_chord(&CHORDS[0], 1.0);
        b.master.go(1.0, 6.0, sr);
        b.tone_gain.go(1.0, 12.0, sr);
        b
    }

    fn band(&self) -> &'static str {
        match self.beat {
            b if b < 4.0 => "delta",
            b if b < 8.0 => "theta",
            b if b < 13.0 => "alpha",
            b if b < 30.0 => "beta",
            _ => "gamma",
        }
    }

    fn control(&mut self) {
        let x = self.x.next();
        self.y.next();
        self.pads.cutoff.set(450.0 + 700.0 * x);
        self.pads.level.set(0.03);
        self.chord_wait -= CTRL as i64;
        if self.chord_wait <= 0 {
            self.chord_wait = (self.sr * self.rng.range(30.0, 60.0)) as i64;
            let mut c = self.chord;
            while c == self.chord {
                c = self.rng.int(0, CHORDS.len() as i32 - 1) as usize;
            }
            self.chord = c;
            self.pads.set_chord(&CHORDS[c], 1.0);
        }
    }
}

impl Generator for Binaural {
    fn render(&mut self, l: &mut [f32], r: &mut [f32]) {
        let sr = self.sr;
        let freqs = [CARRIER, CARRIER + self.beat];
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            if self.ctrl == 0 {
                self.control();
            }
            self.ctrl = (self.ctrl + 1) % CTRL;
            let mut mix = Mix::default();
            self.pads.tick(&mut mix, 0.5);
            let (rl, rr) = self.reverb.tick(mix.rev.0, mix.rev.1);
            let tg = self.tone_gain.next() * 0.085;
            let bed = 0.008 * (0.4 + 1.2 * self.y.value);
            let mut out = [mix.dry.0 + rl * 0.35, mix.dry.1 + rr * 0.35];
            for c in 0..2 {
                self.ph[c] = wrap(self.ph[c] + freqs[c] / sr);
                out[c] += sine(self.ph[c]) * tg + self.pink[c].tick(self.noise[c].white()) * bed;
            }
            let g = self.master.next();
            *a = out[0] * g;
            *b = out[1] * g;
        }
    }

    fn tune(&mut self, x: f32, y: f32) {
        self.x.set(x.clamp(0.0, 1.0));
        self.y.set(y.clamp(0.0, 1.0));
    }

    fn status(&self, st: &mut Status) {
        st.mode = "binaural";
        st.key = self.band();
        st.root = "";
        st.section = "headphones on";
        st.beat = "";
        st.phase = "";
        st.bpm = 0.0;
        st.value = self.beat;
        st.value_unit = "Hz beat";
        st.x = self.x.target;
        st.y = self.y.target;
        st.x_labels = ("Dark", "Warm");
        st.y_labels = ("Quiet bed", "Airy bed");
    }
}
