//! soundel: generative focus soundscapes for the terminal. Offline, endless, no account.

mod audio;
mod chimes;
mod dancer;
mod dsp;
mod engine;
mod modes;
mod music;
mod render;
mod rng;
mod session;
mod timefmt;
mod ui;
mod voices;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use chrono::Timelike;
use clap::{Args, Parser, Subcommand};

use crate::chimes::ChimeKind;
use crate::engine::{Silence, Status};
use crate::modes::{Mode, ModeParams, NoiseColor};
use crate::session::{Session, SessionOpts, TimerSpec};

#[derive(Parser, Debug)]
#[command(
    name = "soundel",
    version,
    about = "Generative focus soundscapes in your terminal. Offline, endless, no account.",
    after_help = "Examples:\n  soundel                      Focus until you press q\n  soundel focus --timer        Pomodoro: 25 min focus / 5 min break, with chimes\n  soundel study --for 1h       Lo-fi beats for an hour, then fade out\n  soundel noise --color brown  Brown noise\n  soundel render focus --for 10m -o focus.wav",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    #[command(flatten)]
    opts: PlayOpts,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Focus (default): pentatonic pulse that adapts to the time of day
    Focus(PlayOpts),
    /// Lo-fi beats for studying (~80 BPM, swung, minor key)
    Study(PlayOpts),
    /// Minimal techno for deep work (~122 BPM)
    Deep(PlayOpts),
    /// Colored noise: pick with --color, move darker/brighter with ←→
    Noise(PlayOpts),
    /// Pentatonic pad tuned to a frequency (--freq, default 528 Hz)
    Tones(PlayOpts),
    /// Binaural beat under a soft pad (--beat, default 40 Hz); use headphones
    Binaural(PlayOpts),
    /// Calm and beatless; also what plays during timer breaks
    Relax(PlayOpts),
    /// Write a soundscape to a WAV file instead of playing it
    Render(RenderArgs),
    /// Play a cue sound
    Chime(ChimeArgs),
}

#[derive(Args, Debug, Clone)]
struct PlayOpts {
    /// Stop after this long, with a fade-out (25m, 1h30m, 90s; a bare number means minutes)
    #[arg(long = "for", value_name = "DURATION", value_parser = timefmt::parse_duration)]
    duration: Option<f64>,
    /// Pomodoro timer: focus blocks and breaks with chimes (beatless relax during breaks)
    #[arg(long)]
    timer: bool,
    /// Timer: focus block length in minutes
    #[arg(long, value_name = "MIN", default_value_t = 25.0)]
    work: f64,
    /// Timer: short break length in minutes
    #[arg(long = "break", value_name = "MIN", default_value_t = 5.0)]
    break_min: f64,
    /// Timer: long break length in minutes
    #[arg(long, value_name = "MIN", default_value_t = 15.0)]
    long_break: f64,
    /// Timer: focus blocks before a long break
    #[arg(long, value_name = "N", default_value_t = 4)]
    rounds: u32,
    /// Tunepad x, 0-1: Fluid (0) to Structured (1). Noise: dark to bright
    #[arg(short = 'x', long, value_name = "0-1")]
    structure: Option<f32>,
    /// Tunepad y, 0-1: Easy (0) to Intense (1). Noise: still to moving
    #[arg(short = 'y', long, value_name = "0-1", default_value_t = 0.5)]
    intensity: f32,
    /// Volume, 0-1.5
    #[arg(short, long, default_value_t = 0.8)]
    volume: f32,
    /// Use this hour of the day (0-23) instead of the local time
    #[arg(long, value_name = "0-23")]
    hour: Option<u32>,
    /// Resting heart rate; Focus plays at twice it (60 → 120 BPM)
    #[arg(long, value_name = "BPM")]
    heart_rate: Option<f32>,
    /// Exact tempo (overrides --heart-rate)
    #[arg(long)]
    bpm: Option<f32>,
    /// Seed for a reproducible session
    #[arg(long)]
    seed: Option<u64>,
    /// Noise color
    #[arg(long, value_enum, default_value_t = NoiseColor::Deep)]
    color: NoiseColor,
    /// Tones: tonic frequency in Hz (solfeggio: 174 285 396 417 528 639 741 852 963)
    #[arg(long, value_name = "HZ", default_value_t = 528.0)]
    freq: f32,
    /// Binaural: beat frequency in Hz (40 gamma, 14 beta, 10 alpha)
    #[arg(long, value_name = "HZ", default_value_t = 40.0)]
    beat: f32,
    /// No live panel (for background use); stop with Ctrl-C
    #[arg(long)]
    no_ui: bool,
    /// Hide Claudia, who dances to the beat next to the panel
    #[arg(long)]
    no_dancer: bool,
    /// Delay Claudia by this many milliseconds (negative: earlier) if she looks off the beat
    #[arg(long, value_name = "MS", default_value_t = 0.0, allow_hyphen_values = true)]
    dancer_delay: f64,
}

