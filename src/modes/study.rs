//! Study: lo-fi beats. Endel's Study measured ~80 BPM, swung, very dark (≈ −16..−20 dB/oct), minor key.

use crate::dsp::fx::PingPong;
use crate::dsp::reverb::Reverb;
use crate::dsp::*;
use crate::engine::{Generator, Status};
use crate::music::{self, MINOR_PENTA};
use crate::rng::Rng;
use crate::voices::*;

const CTRL: u32 = 64;
const M9: &[i32] = &[0, 3, 7, 10, 14];
const M7: &[i32] = &[0, 3, 7, 10];
const M11: &[i32] = &[0, 3, 7, 10, 14, 17];
const MAJ7: &[i32] = &[0, 4, 7, 11];
const MAJ9: &[i32] = &[0, 4, 7, 11, 14];
const DOM9: &[i32] = &[0, 4, 7, 10, 14];

/// Four-bar progressions as (semitones above the minor tonic, chord shape).
const PROGS: [[(i32, &[i32]); 4]; 5] = [
    [(0, M9), (5, M9), (8, MAJ7), (7, M7)],
    [(0, M7), (8, MAJ9), (3, MAJ7), (10, DOM9)],
    [(0, M9), (10, DOM9), (8, MAJ7), (5, M7)],
    [(5, M9), (10, DOM9), (3, MAJ9), (0, M11)],
    [(0, M11), (3, MAJ7), (8, MAJ9), (7, M7)],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Intro,
    Full,
    Break,
}

pub struct Study {
    sr: f32,
    rng: Rng,
    bpm: f32,
    step_len: f64,
    step_pos: f64,
    step: u64,
    t: u64,
    ctrl: u32,
    x: Smooth,
    y: Smooth,
    key: i32,
    prog: usize,
    chord: usize,
    bars_on_prog: u32,
    part: Part,
    part_bars: u32,
    bar_in_part: u32,
    chord_notes: [f32; 6],
    n_chord: usize,
    bass_root: i32,
    lick_notes: Vec<i32>,
    lick_idx: usize,
    lick_left: u32,
    lick_wait: i64,
    lick_pending: bool,
    keys: Bells,
    licks: Bells,
    bass: Bass,
    kick: Kick,
    snare: Snap,
    hats: Hats,
    crackle: Crackle,
    reverb: Reverb,
    delay: PingPong,
    lp: [Svf; 2],
    lp_hz: f32,
    master: Ramp,
    wow: f32,
    flutter: f32,
}

impl Study {
    pub fn new(sr: f32, seed: u64, hour: f32, x: f32, y: f32, bpm: Option<f32>) -> Self {
        let mut rng = Rng::new(seed);
        let energy = super::focus::circadian_energy(hour);
        let bpm = bpm.unwrap_or(76.0 + 8.0 * energy).clamp(60.0, 100.0);
        let key = rng.pick(&[3, 3, 2, 5, 1, 4]); // Eb minor most often, like the reference
        let step_len = sr as f64 * 60.0 / bpm as f64 / 4.0;
        let ctrl_rate = sr / CTRL as f32;
        let s32 = rng.next_u64() as u32 | 1;
        let mut s = Study {
            sr,
            bpm,
            step_len,
            step_pos: step_len,
            step: 0,
            t: 0,
            ctrl: 0,
            x: Smooth::new(x, 1.0, ctrl_rate),
            y: Smooth::new(y, 1.0, ctrl_rate),
            key,
            prog: rng.int(0, PROGS.len() as i32 - 1) as usize,
            chord: 3,
            bars_on_prog: 0,
            part: Part::Intro,
            part_bars: 4,
            bar_in_part: 0,
            chord_notes: [0.0; 6],
            n_chord: 0,
            bass_root: 36,
            lick_notes: music::scale_notes(key, &MINOR_PENTA, 67, 86),
            lick_idx: 4,
            lick_left: 0,
            lick_wait: (sr * rng.range(10.0, 16.0)) as i64,
            lick_pending: false,
            keys: Bells::new(sr, 24),
            licks: Bells::new(sr, 8),
            bass: Bass::new(sr),
            kick: Kick::new(sr, s32),
            snare: Snap::new(sr, s32.rotate_left(9) | 1),
            hats: Hats::new(sr, s32.rotate_left(17) | 1),
            crackle: Crackle::new(sr, rng.next_u64()),
            reverb: Reverb::new(sr, 1.8, 0.9, 4200.0),
            delay: PingPong::new(sr, 2.0),
            lp: [Svf::default(); 2],
            lp_hz: 1.0,
            master: Ramp::new(0.0),
            wow: 0.0,
            flutter: 0.0,
            rng,
        };
        s.bass.grit = 0.15;
        s.bass.cutoff = 220.0;
        s.delay.set_time(0.75 * 60.0 / bpm);
        s.delay.feedback = 0.28;
        s.delay.set_tone(2200.0);
        s.master.go(1.0, 4.0, sr);
        s
    }

