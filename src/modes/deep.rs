//! Deep: minimal techno for deep work. The reference (Endel's techno focus soundscape) measured
//! ~120 BPM four-on-the-floor in C with a ducked rolling bass, 16th hats and a dotted percussion line.

use crate::dsp::fx::PingPong;
use crate::dsp::reverb::Reverb;
use crate::dsp::*;
use crate::engine::{Generator, Status};
use crate::rng::Rng;
use crate::voices::*;

const CTRL: u32 = 64;
const STAB: [f32; 5] = [48.0, 51.0, 55.0, 58.0, 62.0]; // Cm9
const RIM_STEPS: [u32; 5] = [0, 3, 6, 10, 13];

pub struct Deep {
    sr: f32,
    rng: Rng,
    bpm: f32,
    step_len: f64,
    step_pos: f64,
    step: u64,
    bar: u64,
    t: u64,
    ctrl: u32,
    x: Smooth,
    y: Smooth,
    block_left: u32,
    rim_on: bool,
    stab_on: bool,
    shaker_on: bool,
    roll16: bool,
    breakdown_left: u32,
    next_breakdown: u64,
    filt_ph: f32,
    bass_note: f32,
    kick: Kick,
    bass: Bass,
    hats: Hats,
    shaker: Hats,
    rim: Snap,
    stab: Plucks,
    pads: Pads,
    riser: Riser,
    reverb: Reverb,
    delay: PingPong,
    master: Ramp,
    tilt: [Biquad; 2],
    tilt_db: f32,
}

impl Deep {
    pub fn new(sr: f32, seed: u64, hour: f32, x: f32, y: f32, bpm: Option<f32>) -> Self {
        let mut rng = Rng::new(seed);
        let e = super::focus::circadian_energy(hour);
        let bpm = bpm.unwrap_or(118.0 + 6.0 * e).clamp(100.0, 140.0);
        let step_len = sr as f64 * 60.0 / bpm as f64 / 4.0;
        let ctrl_rate = sr / CTRL as f32;
        let s32 = rng.next_u64() as u32 | 1;
        let mut d = Deep {
            sr,
            bpm,
            step_len,
            step_pos: step_len,
            step: 0,
            bar: 0,
            t: 0,
            ctrl: 0,
            x: Smooth::new(x, 1.0, ctrl_rate),
            y: Smooth::new(y, 1.0, ctrl_rate),
            block_left: 8,
            rim_on: false,
            stab_on: false,
            shaker_on: true,
            roll16: false,
            breakdown_left: 0,
            next_breakdown: rng.int(64, 96) as u64,
            filt_ph: rng.f32(),
            bass_note: 36.0,
            kick: Kick::new(sr, s32),
            bass: Bass::new(sr),
            hats: Hats::new(sr, s32.rotate_left(5) | 1),
            shaker: Hats::new(sr, s32.rotate_left(11) | 1),
            rim: Snap::new(sr, s32.rotate_left(17) | 1),
            stab: Plucks::new(sr, 10),
            pads: Pads::new(sr, 4, rng.next_u64()),
            riser: Riser::new(sr, s32.rotate_left(23) | 1),
            reverb: Reverb::new(sr, 2.4, 1.1, 4500.0),
            delay: PingPong::new(sr, 2.0),
            master: Ramp::new(0.0),
            tilt: [Biquad::default(); 2],
            tilt_db: 99.0,
            rng,
        };
        d.bass.grit = 0.7;
        d.bass.detune = 0.015;
        d.delay.set_time(0.75 * 60.0 / bpm);
        d.delay.feedback = 0.5;
        d.delay.set_tone(2500.0);
        d.pads.attack = 8.0;
        d.pads.release = 3.0;
        d.pads.soft = 0.7;
        d.pads.level.snap(0.02);
        d.pads.set_chord(&[48.0, 55.0, 63.0], 1.0);
        d.master.go(1.0, 5.0, sr);
        d
    }

    fn on_bar(&mut self) {
        self.bar += 1;
        if self.breakdown_left > 0 {
            self.breakdown_left -= 1;
            if self.breakdown_left == 1 {
                let dur = 4.0 * 60.0 / self.bpm;
                self.riser.start(dur, 0.05);
            }
        } else if self.bar >= self.next_breakdown {
            self.breakdown_left = 8;
            self.next_breakdown = self.bar + self.rng.int(64, 112) as u64;
        }
        if self.block_left == 0 {
            self.block_left = 16;
            let x = self.x.value;
            if self.rng.chance(0.4) {
                self.rim_on = !self.rim_on;
            }
            if self.rng.chance(0.4) {
                self.stab_on = !self.stab_on;
            }
            if self.rng.chance(0.3) {
                self.shaker_on = !self.shaker_on;
            }
            self.roll16 = self.rng.chance(0.2 + 0.5 * x);
        }
        self.block_left -= 1;
        self.bass_note = if self.rng.chance(0.85) { 36.0 } else { self.rng.pick(&[31.0, 34.0, 39.0]) };
    }

