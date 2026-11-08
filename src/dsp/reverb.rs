//! 8-line feedback delay network reverb with input diffusion and in-loop damping.

use super::OnePole;

struct Line {
    buf: Vec<f32>,
    idx: usize,
}

impl Line {
    fn new(len: usize) -> Self {
        Line { buf: vec![0.0; len.max(1)], idx: 0 }
    }
    #[inline]
    fn read(&self) -> f32 {
        self.buf[self.idx]
    }
    #[inline]
    fn push(&mut self, v: f32) {
        self.buf[self.idx] = v;
        self.idx += 1;
        if self.idx == self.buf.len() {
            self.idx = 0;
        }
    }
}

struct Allpass {
    line: Line,
    g: f32,
}

impl Allpass {
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let wd = self.line.read();
        let w = x + self.g * wd;
        self.line.push(w);
        wd - self.g * w
    }
}

const LINE_MS: [f32; 8] = [31.7, 37.3, 41.9, 46.1, 53.9, 61.3, 67.1, 73.7];

pub struct Reverb {
    sr: f32,
    lines: Vec<Line>,
    lens: [usize; 8],
    gains: [f32; 8],
    damp: [OnePole; 8],
    pre_l: Line,
    pre_r: Line,
    diff_l: Vec<Allpass>,
    diff_r: Vec<Allpass>,
    hp_l: OnePole,
    hp_r: OnePole,
}

impl Reverb {
    pub fn new(sr: f32, rt60: f32, size: f32, damping_hz: f32) -> Self {
        let ms = |m: f32| ((m * size * sr / 1000.0) as usize).max(8);
        let mut lens = [0usize; 8];
        let mut lines = Vec::with_capacity(8);
        for (i, m) in LINE_MS.iter().enumerate() {
            lens[i] = ms(*m);
            lines.push(Line::new(lens[i]));
        }
        let ap = |m: f32| Allpass { line: Line::new(((m * sr / 1000.0) as usize).max(4)), g: 0.6 };
        let mut r = Reverb {
            sr,
            lines,
            lens,
            gains: [0.0; 8],
            damp: [OnePole::new(damping_hz, sr); 8],
            pre_l: Line::new((0.018 * sr) as usize),
            pre_r: Line::new((0.021 * sr) as usize),
            diff_l: vec![ap(4.1), ap(6.3), ap(9.7)],
            diff_r: vec![ap(4.7), ap(6.9), ap(10.3)],
            hp_l: OnePole::new(140.0, sr),
            hp_r: OnePole::new(140.0, sr),
        };
        r.set_rt60(rt60);
        r
    }

    pub fn set_rt60(&mut self, rt60: f32) {
        for i in 0..8 {
            self.gains[i] = 10f32.powf(-3.0 * self.lens[i] as f32 / (rt60.max(0.1) * self.sr));
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let pl = self.pre_l.read();
        self.pre_l.push(l);
        let pr = self.pre_r.read();
        self.pre_r.push(r);
        let mut dl = self.hp_l.hp(pl);
        let mut dr = self.hp_r.hp(pr);
        for a in self.diff_l.iter_mut() {
            dl = a.tick(dl);
        }
        for a in self.diff_r.iter_mut() {
            dr = a.tick(dr);
        }
        let mut o = [0f32; 8];
        for i in 0..8 {
            o[i] = self.damp[i].lp(self.lines[i].read());
        }
        // Householder feedback matrix: A = I - (2/N)·11ᵀ (lossless, maximally diffuse).
        let s = (o[0] + o[1] + o[2] + o[3] + o[4] + o[5] + o[6] + o[7]) * 0.25;
        for i in 0..8 {
            let input = if i % 2 == 0 { dl } else { dr };
            self.lines[i].push((o[i] - s) * self.gains[i] + input * 0.5);
        }
        let out_l = o[0] - o[2] + o[4] - o[6];
        let out_r = o[1] - o[3] + o[5] - o[7];
        (out_l * 0.5, out_r * 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_decays_and_stays_finite() {
        let sr = 48_000.0;
        let mut r = Reverb::new(sr, 1.5, 1.0, 6000.0);
        let mut early = 0.0f32;
        let mut late = 0.0f32;
        for i in 0..(sr as usize * 4) {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (a, b) = r.tick(x, x);
            assert!(a.is_finite() && b.is_finite());
            let e = a * a + b * b;
            if i < sr as usize / 4 {
                early += e;
            } else if i > sr as usize * 3 {
                late += e;
            }
        }
        assert!(early > 0.0);
        assert!(late < early * 1e-4, "tail too long: early {early} late {late}");
    }
}
