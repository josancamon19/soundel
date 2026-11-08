//! Focus: generative pentatonic soundscape modeled on measurements of Endel's Focus renders.
//!
//! What ~210 minutes of reference audio showed, and how it maps here:
//! - Everything loops. Inside a section the drum pattern is fixed (kick on every beat, a short,
//!   wide snap on beats 2 and 4 that alternates between two tunings bar to bar, hats on the
//!   offbeats), the bass repeats a one-bar figure, and the melody is a two-bar figure that
//!   recurs in 70–90% of bars, changing at most one note every 8 bars. Predictable parts fade
//!   into the background, which is what lets you focus.
//! - Melody notes blend into the bed (rising only ~4–8 dB above it) rather than popping out.
//! - C major pentatonic for the whole session; the sub-bass root rotates through G, D, C, A
//!   (E rarely), each held for 2–10 minutes.
//! - 120 BPM (2 × a 60 bpm resting heart rate). Evening, night, early morning and sessions past
//!   ~45 minutes go half-time.
//! - Dark, warm and wide; Easy ↔ Intense moves the tilt between ≈ −17 and −6 dB/oct.

use crate::dsp::fx::PingPong;
use crate::dsp::reverb::Reverb;
use crate::dsp::*;
use crate::engine::{Generator, Status};
use crate::music::{self, PENTA};
use crate::rng::Rng;
use crate::voices::*;

const ROOTS: [i32; 5] = [7, 2, 0, 9, 4];
const ROOT_WEIGHTS: [f32; 5] = [0.34, 0.26, 0.22, 0.14, 0.04];
const SETTLE_SECS: f64 = 180.0;
const SUSTAIN_SECS: f64 = 45.0 * 60.0;
const CTRL: u32 = 64;
const MEL_STEPS: usize = 32;
const COUNTER_STEPS: usize = 64;

const KICK_GAIN: f32 = 0.5;
const SUB_GAIN: f32 = 0.034;
const PAD_GAIN: f32 = 0.13;
const MEL_GAIN: f32 = 0.085;
const ARP_GAIN: f32 = 0.08;
const COUNTER_GAIN: f32 = 0.09;
const CLAP_GAIN: f32 = 0.17;
const HAT_GAIN: f32 = 0.06;
const BASS_GAIN: f32 = 0.14;

const fn bits(steps: &[usize]) -> u16 {
    let mut m = 0u16;
    let mut i = 0;
    while i < steps.len() {
        m |= 1 << steps[i];
        i += 1;
    }
    m
}
const BEATS: u16 = bits(&[0, 4, 8, 12]);
const ONE_THREE: u16 = bits(&[0, 8]);
const TWO_FOUR: u16 = bits(&[4, 12]);
const OFFBEATS: u16 = bits(&[2, 6, 10, 14]);

/// One-bar bass figures as semitones above the bass root (R = rest).
const R: i8 = i8::MIN;
const BASS_FIGURES: [[i8; 16]; 5] = [
    [R, R, R, 0, R, R, R, 0, R, R, R, 0, R, R, R, 0],
    [R, R, 0, R, R, R, 0, R, R, R, 0, R, R, R, 7, R],
    [R, R, R, R, R, R, 0, 0, R, R, R, R, R, R, 0, 12],
    [0, R, R, 0, R, R, 7, R, R, R, 0, R, R, R, 12, R],
    [R, R, R, 0, R, R, R, R, R, R, R, 0, R, R, 12, R],
];

