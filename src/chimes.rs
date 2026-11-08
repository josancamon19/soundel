//! Cue sounds for the timer and session events. Original sounds in the spirit of Endel's
//! (which are pentatonic C-major blooms, arpeggios and a struck bell), synthesized here.

use clap::ValueEnum;

use crate::dsp::reverb::Reverb;
use crate::voices::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ChimeKind {
    /// Focus block starts: a soft chord bloom.
    Start,
    /// Focus block ends: a rising arpeggio.
    End,
    /// Break is over: three soft bell strikes.
    Break,
    /// Session complete: a falling arpeggio into a bloom.
    Done,
}

#[derive(Clone, Copy, Debug)]
enum Ev {
    Bell { midi: f32, vel: f32, pan: f32, p: BellParams },
    PadOn { midi: f32 },
    PadOff { midi: f32 },
    Thump { vel: f32 },
}

pub struct Chimes {
    sr: f32,
    t: u64,
    events: Vec<(u64, Ev)>,
    bells: Bells,
    pads: Pads,
    kick: Kick,
    reverb: Reverb,
    active_until: u64,
}

impl Chimes {
    pub fn new(sr: f32) -> Self {
        let mut pads = Pads::new(sr, 8, 0xC417E);
        pads.attack = 1.0;
        pads.release = 0.9;
        pads.soft = 0.9;
        pads.width = 1.0;
        pads.cutoff.snap(2600.0);
        pads.level.snap(0.07);
        Chimes {
            sr,
            t: 0,
            events: Vec::with_capacity(64),
            bells: Bells::new(sr, 16),
            pads,
            kick: Kick::new(sr, 77),
            reverb: Reverb::new(sr, 2.6, 1.0, 5000.0),
            active_until: 0,
        }
    }

    fn at(&self, secs: f32) -> u64 {
        self.t + (secs * self.sr) as u64
    }

    fn push(&mut self, secs: f32, ev: Ev) {
        let t = self.at(secs);
        if self.events.len() < self.events.capacity() {
            self.events.push((t, ev));
        }
    }

    pub fn play(&mut self, kind: ChimeKind) {
        let glass = BellParams::GLASS;
        let bell = BellParams::BELL;
        let length = match kind {
            ChimeKind::Start => {
                for (i, &m) in [48.0, 55.0, 60.0, 64.0, 69.0, 72.0, 76.0].iter().enumerate() {
                    self.push(0.06 * i as f32, Ev::PadOn { midi: m });
                    self.push(1.9 + 0.05 * i as f32, Ev::PadOff { midi: m });
                }
                self.push(1.15, Ev::Bell { midi: 72.0, vel: 0.12, pan: -0.3, p: glass });
                self.push(1.3, Ev::Bell { midi: 79.0, vel: 0.09, pan: 0.35, p: glass });
                6.5
            }
            ChimeKind::End => {
                self.push(0.0, Ev::Thump { vel: 0.35 });
                for (i, &m) in [60.0, 64.0, 67.0, 69.0, 72.0, 76.0].iter().enumerate() {
                    let vel = 0.2 - 0.015 * i as f32;
                    self.push(0.075 * i as f32, Ev::Bell { midi: m, vel, pan: -0.5 + 0.2 * i as f32, p: glass });
                }
                5.5
            }
            ChimeKind::Break => {
                for (k, &s) in [0.0f32, 1.5, 2.4].iter().enumerate() {
                    let v = 0.17 - 0.035 * k as f32;
                    self.push(s, Ev::Bell { midi: 67.0, vel: v, pan: -0.2, p: bell });
                    self.push(s + 0.02, Ev::Bell { midi: 74.0, vel: v * 0.6, pan: 0.25, p: bell });
                }
                6.5
            }
            ChimeKind::Done => {
                for (i, &m) in [81.0, 76.0, 72.0, 69.0, 64.0, 60.0].iter().enumerate() {
                    let vel = 0.17 - 0.01 * i as f32;
                    self.push(0.11 * i as f32, Ev::Bell { midi: m, vel, pan: 0.5 - 0.2 * i as f32, p: glass });
                }
                for &m in &[48.0, 55.0, 64.0, 72.0] {
                    self.push(0.7, Ev::PadOn { midi: m });
                    self.push(3.2, Ev::PadOff { midi: m });
                }
                8.0
            }
        };
        let end = self.at(length);
        self.active_until = self.active_until.max(end);
    }

    pub fn active(&self) -> bool {
        self.t < self.active_until
    }

    fn fire(&mut self, ev: Ev) {
        match ev {
            Ev::Bell { midi, vel, pan, p } => self.bells.play(midi, vel, pan, &p, 0),
            Ev::PadOn { midi } => self.pads.note_on(midi, 1.0),
            Ev::PadOff { midi } => self.pads.note_off(midi),
            Ev::Thump { vel } => self.kick.trigger(vel, 120.0, 55.0, 0.18, 0.0, 0),
        }
    }

    #[inline]
    pub fn tick(&mut self) -> (f32, f32) {
        if !self.active() {
            self.t += 1;
            return (0.0, 0.0);
        }
        let mut i = 0;
        while i < self.events.len() {
            if self.events[i].0 <= self.t {
                let (_, ev) = self.events.swap_remove(i);
                self.fire(ev);
            } else {
                i += 1;
            }
        }
        let mut mix = Mix::default();
        self.bells.tick(1.0, &mut mix, 0.5, 0.0);
        self.pads.tick(&mut mix, 0.45);
        let k = self.kick.tick();
        mix.add(k, k, 0.0, 0.0);
        let (rl, rr) = self.reverb.tick(mix.rev.0, mix.rev.1);
        self.t += 1;
        (mix.dry.0 + rl * 0.45, mix.dry.1 + rr * 0.45)
    }
}
