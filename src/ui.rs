//! Terminal front end: a live panel with keyboard control and Claudia dancing beside it, or a
//! headless mode.

use std::collections::VecDeque;
use std::io::{self, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::{Print, Stylize};
use crossterm::{cursor, execute, queue, terminal};

use crate::dancer::{self, Dancer};
use crate::engine::Status;
use crate::session::Command;
use crate::timefmt::clock;

const TEXT_LINES: usize = 4;
const QUIT_GRACE: Duration = Duration::from_secs(4);
/// About 60 frames a second; only rows that changed are redrawn.
const FRAME: Duration = Duration::from_millis(16);
/// Columns between the dancer and the text.
const GAP: usize = 2;

pub struct UiOpts {
    /// Show Claudia dancing next to the panel.
    pub dancer: bool,
    /// Seconds to delay her (negative: advance) if she looks off the beat.
    pub dancer_delay: f64,
}

struct RawGuard;

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), cursor::Show);
    }
}

#[derive(Clone, Copy)]
enum Look {
    Bold,
    Plain,
    Dim,
}

pub fn interactive(
    tx: &Sender<Command>,
    status: &Arc<Mutex<Status>>,
    finished: &Arc<AtomicBool>,
    opts: &UiOpts,
) -> Result<()> {
    let mut out = io::stdout();
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let dancer = (opts.dancer && rows as usize > dancer::ROWS + 2 && cols as usize >= dancer::W + GAP + 40)
        .then(|| Dancer::new(dancer::truecolor()));
    let lines = if dancer.is_some() { dancer::ROWS } else { TEXT_LINES };
    terminal::enable_raw_mode()?;
    let _guard = RawGuard;
    execute!(out, cursor::Hide)?;
    for _ in 0..lines {
        write!(out, "\r\n")?;
    }
    execute!(out, cursor::MoveUp(lines as u16), cursor::SavePosition)?;

    let mut screen = Screen { drawn: vec![String::new(); lines], text_top: (lines - TEXT_LINES) / 2 };
    let mut beat_clock = BeatClock::default();
    let start = Instant::now();
    let mut quit_at: Option<Instant> = None;
    loop {
        if finished.load(Ordering::Relaxed) || quit_at.is_some_and(|t| t.elapsed() > QUIT_GRACE) {
            break;
        }
        if event::poll(FRAME)? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Release {
                    continue;
                }
                let cmd = match k.code {
                    KeyCode::Left | KeyCode::Char('h') => Some(Command::Tune(-0.1, 0.0)),
                    KeyCode::Right | KeyCode::Char('l') => Some(Command::Tune(0.1, 0.0)),
                    KeyCode::Up | KeyCode::Char('k') => Some(Command::Tune(0.0, 0.1)),
                    KeyCode::Down | KeyCode::Char('j') => Some(Command::Tune(0.0, -0.1)),
                    KeyCode::Char('+') | KeyCode::Char('=') => Some(Command::Volume(0.05)),
                    KeyCode::Char('-') | KeyCode::Char('_') => Some(Command::Volume(-0.05)),
                    KeyCode::Char(' ') | KeyCode::Char('p') => Some(Command::Pause),
                    KeyCode::Char('n') => Some(Command::Skip),
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Some(Command::Quit),
                    KeyCode::Char('q') | KeyCode::Esc => Some(Command::Quit),
                    _ => None,
                };
                if let Some(c) = cmd {
                    if matches!(c, Command::Quit) && quit_at.is_none() {
                        quit_at = Some(Instant::now());
                    }
                    let _ = tx.send(c);
                }
            }
        }
        let st = *status.lock().unwrap();
        beat_clock.push(&st);
        let figure = dancer.as_ref().map(|d| figure(d, &mut beat_clock, &st, start, opts.dancer_delay));
        screen.draw(&mut out, &st, figure.as_deref())?;
    }
    let st = *status.lock().unwrap();
    let figure = dancer.as_ref().map(|d| figure(d, &mut beat_clock, &st, start, opts.dancer_delay));
    screen.draw(&mut out, &st, figure.as_deref())?;
    execute!(out, cursor::RestorePosition, cursor::MoveDown(lines as u16 - 1))?;
    write!(out, "\r\n")?;
    out.flush()?;
    Ok(())
}

