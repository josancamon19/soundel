//! The soundscapes.

pub mod binaural;
pub mod deep;
pub mod focus;
pub mod noise;
pub mod study;
pub mod tones;

use clap::ValueEnum;

use crate::engine::Generator;
pub use noise::NoiseColor;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    Focus,
    Study,
    Deep,
    Noise,
    Tones,
    Binaural,
    Relax,
}

#[derive(Clone, Copy, Debug)]
pub struct ModeParams {
    pub sr: f32,
    pub seed: u64,
    pub hour: f32,
    pub x: f32,
    pub y: f32,
    pub bpm: Option<f32>,
    pub color: NoiseColor,
    pub freq: f32,
    pub beat: f32,
}

pub fn build(mode: Mode, p: &ModeParams) -> Box<dyn Generator> {
    match mode {
        Mode::Focus | Mode::Relax => Box::new(focus::Focus::new(focus::FocusParams {
            sr: p.sr,
            seed: p.seed,
            hour: p.hour,
            x: p.x,
            y: p.y,
            bpm: p.bpm.unwrap_or(120.0),
            relax: mode == Mode::Relax,
        })),
        Mode::Study => Box::new(study::Study::new(p.sr, p.seed, p.hour, p.x, p.y, p.bpm)),
        Mode::Deep => Box::new(deep::Deep::new(p.sr, p.seed, p.hour, p.x, p.y, p.bpm)),
        Mode::Noise => Box::new(noise::NoiseGen::new(p.sr, p.seed, p.color, p.x, p.y)),
        Mode::Tones => Box::new(tones::Tones::new(p.sr, p.seed, p.freq, p.x, p.y)),
        Mode::Binaural => Box::new(binaural::Binaural::new(p.sr, p.seed, p.beat, p.x, p.y)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Mode; 7] = [Mode::Focus, Mode::Study, Mode::Deep, Mode::Noise, Mode::Tones, Mode::Binaural, Mode::Relax];

    fn params(seed: u64) -> ModeParams {
        ModeParams {
            sr: 16_000.0,
            seed,
            hour: 10.0,
            x: 0.5,
            y: 0.5,
            bpm: None,
            color: NoiseColor::Deep,
            freq: 528.0,
            beat: 40.0,
        }
    }

    fn render(mode: Mode, seed: u64, secs: usize) -> (Vec<f32>, Vec<f32>) {
        let mut g = build(mode, &params(seed));
        let n = 16_000 * secs;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        g.render(&mut l, &mut r);
        (l, r)
    }

    #[test]
    fn every_mode_renders_finite_audio_at_a_sane_level() {
        for mode in ALL {
            let (l, r) = render(mode, 3, 25);
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()), "{mode:?} produced NaN/inf");
            let tail = &l[l.len() / 2..];
            let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
            let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
            let rms_db = 20.0 * rms.log10();
            assert!((-45.0..-8.0).contains(&rms_db), "{mode:?} rms {rms_db:.1} dBFS");
            assert!(peak < 1.6, "{mode:?} peak {peak}");
        }
    }

    #[test]
    fn every_mode_is_deterministic_for_a_seed() {
        for mode in ALL {
            assert_eq!(render(mode, 11, 3), render(mode, 11, 3), "{mode:?}");
        }
    }

    #[test]
    fn binaural_ears_differ_by_the_beat() {
        let (l, r) = render(Mode::Binaural, 5, 20);
        let corr = l.iter().zip(&r).map(|(a, b)| a * b).sum::<f32>()
            / (l.iter().map(|a| a * a).sum::<f32>().sqrt() * r.iter().map(|b| b * b).sum::<f32>().sqrt());
        assert!(corr < 0.9, "ears too similar: {corr}");
    }
}
