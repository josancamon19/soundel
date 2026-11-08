//! A listening session: the soundscape plus volume, pause, fades, the Pomodoro timer
//! (crossfading to a beatless Relax soundscape for breaks), cue chimes and the master limiter.

use std::f32::consts::FRAC_PI_2;

use crate::chimes::{ChimeKind, Chimes};
use crate::dsp::fx::Limiter;
use crate::dsp::{Ramp, Smooth};
use crate::engine::{Generator, Status};

#[derive(Clone, Copy, Debug)]
pub enum Command {
    /// Move the tunepad by (dx, dy).
    Tune(f32, f32),
    /// Change volume by a delta.
    Volume(f32),
    Pause,
    Skip,
    Quit,
}

#[derive(Clone, Copy, Debug)]
pub struct TimerSpec {
    pub work: f64,
    pub brk: f64,
    pub long_break: f64,
    pub rounds: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerPhase {
    Focus,
    Break,
    LongBreak,
}

impl TimerPhase {
    fn name(self) -> &'static str {
        match self {
            TimerPhase::Focus => "focus",
            TimerPhase::Break => "break",
            TimerPhase::LongBreak => "long break",
        }
    }
}

/// Pomodoro state machine in samples.
#[derive(Clone, Copy, Debug)]
pub struct Pomodoro {
    work: u64,
    brk: u64,
    long_break: u64,
    pub rounds: u32,
    pub round: u32,
    pub phase: TimerPhase,
    phase_start: u64,
}

impl Pomodoro {
    pub fn new(spec: TimerSpec, sr: f32) -> Self {
        let s = |secs: f64| (secs.max(1.0) * sr as f64) as u64;
        Pomodoro {
            work: s(spec.work),
            brk: s(spec.brk),
            long_break: s(spec.long_break),
            rounds: spec.rounds.max(1),
            round: 1,
            phase: TimerPhase::Focus,
            phase_start: 0,
        }
    }

    fn phase_len(&self) -> u64 {
        match self.phase {
            TimerPhase::Focus => self.work,
            TimerPhase::Break => self.brk,
            TimerPhase::LongBreak => self.long_break,
        }
    }

    pub fn remaining(&self, t: u64) -> u64 {
        (self.phase_start + self.phase_len()).saturating_sub(t)
    }

    /// Advance to `t`; returns the new phase when one ends.
    pub fn update(&mut self, t: u64) -> Option<TimerPhase> {
        if t < self.phase_start + self.phase_len() {
            return None;
        }
        self.phase_start += self.phase_len();
        self.phase = match self.phase {
            TimerPhase::Focus if self.round >= self.rounds => TimerPhase::LongBreak,
            TimerPhase::Focus => TimerPhase::Break,
            TimerPhase::Break => {
                self.round += 1;
                TimerPhase::Focus
            }
            TimerPhase::LongBreak => {
                self.round = 1;
                TimerPhase::Focus
            }
        };
        Some(self.phase)
    }
}

pub struct SessionOpts {
    pub sr: f32,
    pub volume: f32,
    pub duration: Option<f64>,
    pub timer: Option<TimerSpec>,
    pub x: f32,
    pub y: f32,
    /// Play this cue and finish when it's done (`soundel chime`).
    pub chime_only: Option<ChimeKind>,
}

const BLOCK: usize = 512;
const CHIME_GAIN: f32 = 0.9;

pub struct Session {
    sr: f32,
    main: Box<dyn Generator>,
    relax: Option<Box<dyn Generator>>,
    blend: Ramp,
    volume: Smooth,
    vol_target: f32,
    pause: Ramp,
    paused: bool,
    fade_in: Ramp,
    fade_out: Ramp,
    end_fade: f32,
    ending: bool,
    chimes: Chimes,
    duck: Smooth,
    timer: Option<Pomodoro>,
    end_at: Option<u64>,
    pending_chime: Option<(u64, ChimeKind)>,
    chime_only: bool,
    t: u64,
    limiter: Limiter,
    x: f32,
    y: f32,
    buf: [Vec<f32>; 4],
    finished: bool,
}

