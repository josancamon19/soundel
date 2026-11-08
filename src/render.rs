//! Offline rendering to a 16-bit WAV file.

use std::path::Path;

use anyhow::Result;

use crate::rng::Rng;
use crate::session::Session;

pub struct RenderStats {
    pub seconds: f64,
    pub peak_db: f32,
    pub rms_db: f32,
}

pub fn to_wav(session: &mut Session, seconds: f64, sample_rate: u32, path: &Path) -> Result<RenderStats> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    let total = (seconds * sample_rate as f64) as usize;
    let mut l = vec![0f32; 4096];
    let mut r = vec![0f32; 4096];
    let mut dither = Rng::new(0xD17);
    let mut peak = 0f32;
    let mut sum_sq = 0f64;
    let mut done = 0;
    while done < total {
        let n = (total - done).min(l.len());
        session.render(&mut l[..n], &mut r[..n]);
        for i in 0..n {
            for x in [l[i], r[i]] {
                peak = peak.max(x.abs());
                sum_sq += (x as f64) * (x as f64);
                let tpdf = (dither.f32() - dither.f32()) / 32768.0;
                w.write_sample(((x + tpdf).clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
            }
        }
        done += n;
    }
    w.finalize()?;
    let rms = (sum_sq / (2 * total.max(1)) as f64).sqrt() as f32;
    Ok(RenderStats { seconds, peak_db: 20.0 * peak.max(1e-9).log10(), rms_db: 20.0 * rms.max(1e-9).log10() })
}