    fn swing_wait(&mut self, pos: u32) -> u32 {
        let swing = 0.63 - 0.1 * self.x.value;
        let mut w = if pos % 2 == 1 { (swing - 0.5) * 2.0 * self.step_len as f32 } else { 0.0 };
        w += self.rng.f32() * 0.008 * self.sr;
        w as u32
    }

    fn set_chord(&mut self) {
        let (off, shape) = PROGS[self.prog][self.chord];
        let pc = (self.key + off).rem_euclid(12);
        let root_midi = 48 + pc;
        self.bass_root = root_midi - 12;
        let mut notes = [0f32; 6];
        let mut n = 0;
        for (i, &iv) in shape.iter().enumerate() {
            if i == 0 && self.rng.chance(0.6) {
                continue; // rootless voicing; the bass has the root
            }
            let mut m = root_midi + iv;
            while m < 55 {
                m += 12;
            }
            while m > 77 {
                m -= 12;
            }
            if n < notes.len() && !notes[..n].iter().any(|&v| v as i32 == m) {
                notes[n] = m as f32;
                n += 1;
            }
        }
        notes[..n].sort_by(|a, b| a.partial_cmp(b).unwrap());
        self.chord_notes = notes;
        self.n_chord = n;
    }

    fn on_bar(&mut self) {
        self.bar_in_part += 1;
        if self.bar_in_part > self.part_bars {
            self.bar_in_part = 1;
            let (part, bars) = match self.part {
                Part::Intro | Part::Break => (Part::Full, self.rng.pick(&[16, 24, 32])),
                Part::Full => {
                    if self.rng.chance(0.6) {
                        (Part::Break, self.rng.pick(&[4, 8]))
                    } else {
                        (Part::Full, 16)
                    }
                }
            };
            self.part = part;
            self.part_bars = bars;
        }
        self.chord = (self.chord + 1) % 4;
        self.bars_on_prog += 1;
        if self.chord == 0 && self.bars_on_prog >= 16 && self.rng.chance(0.5) {
            self.bars_on_prog = 0;
            let mut p = self.prog;
            while p == self.prog {
                p = self.rng.int(0, PROGS.len() as i32 - 1) as usize;
            }
            self.prog = p;
        }
        self.set_chord();
    }

    fn on_step(&mut self, pos: u32) {
        if pos == 0 {
            self.on_bar();
        }
        let sw = self.swing_wait(pos);
        let y = self.y.value;
        let drums = self.part == Part::Full;

        let comp = pos == 0 || (pos == 10 && self.rng.chance(0.45)) || (pos == 7 && self.rng.chance(0.2));
        if comp {
            let vel = if pos == 0 { 0.11 } else { 0.075 } * self.rng.range(0.85, 1.0);
            let strum = self.rng.range(0.006, 0.016) * self.sr;
            for i in 0..self.n_chord {
                let pan = -0.75 + 1.5 * i as f32 / (self.n_chord.max(2) - 1) as f32;
                let note = self.chord_notes[i];
                self.keys.play(note, vel, pan, &BellParams::KEYS, sw + (i as f32 * strum) as u32);
            }
        }

        if pos == 0 {
            self.bass.play(self.bass_root as f32, 0.2, 0.45, sw);
        } else if (pos == 10 && self.rng.chance(0.6)) || (pos == 7 && self.rng.chance(0.25)) {
            let n = if self.rng.chance(0.7) { self.bass_root } else { self.bass_root + 7 };
            self.bass.play(n as f32, 0.15, 0.35, sw);
        }

        if drums {
            if pos == 0 || pos == 10 || (pos == 7 && self.rng.chance(0.3)) || (pos == 3 && self.rng.chance(0.08)) {
                self.kick.trigger(0.42, 140.0, 48.0, 0.16, 0.08, sw);
            }
            if pos == 4 || pos == 12 {
                self.snare.trigger(0.13 * (0.7 + 0.5 * y), &SnapParams::SNARE, 0.05, sw);
            } else if pos == 15 && self.rng.chance(0.12) {
                self.snare.trigger(0.035, &SnapParams::SNARE, 0.05, sw);
            }
            if pos % 2 == 0 {
                let v = if pos % 4 == 0 { 0.045 } else { 0.032 } * (0.6 + 0.7 * y) * self.rng.range(0.8, 1.0);
                self.hats.trigger(v, 0.022, 0.2, 8000.0, sw);
            } else if self.rng.chance(0.08 + 0.2 * y) {
                self.hats.trigger(0.014, 0.015, 0.25, 8500.0, sw);
            }
        } else if self.part == Part::Break && pos % 4 == 2 && self.rng.chance(0.5) {
            self.hats.trigger(0.02, 0.02, 0.2, 8000.0, sw);
        }

        if self.lick_pending && pos % 2 == 0 {
            if self.rng.chance(0.65) {
                self.play_lick_note(sw);
            }
        }
    }