#[derive(Args, Debug)]
struct RenderArgs {
    /// Soundscape to render
    #[arg(value_enum, default_value_t = Mode::Focus)]
    mode: Mode,
    /// Output WAV file
    #[arg(short, long, default_value = "soundel.wav")]
    out: PathBuf,
    /// Sample rate of the file
    #[arg(long, default_value_t = 48000)]
    rate: u32,
    #[command(flatten)]
    opts: PlayOpts,
}

#[derive(Args, Debug)]
struct ChimeArgs {
    /// Which cue
    #[arg(value_enum, default_value_t = ChimeKind::Start)]
    kind: ChimeKind,
    /// Volume, 0-1.5
    #[arg(short, long, default_value_t = 0.8)]
    volume: f32,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("soundel: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => play(Mode::Focus, cli.opts),
        Some(Cmd::Focus(o)) => play(Mode::Focus, o),
        Some(Cmd::Study(o)) => play(Mode::Study, o),
        Some(Cmd::Deep(o)) => play(Mode::Deep, o),
        Some(Cmd::Noise(o)) => play(Mode::Noise, o),
        Some(Cmd::Tones(o)) => play(Mode::Tones, o),
        Some(Cmd::Binaural(o)) => play(Mode::Binaural, o),
        Some(Cmd::Relax(o)) => play(Mode::Relax, o),
        Some(Cmd::Render(a)) => render_cmd(a),
        Some(Cmd::Chime(a)) => chime(a),
    }
}

fn mode_name(mode: Mode) -> String {
    format!("{mode:?}").to_lowercase()
}

fn validate(o: &PlayOpts) -> Result<()> {
    if o.hour.is_some_and(|h| h > 23) {
        bail!("--hour must be between 0 and 23");
    }
    if o.structure.is_some_and(|x| !(0.0..=1.0).contains(&x)) {
        bail!("-x/--structure must be between 0 and 1");
    }
    if !(0.0..=1.0).contains(&o.intensity) {
        bail!("-y/--intensity must be between 0 and 1");
    }
    if !(0.0..=1.5).contains(&o.volume) {
        bail!("--volume must be between 0 and 1.5");
    }
    if !(40.0..=2000.0).contains(&o.freq) {
        bail!("--freq must be between 40 and 2000 Hz");
    }
    if !(0.5..=60.0).contains(&o.beat) {
        bail!("--beat must be between 0.5 and 60 Hz");
    }
    if o.bpm.is_some_and(|b| !(40.0..=200.0).contains(&b)) {
        bail!("--bpm must be between 40 and 200");
    }
    if o.heart_rate.is_some_and(|h| !(35.0..=120.0).contains(&h)) {
        bail!("--heart-rate must be between 35 and 120");
    }
    if !(-2000.0..=2000.0).contains(&o.dancer_delay) {
        bail!("--dancer-delay must be between -2000 and 2000 ms");
    }
    if o.timer && (o.work <= 0.0 || o.break_min <= 0.0 || o.long_break <= 0.0 || o.rounds == 0) {
        bail!("timer lengths and rounds must be positive");
    }
    Ok(())
}