impl Session {
    pub fn new(o: SessionOpts, main: Box<dyn Generator>, relax: Option<Box<dyn Generator>>) -> Self {
        let sr = o.sr;
        let mut fade_in = Ramp::new(0.0);
        fade_in.go(1.0, 2.5, sr);
        let timer = o.timer.map(|t| Pomodoro::new(t, sr));
        let pending_chime = if let Some(kind) = o.chime_only {
            Some(((0.05 * sr) as u64, kind))
        } else if timer.is_some() {
            Some(((0.4 * sr) as u64, ChimeKind::Start))
        } else {
            None
        };
        Session {
            sr,
            main,
            relax,
            blend: Ramp::new(0.0),
            volume: Smooth::new(o.volume, 0.08, sr),
            vol_target: o.volume,
            pause: Ramp::new(1.0),
            paused: false,
            fade_in,
            fade_out: Ramp::new(1.0),
            end_fade: o.duration.map(|d| (d * 0.25).min(7.0) as f32).unwrap_or(7.0),
            ending: false,
            chimes: Chimes::new(sr),
            duck: Smooth::new(1.0, 0.4, sr),
            timer,
            end_at: o.duration.map(|d| (d * sr as f64) as u64),
            pending_chime,
            chime_only: o.chime_only.is_some(),
            t: 0,
            limiter: Limiter::new(sr, -1.0),
            x: o.x,
            y: o.y,
            buf: [vec![0.0; BLOCK], vec![0.0; BLOCK], vec![0.0; BLOCK], vec![0.0; BLOCK]],
            finished: false,
        }
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    pub fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Tune(dx, dy) => {
                self.x = (self.x + dx).clamp(0.0, 1.0);
                self.y = (self.y + dy).clamp(0.0, 1.0);
                self.main.tune(self.x, self.y);
                if let Some(r) = self.relax.as_mut() {
                    r.tune(self.x, self.y);
                }
            }
            Command::Volume(d) => {
                self.vol_target = (self.vol_target + d).clamp(0.0, 1.5);
                self.volume.set(self.vol_target);
            }
            Command::Pause => {
                self.paused = !self.paused;
                if self.paused {
                    self.pause.go(0.0, 0.4, self.sr);
                } else {
                    self.pause.go(1.0, 0.8, self.sr);
                }
            }
            Command::Skip => {
                if self.blend.value > 0.5 {
                    if let Some(r) = self.relax.as_mut() {
                        r.skip();
                    }
                } else {
                    self.main.skip();
                }
            }
            Command::Quit => self.end(false),
        }
    }

    fn end(&mut self, natural: bool) {
        if self.ending {
            if !natural {
                self.fade_out.go(0.0, 0.6, self.sr);
            }
            return;
        }
        self.ending = true;
        if natural {
            self.fade_out.go(0.0, self.end_fade, self.sr);
            self.chimes.play(ChimeKind::Done);
        } else {
            self.fade_out.go(0.0, 1.5, self.sr);
        }
    }

    fn events(&mut self) {
        if let Some((at, kind)) = self.pending_chime {
            if self.t >= at {
                self.chimes.play(kind);
                self.pending_chime = None;
            }
        }
        if let Some(tm) = self.timer.as_mut() {
            if let Some(phase) = tm.update(self.t) {
                match phase {
                    TimerPhase::Focus => {
                        self.chimes.play(ChimeKind::Break);
                        self.blend.go(0.0, 6.0, self.sr);
                    }
                    TimerPhase::Break | TimerPhase::LongBreak => {
                        self.chimes.play(ChimeKind::End);
                        if self.relax.is_some() {
                            self.blend.go(1.0, 6.0, self.sr);
                        }
                    }
                }
            }
        }
        if let Some(end) = self.end_at {
            if !self.ending && self.t + (self.end_fade * self.sr) as u64 >= end {
                self.end(true);
            }
        }
        self.duck.set(if self.chimes.active() && !self.chime_only { 0.6 } else { 1.0 });
    }

    pub fn render(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mut done = 0;
        while done < l.len() {
            let n = (l.len() - done).min(BLOCK);
            self.render_block(&mut l[done..done + n], &mut r[done..done + n]);
            done += n;
        }
    }

    fn render_block(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        self.events();
        let silent = self.paused && self.pause.done() && self.pause.value <= 0.0;
        let blend_moving = !self.blend.done();
        let [ml, mr, rl, rr] = &mut self.buf;
        if !silent && (self.blend.value < 1.0 || blend_moving) {
            self.main.render(&mut ml[..n], &mut mr[..n]);
        } else {
            ml[..n].fill(0.0);
            mr[..n].fill(0.0);
        }
        match self.relax.as_mut() {
            Some(rx) if !silent && (self.blend.value > 0.0 || blend_moving) => rx.render(&mut rl[..n], &mut rr[..n]),
            _ => {
                rl[..n].fill(0.0);
                rr[..n].fill(0.0);
            }
        }
        for i in 0..n {
            let b = self.blend.next() * FRAC_PI_2;
            let (ga, gb) = (b.cos(), b.sin());
            let vol = self.volume.next();
            let g = vol * self.pause.next() * self.fade_in.next() * self.fade_out.next() * self.duck.next();
            let (cl, cr) = self.chimes.tick();
            let xl = (ml[i] * ga + rl[i] * gb) * g + cl * vol * CHIME_GAIN;
            let xr = (mr[i] * ga + rr[i] * gb) * g + cr * vol * CHIME_GAIN;
            let (ol, or) = self.limiter.tick(xl, xr);
            l[i] = ol;
            r[i] = or;
            self.t += 1;
        }
        let chime_done = self.chime_only && self.pending_chime.is_none() && !self.chimes.active();
        let fade_done = self.ending && self.fade_out.done() && !self.chimes.active();
        if chime_done || fade_done {
            self.finished = true;
        }
    }

    pub fn fill_status(&self, st: &mut Status) {
        st.beats = 0.0;
        st.kick = 0;
        st.snap = 0;
        st.groove = 0.0;
        if self.blend.value > 0.5 {
            if let Some(r) = self.relax.as_ref() {
                r.status(st);
            }
        } else {
            self.main.status(st);
        }
        st.x = self.x;
        st.y = self.y;
        st.volume = self.vol_target;
        st.paused = self.paused;
        st.elapsed = self.t as f64 / self.sr as f64;
        st.timer = self.timer.map(|tm| {
            (tm.phase.name(), tm.remaining(self.t) as f64 / self.sr as f64, tm.round, tm.rounds)
        });
        st.ends_in = self.end_at.map(|e| e.saturating_sub(self.t) as f64 / self.sr as f64);
        st.ending = self.ending;
        st.finished = self.finished;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Silence;
    use crate::modes::{self, Mode, ModeParams, NoiseColor};

    const SR: f32 = 8_000.0;

    fn params() -> ModeParams {
        ModeParams { sr: SR, seed: 1, hour: 10.0, x: 0.5, y: 0.5, bpm: None, color: NoiseColor::Deep, freq: 528.0, beat: 40.0 }
    }

    fn run(s: &mut Session, secs: f32) {
        let n = (secs * SR) as usize;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        s.render(&mut l, &mut r);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0));
    }

    #[test]
    fn pomodoro_cycles_through_breaks_and_long_break() {
        let spec = TimerSpec { work: 2.0, brk: 1.0, long_break: 3.0, rounds: 2 };
        let mut p = Pomodoro::new(spec, 10.0);
        let mut seen = vec![];
        for t in 0..200 {
            if let Some(ph) = p.update(t) {
                seen.push((t, ph, p.round));
            }
        }
        assert_eq!(seen[0], (20, TimerPhase::Break, 1));
        assert_eq!(seen[1], (30, TimerPhase::Focus, 2));
        assert_eq!(seen[2], (50, TimerPhase::LongBreak, 2));
        assert_eq!(seen[3], (80, TimerPhase::Focus, 1));
    }

    #[test]
    fn timer_crossfades_to_relax_during_breaks() {
        let p = params();
        let opts = SessionOpts {
            sr: SR,
            volume: 0.8,
            duration: None,
            timer: Some(TimerSpec { work: 10.0, brk: 15.0, long_break: 20.0, rounds: 4 }),
            x: 0.5,
            y: 0.5,
            chime_only: None,
        };
        let mut s = Session::new(opts, modes::build(Mode::Focus, &p), Some(modes::build(Mode::Relax, &p)));
        let mut st = Status::default();
        run(&mut s, 5.0);
        s.fill_status(&mut st);
        assert_eq!(st.mode, "focus");
        assert_eq!(st.timer.unwrap().0, "focus");
        run(&mut s, 13.0);
        s.fill_status(&mut st);
        assert_eq!(st.mode, "relax");
        assert_eq!(st.timer.unwrap().0, "break");
        run(&mut s, 15.0);
        s.fill_status(&mut st);
        assert_eq!(st.mode, "focus");
        assert_eq!(st.timer.unwrap().2, 2);
    }

    #[test]
    fn duration_fades_out_and_finishes() {
        let p = params();
        let opts = SessionOpts { sr: SR, volume: 0.8, duration: Some(10.0), timer: None, x: 0.5, y: 0.5, chime_only: None };
        let mut s = Session::new(opts, modes::build(Mode::Focus, &p), None);
        run(&mut s, 9.0);
        assert!(!s.finished());
        run(&mut s, 9.0);
        assert!(s.finished());
    }

    #[test]
    fn quit_finishes_quickly() {
        let p = params();
        let opts = SessionOpts { sr: SR, volume: 0.8, duration: None, timer: None, x: 0.5, y: 0.5, chime_only: None };
        let mut s = Session::new(opts, modes::build(Mode::Deep, &p), None);
        run(&mut s, 2.0);
        s.command(Command::Quit);
        run(&mut s, 2.0);
        assert!(s.finished());
    }

    #[test]
    fn chime_only_session_finishes_after_the_chime() {
        let opts = SessionOpts {
            sr: SR,
            volume: 0.8,
            duration: None,
            timer: None,
            x: 0.5,
            y: 0.5,
            chime_only: Some(ChimeKind::Break),
        };
        let mut s = Session::new(opts, Box::new(Silence), None);
        run(&mut s, 1.0);
        assert!(!s.finished());
        run(&mut s, 8.0);
        assert!(s.finished());
    }
}