pub fn headless(tx: Sender<Command>, finished: &Arc<AtomicBool>) -> Result<()> {
    let quit = Arc::new(AtomicBool::new(false));
    let q = quit.clone();
    ctrlc::set_handler(move || {
        let _ = tx.send(Command::Quit);
        q.store(true, Ordering::Relaxed);
    })?;
    let mut quit_since: Option<Instant> = None;
    while !finished.load(Ordering::Relaxed) {
        if quit.load(Ordering::Relaxed) && quit_since.get_or_insert_with(Instant::now).elapsed() > QUIT_GRACE {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Claudia for what's being heard right now (shifted by the user's delay).
fn figure(d: &Dancer, beat_clock: &mut BeatClock, latest: &Status, start: Instant, delay: f64) -> Vec<String> {
    let now = Instant::now();
    let at = if delay >= 0.0 {
        now.checked_sub(Duration::from_secs_f64(delay)).unwrap_or(now)
    } else {
        now + Duration::from_secs_f64(-delay)
    };
    let (st, beats) = beat_clock.at(at).unwrap_or((*latest, latest.beats));
    d.frame(&st, beats, now.duration_since(start).as_secs_f64())
}

/// Status snapshots replayed when their audio reaches the listener, so the dancer follows what
/// you hear rather than what the engine is rendering 20–200 ms ahead of it.
#[derive(Default)]
struct BeatClock {
    queue: VecDeque<Status>,
}

impl BeatClock {
    fn push(&mut self, st: &Status) {
        let Some(heard) = st.heard_at else { return };
        if self.queue.back().and_then(|b| b.heard_at).is_some_and(|last| last >= heard) {
            return;
        }
        self.queue.push_back(*st);
        if self.queue.len() > 256 {
            self.queue.pop_front();
        }
    }

    /// The snapshot being heard at `t`, and the beat position at `t`.
    fn at(&mut self, t: Instant) -> Option<(Status, f64)> {
        while self.queue.len() > 1 && self.queue[1].heard_at.is_some_and(|h| h <= t) {
            self.queue.pop_front();
        }
        let s = *self.queue.front()?;
        let heard = s.heard_at?;
        let dt = if t >= heard { (t - heard).as_secs_f64() } else { -(heard - t).as_secs_f64() };
        let beats = if s.paused { s.beats } else { s.beats + dt * s.bpm as f64 / 60.0 };
        Some((s, beats))
    }
}

/// What's on the panel's lines, so only changed rows are rewritten.
struct Screen {
    drawn: Vec<String>,
    text_top: usize,
}

impl Screen {
    fn draw(&mut self, out: &mut Stdout, st: &Status, figure: Option<&[String]>) -> Result<()> {
        let width = match terminal::size() {
            Ok((w, _)) if w >= 20 => w as usize,
            _ => 80,
        };
        // A terminal narrowed since we started keeps the text and drops her.
        let figure = figure.filter(|_| width > dancer::W + GAP + 20);
        let left = if figure.is_some() { dancer::W + GAP } else { 0 };
        let room = width.saturating_sub(left + 1);
        let text = panel(st, room.min(76));
        let mut changed = false;
        for i in 0..self.drawn.len() {
            let mut row = String::new();
            if let Some(f) = figure {
                row += &f[i];
                row += &" ".repeat(GAP);
            }
            if let Some((t, look)) = i.checked_sub(self.text_top).and_then(|j| text.get(j)) {
                let t: String = t.chars().take(room).collect();
                row += &match look {
                    Look::Bold => t.bold().to_string(),
                    Look::Dim => t.dim().to_string(),
                    Look::Plain => t,
                };
            }
            if row == self.drawn[i] {
                continue;
            }
            if !changed {
                queue!(out, terminal::BeginSynchronizedUpdate)?;
                changed = true;
            }
            queue!(out, cursor::RestorePosition)?;
            if i > 0 {
                queue!(out, cursor::MoveDown(i as u16))?;
            }
            queue!(out, cursor::MoveToColumn(0), Print(&row), terminal::Clear(terminal::ClearType::UntilNewLine))?;
            self.drawn[i] = row;
        }
        if changed {
            queue!(out, terminal::EndSynchronizedUpdate)?;
            out.flush()?;
        }
        Ok(())
    }
}

/// The text lines; `cols` is where the clock is right-aligned.
fn panel(st: &Status, cols: usize) -> Vec<(String, Look)> {
    let mut head = format!(" ◉ {}", st.mode.to_uppercase());
    if st.bpm > 0.0 {
        head += &format!(" · {:.0} BPM", st.bpm);
    }
    if st.value > 0.0 {
        head += &format!(" · {:.0} {}", st.value, st.value_unit);
    }
    if !st.key.is_empty() {
        head += &format!(" · {}", st.key);
    }
    if !st.root.is_empty() {
        head += &format!(", root {}", st.root);
    }
    if !st.section.is_empty() {
        head += &format!(" · {}", st.section);
    }
    if !st.beat.is_empty() {
        head += &format!(" · {}", st.beat);
    }
    let time = clock(st.elapsed);
    let pad = cols.saturating_sub(head.chars().count() + time.len()).max(3);
    head += &" ".repeat(pad);
    head += &time;

    let tune = format!(
        "   {} {} {}    {} {} {}    vol {}",
        st.x_labels.0,
        slider(st.x),
        st.x_labels.1,
        st.y_labels.0,
        slider(st.y),
        st.y_labels.1,
        volume(st.volume)
    );

    let mut info = vec![];
    if !st.phase.is_empty() {
        info.push(st.phase.to_string());
    }
    if let Some((phase, left, round, rounds)) = st.timer {
        info.push(format!("timer: {phase}, {} left (round {round}/{rounds})", clock(left)));
    }
    if let Some(e) = st.ends_in {
        info.push(format!("ends in {}", clock(e)));
    }
    if st.paused {
        info.push("paused".into());
    }
    if st.ending {
        info.push("fading out".into());
    }
    let info = format!("   {}", info.join(" · "));
    let help = "   ←→ ↑↓ tune   +/- volume   space pause   n next section   q quit".to_string();
    vec![(head, Look::Bold), (tune, Look::Plain), (info, Look::Plain), (help, Look::Dim)]
}

fn slider(v: f32) -> String {
    let n = 11;
    let pos = (v.clamp(0.0, 1.0) * (n - 1) as f32).round() as usize;
    (0..n).map(|i| if i == pos { '●' } else { '─' }).collect()
}

fn volume(v: f32) -> String {
    let n = 8;
    let k = ((v / 1.5).clamp(0.0, 1.0) * n as f32).round() as usize;
    format!("{}{} {:>3.0}%", "█".repeat(k), "░".repeat(n - k), v * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_shows_the_essentials() {
        let st = Status {
            mode: "focus",
            bpm: 120.0,
            key: "C pentatonic",
            root: "G",
            section: "pulse",
            beat: "four on the floor",
            phase: "in flow",
            elapsed: 125.0,
            timer: Some(("focus", 600.0, 2, 4)),
            ..Status::default()
        };
        let p = panel(&st, 76);
        assert!(p[0].0.contains("FOCUS · 120 BPM · C pentatonic, root G · pulse · four on the floor"));
        assert!(p[0].0.ends_with("02:05"));
        assert!(p[1].0.contains("Fluid ─────●───── Structured"));
        assert!(p[2].0.contains("timer: focus, 10:00 left (round 2/4)"));
    }

    #[test]
    fn beat_clock_plays_snapshots_when_they_are_heard() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let snap = |beats: f64, section: &'static str, at: u64| Status {
            bpm: 120.0,
            beats,
            section,
            heard_at: Some(ms(at)),
            ..Status::default()
        };
        let mut c = BeatClock::default();
        c.push(&snap(10.0, "pulse", 200));
        c.push(&snap(10.1, "groove", 250));
        c.push(&snap(10.1, "groove", 250)); // the same snapshot read twice
        assert_eq!(c.queue.len(), 2);
        // Before anything is heard, extrapolate backwards from the first snapshot.
        let (s, beats) = c.at(ms(100)).unwrap();
        assert_eq!(s.section, "pulse");
        assert!((beats - 9.8).abs() < 1e-9);
        // Between snapshots, the earlier one plays on; 120 BPM is 2 beats a second.
        let (s, beats) = c.at(ms(240)).unwrap();
        assert_eq!(s.section, "pulse");
        assert!((beats - 10.08).abs() < 1e-9);
        let (s, beats) = c.at(ms(300)).unwrap();
        assert_eq!(s.section, "groove");
        assert!((beats - 10.2).abs() < 1e-9);
        assert_eq!(c.queue.len(), 1);
    }

    #[test]
    fn a_paused_clock_stands_still() {
        let t0 = Instant::now();
        let mut c = BeatClock::default();
        c.push(&Status { bpm: 120.0, beats: 4.0, paused: true, heard_at: Some(t0), ..Status::default() });
        let (_, beats) = c.at(t0 + Duration::from_secs(3)).unwrap();
        assert_eq!(beats, 4.0);
    }
}