    fn play_lick_note(&mut self, wait: u32) {
        let n = self.lick_notes.len() as i32;
        let step = self.rng.pick(&[-2, -1, -1, 1, 1, 2]);
        self.lick_idx = (self.lick_idx as i32 + step).clamp(0, n - 1) as usize;
        let note = self.lick_notes[self.lick_idx] as f32;
        let pan = self.rng.range(-0.4, 0.4);
        self.licks.play(note, 0.06 * self.rng.range(0.7, 1.0), pan, &BellParams::GLASS, wait);
        self.lick_left = self.lick_left.saturating_sub(1);
        if self.lick_left == 0 {
            self.lick_pending = false;
            self.lick_wait = (self.sr * self.rng.range(6.0, 14.0)) as i64;
        }
    }

    fn control(&mut self) {
        self.x.next();
        let y = self.y.next();
        let lp = 1500.0 * (0.75 + 0.9 * y);
        if (lp / self.lp_hz - 1.0).abs() > 0.01 {
            self.lp_hz = lp;
            for f in self.lp.iter_mut() {
                f.set(lp, 0.7, self.sr);
            }
        }
        self.keys.bright = 0.6 + 0.6 * y;
        self.lick_wait -= CTRL as i64;
        if self.lick_wait <= 0 && !self.lick_pending && self.part != Part::Intro {
            self.lick_left = self.rng.int(3, 6) as u32;
            self.lick_pending = true;
            self.lick_wait = i64::MAX / 2;
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

        // Tape wow and flutter on the keys.
        self.wow = wrap(self.wow + 0.45 / self.sr);
        self.flutter = wrap(self.flutter + 5.3 / self.sr);
        let pm = 1.0 + 0.0035 * sine(self.wow) + 0.0008 * sine(self.flutter);

        let mut mix = Mix::default();
        let k = self.kick.tick();
        mix.add(k, k, 0.0, 0.0);
        let duck = 1.0 - 0.25 * (self.kick.env / 0.42).min(1.0);
        let (bl, br) = self.bass.tick();
        mix.add(bl * duck, br * duck, 0.0, 0.0);
        self.keys.tick(pm, &mut mix, 0.45, 0.12);
        self.licks.tick(pm, &mut mix, 0.45, 0.35);
        self.snare.tick(&mut mix, 0.15, 0.0);
        self.hats.tick(&mut mix, 0.05);
        self.crackle.tick(&mut mix);

        let (dl, dr) = self.delay.tick(mix.dly.0, mix.dly.1);
        let (rl, rr) = self.reverb.tick(mix.rev.0 + dl * 0.3, mix.rev.1 + dr * 0.3);
        let l = mix.dry.0 + dl * 0.5 + rl * 0.5;
        let r = mix.dry.1 + dr * 0.5 + rr * 0.5;
        let g = self.master.next() * 0.62;
        let l = (self.lp[0].lp(l) * 1.2).tanh() / 1.2 * g;
        let r = (self.lp[1].lp(r) * 1.2).tanh() / 1.2 * g;
        self.t += 1;
        (l, r)
    }
}

impl Generator for Study {
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
        self.prog = (self.prog + 1) % PROGS.len();
        self.bars_on_prog = 0;
    }

    fn status(&self, st: &mut Status) {
        st.mode = "study";
        st.key = MINOR_KEYS[self.key.rem_euclid(12) as usize];
        st.root = music::name(self.bass_root);
        st.section = match self.part {
            Part::Intro => "intro",
            Part::Full => "beat",
            Part::Break => "break",
        };
        st.beat = if self.part == Part::Full { "swung boom-bap" } else { "no drums" };
        st.phase = "";
        st.bpm = self.bpm;
        st.beats = self.t as f64 / (4.0 * self.step_len);
        if self.part == Part::Full {
            st.kick = 1 | 1 << 10;
            st.snap = 1 << 4 | 1 << 12;
            st.groove = 1.0;
        }
        st.x = self.x.target;
        st.y = self.y.target;
        st.x_labels = ("Loose", "Tight");
        st.y_labels = ("Soft", "Crisp");
    }
}

const MINOR_KEYS: [&str; 12] = [
    "C minor", "C# minor", "D minor", "Eb minor", "E minor", "F minor", "F# minor", "G minor", "G# minor", "A minor",
    "Bb minor", "B minor",
];
