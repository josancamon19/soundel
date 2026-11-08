//! The interface every soundscape implements, plus the status snapshot shown in the UI.

use std::time::Instant;

pub trait Generator: Send {
    /// Overwrite `l` and `r` with the next block of audio.
    fn render(&mut self, l: &mut [f32], r: &mut [f32]);
    /// Tunepad position, both in [0, 1].
    fn tune(&mut self, _x: f32, _y: f32) {}
    /// Move on to the next section at the next bar line.
    fn skip(&mut self) {}
    fn status(&self, st: &mut Status);
}

/// Snapshot of what is playing. `Copy` with static strings so the audio thread never allocates.
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub mode: &'static str,
    pub key: &'static str,
    pub root: &'static str,
    pub section: &'static str,
    pub beat: &'static str,
    pub phase: &'static str,
    pub bpm: f32,
    pub value: f32,
    pub value_unit: &'static str,
    pub x: f32,
    pub y: f32,
    pub x_labels: (&'static str, &'static str),
    pub y_labels: (&'static str, &'static str),
    pub volume: f32,
    pub paused: bool,
    pub elapsed: f64,
    /// (phase name, seconds left, round, rounds per cycle)
    pub timer: Option<(&'static str, f64, u32, u32)>,
    pub ends_in: Option<f64>,
    pub ending: bool,
    pub finished: bool,
    /// Beat clock for the dancer: position in beats at the end of the audio rendered so far
    /// (bars start on multiples of 4).
    pub beats: f64,
    /// Which 16th steps of the current bar have a kick, and a snap or snare.
    pub kick: u16,
    pub snap: u16,
    /// How present the drums are, 0 (none) to 1.
    pub groove: f32,
    /// When the audio at `beats` reaches the listener; set by the audio thread.
    pub heard_at: Option<Instant>,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            mode: "",
            key: "",
            root: "",
            section: "",
            beat: "",
            phase: "",
            bpm: 0.0,
            value: 0.0,
            value_unit: "",
            x: 0.5,
            y: 0.5,
            x_labels: ("Fluid", "Structured"),
            y_labels: ("Easy", "Intense"),
            volume: 0.8,
            paused: false,
            elapsed: 0.0,
            timer: None,
            ends_in: None,
            ending: false,
            finished: false,
            beats: 0.0,
            kick: 0,
            snap: 0,
            groove: 0.0,
            heard_at: None,
        }
    }
}

/// Plays nothing; used by `soundel chime`.
pub struct Silence;

impl Generator for Silence {
    fn render(&mut self, l: &mut [f32], r: &mut [f32]) {
        l.fill(0.0);
        r.fill(0.0);
    }
    fn status(&self, st: &mut Status) {
        st.mode = "chime";
    }
}