/// Energy by hour of day. Picks four-on-the-floor vs half-time and how bright things get.
/// Reference renders: 9h, 13h, 17h four-on-the-floor; 21h, 1h, 6h half-time and darker.
pub fn circadian_energy(hour: f32) -> f32 {
    const E: [f32; 24] = [
        0.2, 0.15, 0.1, 0.1, 0.15, 0.25, 0.4, 0.55, 0.7, 0.85, 0.95, 1.0, 0.95, 0.9, 0.85, 0.85, 0.8, 0.7, 0.6, 0.5,
        0.42, 0.35, 0.3, 0.25,
    ];
    let h = hour.rem_euclid(24.0);
    let i = h.floor() as usize % 24;
    let f = h - h.floor();
    E[i] * (1.0 - f) + E[(i + 1) % 24] * f
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pulse,
    Arps,
    Float,
    Groove,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Pulse => "pulse",
            Kind::Arps => "arpeggios",
            Kind::Float => "float",
            Kind::Groove => "groove",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KickMode {
    Four,
    Half,
    Loose,
    Off,
}

impl KickMode {
    pub fn name(self) -> &'static str {
        match self {
            KickMode::Four => "four on the floor",
            KickMode::Half => "half-time",
            KickMode::Loose => "loose beat",
            KickMode::Off => "no beat",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Settle,
    Flow,
    Sustain,
}

/// A note in a loop: index into the section's chord-tone table, velocity, pan.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Note {
    deg: i8,
    vel: f32,
    pan: f32,
}

/// Fixed drum pattern for a section; bit `i` = 16th step `i` of the bar.
#[derive(Clone, Copy, Debug, Default)]
struct Drums {
    kick: u16,
    clap: u16,
    hat: u16,
    ghost: u16,
}

#[derive(Clone, Copy, Debug)]
struct Section {
    kind: Kind,
    root: i32,
    bars: u32,
    kick: KickMode,
    drums: Drums,
    hat_level: f32,
    bass_on: bool,
    bass: [i8; 16],
    mel: [Option<Note>; MEL_STEPS],
    mel_decay: f32,
    timbre: BellParams,
    arps: bool,
    counter_on: bool,
    counter: [Option<Note>; COUNTER_STEPS],
    pad_cut: f32,
    pad_level: f32,
    break_end: bool,
    voicing: [i32; 6],
    nv: usize,
    mel_table: [i32; 16],
    n_mel: usize,
    counter_table: [i32; 16],
    n_counter: usize,
}

impl Section {
    fn empty() -> Self {
        Section {
            kind: Kind::Pulse,
            root: -1,
            bars: 0,
            kick: KickMode::Off,
            drums: Drums::default(),
            hat_level: 0.0,
            bass_on: false,
            bass: [R; 16],
            mel: [None; MEL_STEPS],
            mel_decay: 0.7,
            timbre: BellParams::MALLET,
            arps: false,
            counter_on: false,
            counter: [None; COUNTER_STEPS],
            pad_cut: 700.0,
            pad_level: 1.0,
            break_end: false,
            voicing: [0; 6],
            nv: 0,
            mel_table: [0; 16],
            n_mel: 0,
            counter_table: [0; 16],
            n_counter: 0,
        }
    }
}

/// Chord tones of `voicing` (pitch classes) laid out across [lo, hi].
fn chord_table(voicing: &[i32], lo: i32, hi: i32, out: &mut [i32; 16]) -> usize {
    let mut n = 0;
    for m in lo..=hi {
        if n < out.len() && voicing.iter().any(|v| v.rem_euclid(12) == m.rem_euclid(12)) {
            out[n] = m;
            n += 1;
        }
    }
    n
}

/// A two-bar melodic figure: `notes` onsets on the 16th grid (mostly eighths), a mostly-stepwise
/// contour through the chord tones that closes near where it started, alternating sides.
fn make_melody(r: &mut Rng, table_len: usize, notes: usize, sixteenths: f32) -> [Option<Note>; MEL_STEPS] {
    let mut out = [None; MEL_STEPS];
    if table_len < 2 || notes == 0 {
        return out;
    }
    let mut w = [0f32; MEL_STEPS];
    for (s, wv) in w.iter_mut().enumerate() {
        *wv = if s % 4 == 0 {
            2.2
        } else if s % 2 == 0 {
            2.0
        } else {
            0.4 + 1.4 * sixteenths
        };
    }
    let mut chosen = [false; MEL_STEPS];
    for _ in 0..notes.min(MEL_STEPS) {
        let i = r.weighted(&w);
        chosen[i] = true;
        w[i] = 0.0;
    }
    let total = chosen.iter().filter(|&&c| c).count();
    let n = table_len as i32;
    let start = r.int(n / 4, (3 * n / 4).max(n / 4));
    let mut idx = start;
    let mut side = if r.chance(0.5) { 1.0 } else { -1.0 };
    let mut k = 0;
    for (s, &on) in chosen.iter().enumerate() {
        if !on {
            continue;
        }
        if k > 0 {
            idx += [-2, -1, 0, 1, 2][r.weighted(&[0.14, 0.32, 0.08, 0.32, 0.14])];
            if k + 1 == total && (idx - start).abs() > 2 {
                idx += (start - idx).signum() * 2;
            }
            if idx < 0 {
                idx = 1;
            }
            if idx >= n {
                idx = n - 2;
            }
            idx = idx.clamp(0, n - 1);
        }
        let accent = if s % 4 == 0 {
            1.0
        } else if s % 2 == 0 {
            0.85
        } else {
            0.65
        };
        side = -side;
        out[s] = Some(Note { deg: idx as i8, vel: accent * r.range(0.85, 1.0), pan: side * r.range(0.3, 0.75) });
        k += 1;
    }
    out
}

/// A slow four-bar counter line: a few long tones on strong or syncopated slots.
fn make_counter(r: &mut Rng, table_len: usize, notes: usize) -> [Option<Note>; COUNTER_STEPS] {
    const SLOTS: [usize; 12] = [0, 6, 8, 14, 16, 22, 24, 32, 38, 40, 48, 56];
    let mut out = [None; COUNTER_STEPS];
    if table_len == 0 {
        return out;
    }
    let mut w = [1.0f32; 12];
    w[0] = 3.0;
    w[7] = 2.0;
    for _ in 0..notes.min(SLOTS.len()) {
        let i = r.weighted(&w);
        w[i] = 0.0;
        let deg = r.int(0, table_len as i32 - 1) as i8;
        out[SLOTS[i]] = Some(Note { deg, vel: r.range(0.7, 1.0), pan: r.range(-0.5, 0.5) });
    }
    out
}

pub struct FocusParams {
    pub sr: f32,
    pub seed: u64,
    pub hour: f32,
    pub x: f32,
    pub y: f32,
    pub bpm: f32,
    /// Beatless, slower and softer variant (Pomodoro breaks, `soundel relax`).
    pub relax: bool,
}

pub struct Focus {
    sr: f32,
    rng: Rng,
    relax: bool,
    start_hour: f32,
    energy: f32,
    bpm: f32,
    step_len: f64,
    step_pos: f64,
    step: u64,
    sec_start: u64,
    bar_in_sec: u32,
    t: u64,
    ctrl: u32,
    x: Smooth,
    y: Smooth,
    sec: Section,
    sec_index: u32,
    breath: f32,
    phase: Phase,
    drums_started: bool,
    drum_gain: Ramp,
    master: Ramp,
    trim: Smooth,
    scale: Vec<i32>,
    drift_wait: i64,
    skip_req: bool,
    kick: Kick,
    sub: Sub,
    pads: Pads,
    mel: Bells,
    counter: Bells,
    plucks: Plucks,
    hats: Hats,
    clap: Clap,
    bass: Bass,
    air: Air,
    reverb: Reverb,
    delay: PingPong,
    rev_wet: f32,
    tilt: [Biquad; 2],
    lp: [Svf; 2],
    dc: [DcBlock; 2],
    tilt_db: f32,
    lp_hz: f32,
}

impl Focus {
    pub fn new(p: FocusParams) -> Self {
        let sr = p.sr;
        let mut rng = Rng::new(p.seed);
        let seed32 = rng.next_u64() as u32;
        let step_len = sr as f64 * 60.0 / p.bpm as f64 / 4.0;
        let ctrl_rate = sr / CTRL as f32;
        let mut f = Focus {
            sr,
            relax: p.relax,
            start_hour: p.hour,
            energy: circadian_energy(p.hour),
            bpm: p.bpm,
            step_len,
            step_pos: step_len,
            step: 0,
            sec_start: 0,
            bar_in_sec: 0,
            t: 0,
            ctrl: 0,
            x: Smooth::new(p.x, 1.2, ctrl_rate),
            y: Smooth::new(p.y, 1.2, ctrl_rate),
            sec: Section::empty(),
            sec_index: 0,
            breath: 1.0,
            phase: Phase::Settle,
            drums_started: false,
            drum_gain: Ramp::new(0.0),
            master: Ramp::new(0.0),
            trim: Smooth::new(1.0, 20.0, ctrl_rate),
            scale: music::scale_notes(0, &PENTA, 48, 72),
            drift_wait: (sr * rng.range(40.0, 90.0)) as i64,
            skip_req: false,
            kick: Kick::new(sr, seed32 | 1),
            sub: Sub::new(sr),
            pads: Pads::new(sr, 8, rng.next_u64()),
            mel: Bells::new(sr, 16),
            counter: Bells::new(sr, 8),
            plucks: Plucks::new(sr, 16),
            hats: Hats::new(sr, seed32.rotate_left(7) | 1),
            clap: Clap::new(sr, seed32.rotate_left(13)),
            bass: Bass::new(sr),
            air: Air::new(sr, seed32.rotate_left(19) | 1, 5200.0),
            reverb: Reverb::new(sr, if p.relax { 3.4 } else { 2.0 }, 1.0, 5200.0),
            delay: PingPong::new(sr, 2.0),
            rev_wet: if p.relax { 0.6 } else { 0.55 },
            tilt: [Biquad::default(); 2],
            lp: [Svf::default(); 2],
            dc: [DcBlock::default(); 2],
            tilt_db: 99.0,
            lp_hz: 1.0,
            rng,
        };
        f.delay.set_time(0.75 * 60.0 / p.bpm);
        f.delay.feedback = 0.3;
        f.pads.attack = if p.relax { 6.0 } else { 4.0 };
        f.pads.release = if p.relax { 2.2 } else { 1.4 };
        f.sub.level.snap(SUB_GAIN);
        f.bass.grit = 0.3;
        f.master.go(1.0, 6.0, sr);
        let s = f.plan_section();
        f.apply_section(s);
        f.control();
        f
    }

    fn plan_section(&mut self) -> Section {
        let x = self.x.target;
        let y = self.y.target;
        let e = self.energy;
        let first = self.sec_index == 0;
        let prev = self.sec;
        let relax = self.relax;
        let sustain = self.phase == Phase::Sustain;
        let bar_secs = 4.0 * 60.0 / self.bpm;
        let r = &mut self.rng;

        let kind = if relax {
            Kind::Float
        } else if first {
            Kind::Pulse
        } else {
            let mut w = [0.38, 0.24, 0.16, 0.22];
            w[1] *= 0.6 + 0.8 * x;
            w[3] *= (0.5 + 1.0 * x) * (0.4 + 0.8 * e);
            w[2] *= 1.4 - 0.8 * x;
            w[prev.kind as usize] *= 0.35;
            if prev.kind == Kind::Float {
                w[2] = 0.0;
            }
            [Kind::Pulse, Kind::Arps, Kind::Float, Kind::Groove][r.weighted(&w)]
        };
        let root = if first {
            r.pick(&[0, 7])
        } else {
            let mut w = ROOT_WEIGHTS;
            for (i, &pc) in ROOTS.iter().enumerate() {
                if pc == prev.root {
                    w[i] = 0.0;
                }
            }
            ROOTS[r.weighted(&w)]
        };
        let minutes = if first { r.range(2.2, 3.6) } else { (4.2 * (0.45 * r.gauss()).exp()).clamp(2.0, 10.0) };
        let minutes = minutes * if sustain { 1.3 } else { 1.0 };
        let bars = ((minutes * 60.0 / bar_secs).round() as u32).max(4);

        let halftime = e < 0.5 || sustain;
        let kick = if relax {
            KickMode::Off
        } else {
            match kind {
                Kind::Float => KickMode::Loose,
                _ if x < 0.25 => KickMode::Loose,
                _ if halftime => KickMode::Half,
                _ => KickMode::Four,
            }
        };
        let beat = kick != KickMode::Off;
        let mut drums = Drums {
            kick: match kick {
                KickMode::Four => BEATS,
                KickMode::Half => ONE_THREE,
                KickMode::Loose => 1 | if r.chance(0.5) { 1 << 10 } else { 1 << 8 },
                KickMode::Off => 0,
            },
            ..Drums::default()
        };
        if beat && kind != Kind::Float {
            drums.clap = if first || r.chance(0.9) { TWO_FOUR } else { 0 };
            drums.hat = OFFBEATS;
            for s in [1usize, 3, 5, 7, 9, 11, 13, 15] {
                if r.chance(0.08 + 0.4 * y) {
                    drums.ghost |= 1 << s;
                }
            }
        } else if beat {
            drums.hat = OFFBEATS;
        }
        let hat_level = match kind {
            Kind::Float => 0.35,
            Kind::Groove => 1.0,
            _ => 0.85,
        } * (0.5 + 0.6 * e)
            * if sustain { 0.75 } else { 1.0 };
        let bass_on = !relax && (kind == Kind::Groove || (kind != Kind::Float && x > 0.6 && r.chance(0.6)));
        let bass = BASS_FIGURES[r.int(0, BASS_FIGURES.len() as i32 - 1) as usize];

        // Busy and sparse sections alternate so attention never fully habituates.
        let breath = if relax {
            0.5
        } else if first {
            0.8
        } else if self.breath > 1.0 {
            r.range(0.55, 0.8)
        } else {
            r.range(1.1, 1.4)
        };
        self.breath = breath;

        let mut voicing = [0i32; 6];
        let nv = music::pad_voicing(r, root, &self.scale, &mut voicing);
        let mut mel_table = [0i32; 16];
        let n_mel = chord_table(&voicing[..nv], 67, 91, &mut mel_table);
        let mut counter_table = [0i32; 16];
        let n_counter = chord_table(&voicing[..nv], 60, 76, &mut counter_table);

        let arps = kind == Kind::Arps;
        let mel_notes = match kind {
            Kind::Arps => 9 + (5.0 * x) as usize,
            Kind::Float => 2 + (2.0 * breath) as usize,
            _ => (3.0 + 4.0 * breath * (0.7 + 0.6 * y)).round() as usize,
        };
        let mel = make_melody(r, n_mel, mel_notes, if arps { 0.9 } else { 0.25 + 0.5 * x });
        let mel_decay = match kind {
            Kind::Arps => 0.32,
            Kind::Float => 1.6,
            _ => 0.75,
        } * (1.25 - 0.4 * x);
        let timbre = if relax {
            BellParams::GLASS
        } else {
            [BellParams::MALLET, BellParams::KEYS, BellParams::PLUCK, BellParams::GLASS]
                [r.weighted(&[0.4, 0.2, 0.25, 0.15])]
        };
        let counter_on = relax || kind == Kind::Float || kind == Kind::Pulse || r.chance(0.5);
        let counter_notes = r.int(2, 4) as usize;
        let counter = make_counter(r, n_counter, counter_notes);

        let pad_cut = r.range(450.0, 900.0);
        let pad_level = match kind {
            Kind::Float => 1.6,
            Kind::Pulse => 1.0,
            Kind::Arps => 0.8,
            Kind::Groove => 0.85,
        } * if relax { 1.1 } else { 1.0 };
        let break_end = beat && !relax && r.chance(0.3);

        Section {
            kind,
            root,
            bars,
            kick,
            drums,
            hat_level,
            bass_on,
            bass,
            mel,
            mel_decay,
            timbre,
            arps,
            counter_on,
            counter,
            pad_cut,
            pad_level,
            break_end,
            voicing,
            nv,
            mel_table,
            n_mel,
            counter_table,
            n_counter,
        }
    }

    fn apply_section(&mut self, s: Section) {
        self.sub.set_note(music::sub_midi(s.root) as f32, 1.2);
        let mut notes = [0f32; 6];
        for (n, v) in notes.iter_mut().zip(s.voicing.iter()).take(s.nv) {
            *n = *v as f32;
        }
        self.pads.set_chord(&notes[..s.nv], 1.0);
        self.sec = s;
        self.sec_index += 1;
        self.bar_in_sec = 0;
        self.sec_start = self.step;
    }

    /// Change one melody note (pitch by a chord tone, or nudge it by a 16th): slow evolution.
    fn mutate_melody(&mut self) {
        let mut slots = [0usize; MEL_STEPS];
        let mut n = 0;
        for (i, m) in self.sec.mel.iter().enumerate() {
            if m.is_some() {
                slots[n] = i;
                n += 1;
            }
        }
        if n == 0 {
            return;
        }
        let i = slots[self.rng.int(0, n as i32 - 1) as usize];
        let Some(mut note) = self.sec.mel[i] else { return };
        if self.rng.chance(0.7) {
            let d = note.deg as i32 + if self.rng.chance(0.5) { 1 } else { -1 };
            note.deg = d.clamp(0, self.sec.n_mel as i32 - 1) as i8;
            self.sec.mel[i] = Some(note);
        } else {
            let j = (i + if self.rng.chance(0.5) { 1 } else { MEL_STEPS - 1 }) % MEL_STEPS;
            if self.sec.mel[j].is_none() {
                self.sec.mel[i] = None;
                self.sec.mel[j] = Some(note);
            }
        }
    }

    fn mutate_counter(&mut self) {
        let n = self.sec.n_counter as i32;
        if n == 0 {
            return;
        }
        let k = self.rng.int(0, COUNTER_STEPS as i32 - 1) as usize;
        for off in 0..COUNTER_STEPS {
            let i = (k + off) % COUNTER_STEPS;
            if let Some(mut note) = self.sec.counter[i] {
                note.deg = (note.deg as i32 + if self.rng.chance(0.5) { 1 } else { -1 }).clamp(0, n - 1) as i8;
                self.sec.counter[i] = Some(note);
                return;
            }
        }
    }

    fn on_bar(&mut self) {
        let secs = self.t as f64 / self.sr as f64;
        self.energy = circadian_energy(self.start_hour + (secs / 3600.0) as f32);
        let phase = if secs < SETTLE_SECS {
            Phase::Settle
        } else if secs < SUSTAIN_SECS {
            Phase::Flow
        } else {
            Phase::Sustain
        };
        if phase != self.phase {
            self.phase = phase;
            if phase == Phase::Sustain {
                self.trim.set(db(-1.5));
            }
        }
        if self.bar_in_sec >= self.sec.bars || self.skip_req {
            self.skip_req = false;
            let s = self.plan_section();
            self.apply_section(s);
        }
        if self.bar_in_sec > 0 && self.bar_in_sec % 8 == 0 && self.rng.chance(0.5) {
            self.mutate_melody();
        }
        if self.bar_in_sec > 0 && self.bar_in_sec % 16 == 0 && self.rng.chance(0.35) {
            self.mutate_counter();
        }
        self.bar_in_sec += 1;
    }

    fn on_step(&mut self, pos: u32) {
        let s = self.sec;
        let x = self.x.value;
        let y = self.y.value;
        let dg = self.drum_gain.value;
        let rel = (self.step - self.sec_start) as usize;
        let in_break = s.break_end && self.bar_in_sec == s.bars && pos >= 8;
        let bit = 1u16 << pos;
        let ms = self.sr / 1000.0;

        if !in_break && dg > 0.0 {
            if s.drums.kick & bit != 0 {
                let decay = if s.kick == KickMode::Four { 0.13 } else { 0.16 };
                let vel = KICK_GAIN * dg * self.rng.range(0.96, 1.0);
                self.kick.trigger(vel, 160.0, 55.0, decay, 0.012, 0);
            }
            if s.drums.clap & bit != 0 {
                // Alternate two tunings bar to bar, like the reference's backbeat.
                let center = if (rel / 16) % 2 == 0 { 1450.0 } else { 1230.0 };
                let vel = CLAP_GAIN * dg * (0.55 + 0.7 * y) * self.rng.range(0.94, 1.0);
                self.clap.trigger(vel, center, 0);
            }
            let hat = HAT_GAIN * s.hat_level * dg * (0.45 + 1.1 * y);
            if s.drums.hat & bit != 0 {
                let w = (self.rng.f32() * 1.5 * ms) as u32;
                self.hats.trigger(hat * self.rng.range(0.92, 1.0), 0.014 + 0.012 * y, 0.15, 7000.0 + 2500.0 * y, w);
            }
            if s.drums.ghost & bit != 0 {
                let w = (self.rng.f32() * 1.5 * ms) as u32;
                self.hats.trigger(hat * 0.38, 0.008, -0.25, 9000.0, w);
            }
        }

        if s.bass_on && !in_break {
            let o = s.bass[pos as usize];
            if o != R {
                let accent = if pos % 4 == 3 { 0.85 } else { 1.0 };
                let note = (music::bass_midi(s.root) + o as i32) as f32;
                self.bass.play(note, BASS_GAIN * accent * dg.max(0.3), 0.16, 0);
            }
        }

        if let Some(n) = s.mel[rel % MEL_STEPS] {
            let midi = s.mel_table[(n.deg as usize).min(s.n_mel.max(1) - 1)] as f32;
            let settle = if self.phase == Phase::Settle { 0.8 } else { 1.0 };
            let pan = n.pan * (1.15 - 0.4 * x);
            if s.arps {
                let vel = ARP_GAIN * n.vel * settle * (0.85 + 0.3 * y);
                self.plucks.play(midi - 12.0, vel, pan, s.mel_decay, 0);
            } else {
                let vel = MEL_GAIN * n.vel * settle * (0.85 + 0.3 * y);
                let p = BellParams { decay: s.mel_decay, ..s.timbre };
                self.mel.play(midi, vel, pan, &p, 0);
            }
        }

        if s.counter_on {
            if let Some(n) = s.counter[rel % COUNTER_STEPS] {
                let midi = s.counter_table[(n.deg as usize).min(s.n_counter.max(1) - 1)] as f32;
                self.counter.play(midi, COUNTER_GAIN * n.vel, n.pan, &BellParams::GLASS, 0);
            }
        }
    }

    /// Move one pad voice to a neighboring scale tone so long sections keep evolving.
    fn drift_pad(&mut self) {
        let n = self.sec.nv;
        if n < 3 {
            return;
        }
        let i = self.rng.int(1, n as i32 - 1) as usize;
        let cur = self.sec.voicing[i];
        let Some(pos) = self.scale.iter().position(|&m| m == cur) else { return };
        let np = if self.rng.chance(0.5) { pos + 1 } else { pos.wrapping_sub(1) };
        if np >= self.scale.len() {
            return;
        }
        let cand = self.scale[np];
        if cand <= self.sec.voicing[0] || cand > 71 {
            return;
        }
        if (0..n).any(|j| j != i && (self.sec.voicing[j] - cand).abs() < 2) {
            return;
        }
        self.sec.voicing[i] = cand;
        self.sec.voicing[..n].sort_unstable();
        let mut notes = [0f32; 6];
        for (k, v) in notes.iter_mut().enumerate().take(n) {
            *v = self.sec.voicing[k] as f32;
        }
        self.pads.set_chord(&notes[..n], 1.0);
    }

    fn control(&mut self) {
        let x = self.x.next();
        let y = self.y.next();
        self.trim.next();
        let e = self.energy;
        let sr = self.sr;
        if !self.drums_started && !self.relax && self.t as f32 > 5.0 * sr {
            self.drums_started = true;
            self.drum_gain.go(1.0, 38.0, sr / CTRL as f32);
        }
        self.drum_gain.next();

        let cut = self.sec.pad_cut * (0.7 + 1.0 * y) * (0.7 + 0.3 * e);
        self.pads.cutoff.set(cut);
        self.pads.level.set(self.sec.pad_level * PAD_GAIN);
        self.pads.width = 1.15 - 0.35 * x;
        self.pads.soft = if self.relax { 0.8 } else { 0.65 - 0.2 * y };
        self.mel.bright = 0.75 + 0.8 * y;
        self.counter.bright = 0.5 + 0.6 * y;
        self.plucks.bright = 0.5 + 0.8 * y;
        self.bass.cutoff = 170.0 + 260.0 * y;
        let float = if self.sec.kind == Kind::Float { 1.3 } else { 1.0 };
        self.air.level.set(0.036 * (0.25 + 1.3 * y) * float);

        let tilt = (-5.0 + 11.0 * y + (e - 0.75) * 5.0).clamp(-12.0, 6.0);
        if (tilt - self.tilt_db).abs() > 0.05 {
            self.tilt_db = tilt;
            for b in self.tilt.iter_mut() {
                b.set_high_shelf(1800.0, tilt, sr);
            }
        }
        let lp = (1800.0 * 12f32.powf(e) * (0.6 + 0.8 * y)).clamp(1500.0, 18000.0);
        if (lp / self.lp_hz - 1.0).abs() > 0.01 {
            self.lp_hz = lp;
            for f in self.lp.iter_mut() {
                f.set(lp, 0.6, sr);
            }
        }

        self.drift_wait -= CTRL as i64;
        if self.drift_wait <= 0 {
            self.drift_wait = (sr * self.rng.range(40.0, 90.0)) as i64;
            self.drift_pad();
        }
    }

    #[inline]
    fn tick(&mut self) -> (f32, f32) {
        self.step_pos += 1.0;
        if self.step_pos >= self.step_len {
            self.step_pos -= self.step_len;
            let pos = (self.step % 16) as u32;
            if pos == 0 {
                self.on_bar();
            }
            self.on_step(pos);
            self.step += 1;
        }
        if self.ctrl == 0 {
            self.control();
        }
        self.ctrl = (self.ctrl + 1) % CTRL;

        let mut mix = Mix::default();
        let k = self.kick.tick();
        mix.add(k, k, 0.015, 0.0);
        let duck = 1.0 - 0.3 * (self.kick.env / KICK_GAIN).min(1.0);
        let (sl, sr) = self.sub.tick();
        mix.add(sl * duck, sr * duck, 0.0, 0.0);
        let (bl, br) = self.bass.tick();
        mix.add(bl * duck, br * duck, 0.03, 0.0);
        self.pads.tick(&mut mix, 0.55);
        self.mel.tick(1.0, &mut mix, 0.4, 0.25);
        self.counter.tick(1.0, &mut mix, 0.55, 0.2);
        self.plucks.tick(&mut mix, 0.3, 0.25);
        self.hats.tick(&mut mix, 0.05);
        self.clap.tick(&mut mix, 0.42);
        self.air.tick(&mut mix);

        let (dl, dr) = self.delay.tick(mix.dly.0, mix.dly.1);
        let (rl, rr) = self.reverb.tick(mix.rev.0 + dl * 0.3, mix.rev.1 + dr * 0.3);
        let l = mix.dry.0 + dl * 0.5 + rl * self.rev_wet;
        let r = mix.dry.1 + dr * 0.5 + rr * self.rev_wet;
        let l = self.dc[0].tick(self.lp[0].lp(self.tilt[0].tick(l)));
        let r = self.dc[1].tick(self.lp[1].lp(self.tilt[1].tick(r)));
        let g = self.master.next() * self.trim.value;
        self.t += 1;
        (l * g, r * g)
    }
}

impl Generator for Focus {
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
        self.skip_req = true;
    }

    fn status(&self, st: &mut Status) {
        st.mode = if self.relax { "relax" } else { "focus" };
        st.key = "C pentatonic";
        st.root = music::name(self.sec.root);
        st.section = self.sec.kind.name();
        st.beat = self.sec.kick.name();
        st.phase = match (self.relax, self.phase) {
            (true, _) => "resting",
            (_, Phase::Settle) => "settling in",
            (_, Phase::Flow) => "in flow",
            (_, Phase::Sustain) => "sustain",
        };
        st.bpm = self.bpm;
        st.x = self.x.target;
        st.y = self.y.target;
        st.x_labels = ("Fluid", "Structured");
        st.y_labels = ("Easy", "Intense");
        st.beats = self.t as f64 / (4.0 * self.step_len);
        let dg = self.drum_gain.value;
        if dg > 0.0 {
            // A section's last bar may drop the drums for its second half.
            let keep = if self.sec.break_end && self.bar_in_sec == self.sec.bars { 0x00ff } else { 0xffff };
            st.kick = self.sec.drums.kick & keep;
            st.snap = self.sec.drums.clap & keep;
            st.groove = dg;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focus(hour: f32, x: f32, relax: bool, seed: u64) -> Focus {
        Focus::new(FocusParams { sr: 16_000.0, seed, hour, x, y: 0.5, bpm: 120.0, relax })
    }

    fn beat_of(f: &Focus) -> &'static str {
        let mut st = Status::default();
        f.status(&mut st);
        st.beat
    }

    fn run(f: &mut Focus, secs: f32) {
        let n = (16_000.0 * secs) as usize;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        f.render(&mut l, &mut r);
    }

    #[test]
    fn circadian_curve_matches_reference_hours() {
        for h in [9.0, 13.0, 17.0] {
            assert!(circadian_energy(h) >= 0.5, "hour {h}");
        }
        for h in [21.0, 1.0, 6.0] {
            assert!(circadian_energy(h) < 0.5, "hour {h}");
        }
    }

    #[test]
    fn daytime_is_four_on_the_floor_and_night_is_half_time() {
        for seed in 0..8 {
            assert_eq!(beat_of(&focus(10.0, 0.5, false, seed)), "four on the floor");
            assert_eq!(beat_of(&focus(22.0, 0.5, false, seed)), "half-time");
            assert_eq!(beat_of(&focus(10.0, 0.1, false, seed)), "loose beat");
            assert_eq!(beat_of(&focus(10.0, 0.5, true, seed)), "no beat");
        }
    }

    #[test]
    fn melody_is_a_fixed_loop_between_mutations() {
        for seed in 0..6 {
            let mut f = focus(10.0, 0.5, false, seed);
            let before = f.sec.mel;
            run(&mut f, 13.0); // 6.5 bars: before the first possible mutation at bar 8
            assert_eq!(f.sec.mel, before);
            let notes = before.iter().filter(|n| n.is_some()).count();
            assert!((2..=14).contains(&notes), "{notes} notes");
        }
    }

    #[test]
    fn melody_figures_use_chord_tones_and_stay_in_range() {
        let mut r = Rng::new(3);
        for _ in 0..200 {
            let m = make_melody(&mut r, 10, 7, 0.5);
            assert_eq!(m.iter().filter(|x| x.is_some()).count(), 7);
            for note in m.iter().flatten() {
                assert!((0..10).contains(&(note.deg as i32)));
                assert!(note.pan.abs() <= 0.75);
            }
        }
    }

    #[test]
    fn renders_clean_audio_and_is_deterministic() {
        let mut a = focus(10.0, 0.5, false, 42);
        let mut b = focus(10.0, 0.5, false, 42);
        let n = 16_000 * 30;
        let (mut al, mut ar) = (vec![0.0; n], vec![0.0; n]);
        let (mut bl, mut br) = (vec![0.0; n], vec![0.0; n]);
        a.render(&mut al, &mut ar);
        b.render(&mut bl, &mut br);
        assert_eq!(al, bl);
        assert_eq!(ar, br);
        let peak = al.iter().chain(ar.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(al.iter().all(|v| v.is_finite()));
        assert!(peak > 0.01 && peak < 1.5, "peak {peak}");
    }

    #[test]
    fn sections_rotate_roots() {
        let mut f = focus(10.0, 0.5, false, 7);
        let mut roots = vec![f.sec.root];
        for _ in 0..12 {
            f.skip();
            run(&mut f, 3.0);
            roots.push(f.sec.root);
        }
        for w in roots.windows(2) {
            assert_ne!(w[0], w[1], "root repeated: {roots:?}");
        }
        for r in &roots {
            assert!(PENTA.contains(r));
        }
    }
}