    fn on_step(&mut self, pos: u32) {
        if pos == 0 {
            self.on_bar();
        }
        let y = self.y.value;
        let x = self.x.value;
        let bd = self.breakdown_left > 0;
        let ms = self.sr / 1000.0;
        if !bd && pos % 4 == 0 {
            self.kick.trigger(0.55, 165.0, 49.0, 0.2, 0.3, 0);
        }
        if !(bd && self.breakdown_left > 4) && (pos % 4 == 2 || (self.roll16 && pos % 4 == 3)) {
            let vel = if pos % 4 == 2 { 0.2 } else { 0.13 } * self.rng.range(0.85, 1.0);
            self.bass.play(self.bass_note, vel, 0.1, 0);
        }
        if pos % 4 == 2 {
            let v = 0.05 * (0.6 + 0.8 * y) * self.rng.range(0.85, 1.0);
            self.hats.trigger(v, 0.06 + 0.03 * y, self.rng.range(-0.5, 0.5), 8500.0, 0);
        } else if self.rng.chance(0.35 + 0.45 * y) {
            let v = 0.018 * (0.6 + 0.8 * y) * if pos % 4 == 0 { 1.0 } else { 0.7 };
            let w = (self.rng.f32() * 2.0 * ms) as u32;
            self.hats.trigger(v, 0.012, self.rng.range(-0.6, 0.6), 10000.0, w);
        }
        if self.shaker_on {
            let v = if pos % 2 == 0 { 0.016 } else { 0.011 } * (0.5 + 0.8 * y);
            let w = (self.rng.f32() * 3.0 * ms) as u32;
            self.shaker.trigger(v, 0.03, self.rng.range(0.1, 0.7), 5500.0, w);
        }
        if self.rim_on && !bd && RIM_STEPS.contains(&pos) {
            self.rim.trigger(0.07 * (0.6 + 0.6 * x), &SnapParams::RIM, -0.3, 0);
        }
        if self.stab_on && self.bar % 2 == 1 && pos == 6 {
            for (i, &n) in STAB.iter().enumerate() {
                let pan = -0.3 + 0.15 * i as f32;
                self.stab.play(n + 12.0, 0.035, pan, 0.22, (i as f32 * 0.004 * self.sr) as u32);
            }
        }
    }

    fn control(&mut self) {
        let x = self.x.next();
        let y = self.y.next();
        self.filt_ph = wrap(self.filt_ph + CTRL as f32 / (self.sr * 64.0 * 4.0 * 60.0 / self.bpm));
        let lfo = 0.5 - 0.5 * (TAU * self.filt_ph).cos();
        self.bass.cutoff = 160.0 + (500.0 + 600.0 * y) * lfo * (0.6 + 0.6 * x);
        self.stab.bright = 0.6 + 0.9 * lfo;
        self.pads.cutoff.set(400.0 + 700.0 * lfo);
        let tilt = -8.0 + 9.0 * y;
        if (tilt - self.tilt_db).abs() > 0.05 {
            self.tilt_db = tilt;
            for b in self.tilt.iter_mut() {
                b.set_high_shelf(2000.0, tilt, self.sr);
            }
        }
    }

    #[inline]
    fn tick(&mut self) -> (f32, f32) {
        self.step_pos += 1.0;
        if self.step_pos >= self.step_len {
            self.step_pos -= self.step_len;
            let pos = (self.step % 16) as u32;
            self.on_step(pos);
            self.step += 1;
        }
        if self.ctrl == 0 {
            self.control();
        }
        self.ctrl = (self.ctrl + 1) % CTRL;

        let mut mix = Mix::default();
        let k = self.kick.tick();
        mix.add(k, k, 0.0, 0.0);
        let duck = 1.0 - 0.75 * (self.kick.env / 0.55).min(1.0);
        let (bl, br) = self.bass.tick();
        mix.add(bl * duck, br * duck, 0.02, 0.0);
        let mut pads = Mix::default();
        self.pads.tick(&mut pads, 0.5);
        mix.add(pads.dry.0 * duck, pads.dry.1 * duck, 0.0, 0.0);
        mix.rev.0 += pads.rev.0;
        mix.rev.1 += pads.rev.1;
        self.hats.tick(&mut mix, 0.04);
        self.shaker.tick(&mut mix, 0.08);
        self.rim.tick(&mut mix, 0.2, 0.45);
        self.stab.tick(&mut mix, 0.35, 0.6);
        self.riser.tick(&mut mix);

        let (dl, dr) = self.delay.tick(mix.dly.0, mix.dly.1);
        let (rl, rr) = self.reverb.tick(mix.rev.0 + dl * 0.4, mix.rev.1 + dr * 0.4);
        let l = mix.dry.0 + dl * 0.55 + rl * 0.3;
        let r = mix.dry.1 + dr * 0.55 + rr * 0.3;
        let g = self.master.next() * 0.6;
        self.t += 1;
        (self.tilt[0].tick(l) * g, self.tilt[1].tick(r) * g)
    }
}

impl Generator for Deep {
    fn render(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let (x, y) = self.tick();
            *a = x;
            *b = y;
        }
    }

    fn tune(&mut self, x: f32, y: f32) {
        self.x.set(x.clamp(0.0, 1.0));
        self.y.set(y.clamp(0.0, 1.0));
    }

    fn skip(&mut self) {
        self.block_left = 0;
    }

    fn status(&self, st: &mut Status) {
        st.mode = "deep";
        st.key = "C minor";
        st.root = "C";
        st.section = if self.breakdown_left > 0 { "breakdown" } else { "groove" };
        st.beat = if self.breakdown_left > 0 { "no kick" } else { "four on the floor" };
        st.phase = "";
        st.bpm = self.bpm;
        st.beats = self.t as f64 / (4.0 * self.step_len);
        if self.breakdown_left == 0 {
            st.kick = 0x1111;
            st.groove = 1.0;
        }
        st.x = self.x.target;
        st.y = self.y.target;
        st.x_labels = ("Fluid", "Structured");
        st.y_labels = ("Easy", "Intense");
    }
}
