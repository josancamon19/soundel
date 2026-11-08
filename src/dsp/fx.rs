//! Effects: ping-pong delay and the master peak limiter.

use super::{tau_coef, OnePole, Smooth};

/// Stereo ping-pong delay with filtered feedback.
pub struct PingPong {
    sr: f32,
    bl: Vec<f32>,
    br: Vec<f32>,
    idx: usize,
    time: Smooth,
    pub feedback: f32,
    lp_l: OnePole,
    lp_r: OnePole,
}

impl PingPong {
    pub fn new(sr: f32, max_secs: f32) -> Self {
        let n = (max_secs * sr) as usize + 2;
        PingPong {
            sr,
            bl: vec![0.0; n],
            br: vec![0.0; n],
            idx: 0,
            time: Smooth::new(0.375, 0.2, sr),
            feedback: 0.35,
            lp_l: OnePole::new(3200.0, sr),
            lp_r: OnePole::new(3200.0, sr),
        }
    }

    pub fn set_time(&mut self, secs: f32) {
        let max = (self.bl.len() - 2) as f32 / self.sr;
        self.time.snap(secs.clamp(0.01, max));
    }

    pub fn set_tone(&mut self, hz: f32) {
        self.lp_l.set(hz, self.sr);
        self.lp_r.set(hz, self.sr);
    }

    #[inline]
    pub fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let len = self.bl.len();
        let d = self.time.next() * self.sr;
        let mut rp = self.idx as f32 - d;
        if rp < 0.0 {
            rp += len as f32;
        }
        let i0 = rp as usize % len;
        let i1 = (i0 + 1) % len;
        let fr = rp - rp.floor();
        let yl = self.bl[i0] + (self.bl[i1] - self.bl[i0]) * fr;
        let yr = self.br[i0] + (self.br[i1] - self.br[i0]) * fr;
        let fb_l = self.lp_l.lp(yr * self.feedback);
        let fb_r = self.lp_r.lp(yl * self.feedback);
        self.bl[self.idx] = (in_l + in_r) * 0.5 + fb_l;
        self.br[self.idx] = fb_r;
        self.idx = (self.idx + 1) % len;
        (yl, yr)
    }
}

/// Soft clipper that never exceeds `c`.
#[inline]
pub fn soft_clip(x: f32, c: f32) -> f32 {
    let a = x.abs();
    let knee = c * 0.9;
    if a <= knee {
        x
    } else {
        let k = c - knee;
        x.signum() * (knee + k * ((a - knee) / k).tanh())
    }
}

/// Look-ahead peak limiter followed by a soft clipper.
pub struct Limiter {
    bl: Vec<f32>,
    br: Vec<f32>,
    idx: usize,
    gain: f32,
    att: f32,
    rel: f32,
    ceiling: f32,
}

impl Limiter {
    pub fn new(sr: f32, ceiling_db: f32) -> Self {
        let n = ((0.003 * sr) as usize).max(1);
        Limiter {
            bl: vec![0.0; n],
            br: vec![0.0; n],
            idx: 0,
            gain: 1.0,
            att: tau_coef(0.0008, sr),
            rel: tau_coef(0.25, sr),
            ceiling: super::db(ceiling_db),
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let peak = l.abs().max(r.abs());
        let want = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
        let c = if want < self.gain { self.att } else { self.rel };
        self.gain += (want - self.gain) * c;
        let dl = self.bl[self.idx];
        let dr = self.br[self.idx];
        self.bl[self.idx] = l;
        self.br[self.idx] = r;
        self.idx = (self.idx + 1) % self.bl.len();
        (soft_clip(dl * self.gain, self.ceiling), soft_clip(dr * self.gain, self.ceiling))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_never_exceeds_ceiling() {
        let sr = 48_000.0;
        let mut lim = Limiter::new(sr, -1.0);
        let ceiling = crate::dsp::db(-1.0);
        for i in 0..48_000 {
            let x = (i as f32 * 0.05).sin() * if i % 9000 < 300 { 4.0 } else { 0.5 };
            let (a, b) = lim.tick(x, -x);
            assert!(a.abs() <= ceiling + 1e-6 && b.abs() <= ceiling + 1e-6);
        }
    }

    #[test]
    fn delay_echoes_after_its_time() {
        let sr = 1000.0;
        let mut d = PingPong::new(sr, 1.0);
        d.set_time(0.1);
        d.feedback = 0.0;
        let mut first = None;
        for i in 0..300 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (l, _) = d.tick(x, x);
            if l.abs() > 0.1 && first.is_none() {
                first = Some(i);
            }
        }
        assert_eq!(first, Some(100));
    }
}