fn build_session(mode: Mode, o: &PlayOpts, sr: f32) -> Result<Session> {
    validate(o)?;
    let seed = o.seed.unwrap_or_else(|| {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
    });
    let hour = o.hour.map(|h| h as f32).unwrap_or_else(|| {
        let now = chrono::Local::now();
        now.hour() as f32 + now.minute() as f32 / 60.0
    });
    let x = o.structure.unwrap_or_else(|| if mode == Mode::Noise { o.color.position().unwrap_or(0.5) } else { 0.5 });
    let bpm = match mode {
        Mode::Focus | Mode::Relax => o.bpm.or(o.heart_rate.map(|hr| (hr * 2.0).clamp(96.0, 150.0))),
        _ => o.bpm,
    };
    let p = ModeParams { sr, seed, hour, x, y: o.intensity, bpm, color: o.color, freq: o.freq, beat: o.beat };
    let main = modes::build(mode, &p);
    let timer = o.timer.then(|| TimerSpec {
        work: o.work * 60.0,
        brk: o.break_min * 60.0,
        long_break: o.long_break * 60.0,
        rounds: o.rounds,
    });
    let relax = timer.map(|_| {
        let rp = ModeParams { seed: seed.wrapping_add(0x9E37_79B9), bpm: if mode == Mode::Focus { bpm } else { None }, ..p };
        modes::build(Mode::Relax, &rp)
    });
    let opts = SessionOpts { sr, volume: o.volume, duration: o.duration, timer, x, y: o.intensity, chime_only: None };
    Ok(Session::new(opts, main, relax))
}

fn play(mode: Mode, o: PlayOpts) -> Result<()> {
    let output = audio::Output::open()?;
    let sr = output.sample_rate();
    let device = output.device_name();
    let session = build_session(mode, &o, sr as f32)?;
    let (tx, rx) = mpsc::channel();
    let status = Arc::new(Mutex::new(Status::default()));
    let finished = Arc::new(AtomicBool::new(false));
    let _stream = output.start(session, rx, status.clone(), finished.clone())?;
    let interactive = !o.no_ui && io::stdin().is_terminal() && io::stdout().is_terminal();
    if interactive {
        println!("soundel {} · {} · {:.1} kHz", mode_name(mode), device, sr as f32 / 1000.0);
        if mode == Mode::Binaural {
            println!("  binaural beats only work on headphones");
        }
        let ui_opts = ui::UiOpts { dancer: !o.no_dancer, dancer_delay: o.dancer_delay / 1000.0 };
        ui::interactive(&tx, &status, &finished, &ui_opts)
    } else {
        eprintln!("soundel: playing {} on {device} (Ctrl-C to stop)", mode_name(mode));
        ui::headless(tx, &finished)
    }
}

fn render_cmd(a: RenderArgs) -> Result<()> {
    let secs = a.opts.duration.unwrap_or(300.0);
    let mut o = a.opts.clone();
    o.duration = Some(secs);
    let mut session = build_session(a.mode, &o, a.rate as f32)?;
    eprintln!("rendering {} of {} to {}", timefmt::clock(secs), mode_name(a.mode), a.out.display());
    let st = render::to_wav(&mut session, secs, a.rate, &a.out)?;
    println!(
        "wrote {} ({}, peak {:.1} dBFS, rms {:.1} dBFS)",
        a.out.display(),
        timefmt::clock(st.seconds),
        st.peak_db,
        st.rms_db
    );
    Ok(())
}

fn chime(a: ChimeArgs) -> Result<()> {
    if !(0.0..=1.5).contains(&a.volume) {
        bail!("--volume must be between 0 and 1.5");
    }
    let output = audio::Output::open()?;
    let sr = output.sample_rate() as f32;
    let opts = SessionOpts { sr, volume: a.volume, duration: None, timer: None, x: 0.5, y: 0.5, chime_only: Some(a.kind) };
    let session = Session::new(opts, Box::new(Silence), None);
    let (_tx, rx) = mpsc::channel();
    let finished = Arc::new(AtomicBool::new(false));
    let _stream = output.start(session, rx, Arc::new(Mutex::new(Status::default())), finished.clone())?;
    let start = Instant::now();
    while !finished.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
