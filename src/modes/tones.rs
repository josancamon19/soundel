//! Tones: a slow pentatonic pad tuned so its tonic is a chosen frequency (528 Hz by default).
//! Endel's solfeggio soundscape measured exactly that: a strong 528 Hz tone over a C pentatonic
//! pad tuned sharp (A4 ≈ 444 Hz) so that C5 lands on 528 Hz, with drones at 66 and 132 Hz.

use crate::dsp::reverb::Reverb;
use crate::dsp::*;
use crate::engine::{Generator, Status};
use crate::music::PENTA;
use crate::rng::Rng;
use crate::voices::*;

const CTRL: u32 = 64;

pub struct Tones {
    sr: f32,
    rng: Rng,
    f0: f32,
    m0: f32,
    pads: Pads,
    bells: Bells,
    reverb: Reverb,
    tone_ph: f32,
    sub_ph: [f32; 2],
    trem: f32,
    master: Ramp,
    x: Smooth,
    y: Smooth,
    held: [f32; 4],
    change_wait: i64,
    bell_wait: i64,
    ctrl: u32,
}

impl Tones {
    pub fn new(sr: f32, seed: u64, f0: f32, x: f32, y: f32) -> Self {
        let mut rng = Rng::new(seed);
        let f0 = f0.clamp(40.0, 2000.0);
        let m0 = hz_midi(f0);
        let ctrl_rate = sr / CTRL as f32;
        let held = [m0 - 24.0, m0 - 17.0, m0 - 12.0, m0 - 8.0];
        let mut t = Tones {
            sr,
            f0,
            m0,
            pads: Pads::new(sr, 8, rng.next_u64()),
            bells: Bells::new(sr, 8),
            reverb: Reverb::new(sr, 4.5, 1.2, 4000.0),
            tone_ph: 0.0,
            sub_ph: [0.0; 2],
            trem: rng.f32(),
            master: Ramp::new(0.0),
            x: Smooth::new(x, 1.5, ctrl_rate),
            y: Smooth::new(y, 1.5, ctrl_rate),
            held,
            change_wait: (sr * rng.range(12.0, 25.0)) as i64,
            bell_wait: (sr * rng.range(10.0, 20.0)) as i64,
            ctrl: 0,
            rng,
        };
        t.pads.attack = 7.0;
        t.pads.release = 3.0;
        t.pads.soft = 0.85;
        t.pads.set_chord(&held, 1.0);
        t.master.go(1.0, 8.0, sr);
        t
    }

    fn candidates(&self) -> [f32; 7] {
        let b = self.m0 - 24.0;
        [b, b + 7.0, b + 12.0, b + 14.0, b + 16.0, b + 19.0, b + 21.0]
    }

    fn control(&mut self) {
        let x = self.x.next();
        let y = self.y.next();
        self.pads.cutoff.set(400.0 + 1000.0 * x);
        self.pads.level.set(0.035 * (0.6 + 0.8 * x));
        self.trem = wrap(self.trem + CTRL as f32 * 0.1 / self.sr);
        let speed = 0.6 + 1.2 * y;
        self.change_wait -= (CTRL as f32 * speed) as i64;
        if self.change_wait <= 0 {
            self.change_wait = (self.sr * self.rng.range(12.0, 30.0)) as i64;
            let cands = self.candidates();
            let i = self.rng.int(1, 3) as usize;
            let c = self.rng.pick(&cands);
            if !self.held.iter().any(|&h| (h - c).abs() < 0.01) {
                self.held[i] = c;
                let held = self.held;
                self.pads.set_chord(&held, 1.0);
            }
        }
        self.bell_wait -= (CTRL as f32 * speed) as i64;
        if self.bell_wait <= 0 {
            self.bell_wait = (self.sr * self.rng.range(15.0, 40.0)) as i64;
            let k = PENTA[self.rng.int(0, 4) as usize] as f32;
            let note = self.m0 + k - if self.rng.chance(0.5) { 12.0 } else { 0.0 };
            let pan = self.rng.range(-0.5, 0.5);
            self.bells.play(note, 0.05, pan, &BellParams::GLASS, 0);
        }
    }
}

impl Generator for Tones {
    fn render(&mut self, l: &mut [f32], r: &mut [f32]) {
        let sr = self.sr;
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            if self.ctrl == 0 {
                self.control();
            }
            self.ctrl = (self.ctrl + 1) % CTRL;
            let mut mix = Mix::default();
            self.pads.tick(&mut mix, 0.5);
            self.bells.tick(1.0, &mut mix, 0.6, 0.0);
            self.tone_ph = wrap(self.tone_ph + self.f0 / sr);
            self.sub_ph[0] = wrap(self.sub_ph[0] + self.f0 / 8.0 / sr);
            self.sub_ph[1] = wrap(self.sub_ph[1] + self.f0 / 4.0 / sr);
            let trem = 1.0 + 0.12 * sine(self.trem) * (0.3 + self.y.value);
            let tone = sine(self.tone_ph) * 0.07 * trem + sine(self.sub_ph[0]) * 0.045 + sine(self.sub_ph[1]) * 0.03;
            mix.add(tone, tone, 0.2, 0.0);
            let (rl, rr) = self.reverb.tick(mix.rev.0, mix.rev.1);
            let g = self.master.next();
            *a = (mix.dry.0 + rl * 0.4) * g;
            *b = (mix.dry.1 + rr * 0.4) * g;
        }
    }

    fn tune(&mut self, x: f32, y: f32) {
        self.x.set(x.clamp(0.0, 1.0));
        self.y.set(y.clamp(0.0, 1.0));
    }

    fn status(&self, st: &mut Status) {
        st.mode = "tones";
        st.key = "pentatonic";
        st.root = "";
        st.section = "drift";
        st.beat = "no beat";
        st.phase = "";
        st.bpm = 0.0;
        st.value = self.f0;
        st.value_unit = "Hz";
        st.x = self.x.target;
        st.y = self.y.target;
        st.x_labels = ("Pure", "Rich");
        st.y_labels = ("Still", "Moving");
    }
}
