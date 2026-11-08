//! Scales, registers and chord voicings.

use crate::rng::Rng;

/// Major pentatonic. Endel's Focus stays in C major pentatonic for entire sessions.
pub const PENTA: [i32; 5] = [0, 2, 4, 7, 9];
pub const MINOR_PENTA: [i32; 5] = [0, 3, 5, 7, 10];
pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

pub fn name(pc: i32) -> &'static str {
    NOTE_NAMES[pc.rem_euclid(12) as usize]
}

/// All MIDI notes in [lo, hi] that belong to `scale` rooted at `tonic`.
pub fn scale_notes(tonic: i32, scale: &[i32], lo: i32, hi: i32) -> Vec<i32> {
    (lo..=hi).filter(|m| scale.contains(&(m - tonic).rem_euclid(12))).collect()
}

/// Sub-bass register for a pitch class: E1..D#2 (41–78 Hz), matching the drones measured in Endel.
pub fn sub_midi(pc: i32) -> i32 {
    let mut m = 24 + pc.rem_euclid(12);
    while m < 28 {
        m += 12;
    }
    m
}

/// Bass-line register: A1..G#2.
pub fn bass_midi(pc: i32) -> i32 {
    let m = sub_midi(pc) + 12;
    if m > 45 {
        m - 12
    } else {
        m
    }
}

/// Open pentatonic pad voicing over `root` within C3..B4: root, fifth (when in scale), then
/// 2–3 more scale tones with no clusters tighter than a whole step. Returns the voice count.
pub fn pad_voicing(rng: &mut Rng, root: i32, scale: &[i32], out: &mut [i32; 6]) -> usize {
    let pc = root.rem_euclid(12);
    let r0 = 48 + pc;
    let mut n = 0;
    out[n] = r0;
    n += 1;
    if scale.contains(&((pc + 7) % 12)) {
        out[n] = r0 + 7;
        n += 1;
    }
    let target = if rng.chance(0.5) { 4 } else { 5 };
    let mut cands = [0i32; 24];
    let mut nc = 0;
    for m in (r0 + 3)..=71 {
        if scale.contains(&m.rem_euclid(12)) && nc < cands.len() {
            cands[nc] = m;
            nc += 1;
        }
    }
    for i in (1..nc).rev() {
        let j = rng.int(0, i as i32) as usize;
        cands.swap(i, j);
    }
    let mut seconds = 0;
    for &c in &cands[..nc] {
        if n >= target {
            break;
        }
        let min_gap = out[..n].iter().map(|o| (o - c).abs()).min().unwrap_or(12);
        if min_gap >= 3 || (min_gap == 2 && seconds == 0 && rng.chance(0.35)) {
            if min_gap == 2 {
                seconds += 1;
            }
            out[n] = c;
            n += 1;
        }
    }
    out[..n].sort_unstable();
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_register_is_deep() {
        for pc in 0..12 {
            let f = crate::dsp::midi_hz(sub_midi(pc) as f32);
            assert!((41.0..80.0).contains(&f), "pc {pc} -> {f} Hz");
        }
    }

    #[test]
    fn voicings_are_pentatonic_open_and_in_range() {
        let mut rng = Rng::new(9);
        for root in [0, 2, 4, 7, 9] {
            for _ in 0..200 {
                let mut v = [0; 6];
                let n = pad_voicing(&mut rng, root, &PENTA, &mut v);
                assert!((3..=5).contains(&n), "{n} voices");
                assert_eq!(v[0].rem_euclid(12), root);
                for w in v[..n].windows(2) {
                    assert!(w[1] - w[0] >= 2, "cluster in {:?}", &v[..n]);
                }
                for m in &v[..n] {
                    assert!(PENTA.contains(&m.rem_euclid(12)));
                    assert!((48..=71).contains(m));
                }
            }
        }
    }
}
