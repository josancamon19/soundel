//! Claudia dancing on the beat next to the panel.
//!
//! Claudia is the pop singer from claudia.gallery: a glossy black bob with heavy bangs, one
//! clay-orange streak on her right (the viewer's left), a clay star clip above her left ear and
//! a headset mic. Her clothes change with every world; here she wears her BLISS desktop-dancer
//! look, a holographic silver mini dress and white platform boots. She is pixel art, two pixels
//! per terminal cell using half blocks.
//!
//! Timing comes from the audio engine (see `ui::BeatClock`): she dips on every kick you hear,
//! answers the snaps with her arms, changes moves every 8 bars with a pose on the last beat,
//! sways when there is no beat and stands still when paused.

use crate::engine::Status;

/// Canvas size: one pixel per terminal column, two per row.
pub const W: usize = 23;
const H: usize = 26;
pub const ROWS: usize = H / 2;
const CX: i32 = 11;
const FLOOR: i32 = H as i32 - 1;
/// Pixels from hip to sole when standing straight.
const LEG: i32 = 11;
/// Beats per move: she changes moves on 8-bar phrases, like the music's loops.
const PHRASE: f64 = 32.0;

type Rgb = (u8, u8, u8);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ink {
    Clear,
    Hair,
    Sheen,
    Clay,
    Star,
    Skin,
    Shade,
    Eye,
    Lid,
    Lip,
    Mouth,
    Dress,
    Fold,
    Holo(u8),
    Boot,
    Sole,
}

const HOLO: [Rgb; 3] = [(246, 186, 230), (166, 232, 246), (204, 190, 252)];

/// Front view of her head, 11 × 8. K hair, k sheen, O streak, * star clip, S skin, s shade,
/// E eye, M lips. (Her headset mic doesn't survive at this size.)
const HEAD: [&str; 8] = [
    "..KKKKKKK..",
    ".KKOkkKKKK.",
    "KKKOKKKKK*K",
    "KKKOKKKKKKK",
    "KKSESSSESKK",
    "KKSSSSSSSKK",
    "KKsSSMSSsKK",
    ".K.sSSSs.K.",
];
/// Bodice from the shoulders down, centered on the upper body. D dress, F fold, 0-2 holographic.
const BODICE: [&str; 4] = ["DDDDD", "FD0DF", ".DDD.", ".D1D."];
/// Flared mini skirt, centered on the hips.
const SKIRT: [&str; 2] = [".DD2DD.", "FDDD0DF"];
/// Shoulder joints, either side of the upper body's center.
const SHOULDER: i32 = 3;

fn ink(ch: char) -> Ink {
    match ch {
        'K' => Ink::Hair,
        'k' => Ink::Sheen,
        'O' => Ink::Clay,
        '*' => Ink::Star,
        'S' => Ink::Skin,
        's' => Ink::Shade,
        'E' => Ink::Eye,
        'M' => Ink::Lip,
        'D' => Ink::Dress,
        'F' => Ink::Fold,
        '0' => Ink::Holo(0),
        '1' => Ink::Holo(1),
        '2' => Ink::Holo(2),
        _ => Ink::Clear,
    }
}

/// Colors for a frame: the holographic dress cycles on the beat, the star clip glints on kicks.
struct Look {
    shimmer: usize,
    glint: bool,
}

impl Look {
    fn rgb(&self, ink: Ink) -> Option<Rgb> {
        Some(match ink {
            Ink::Clear => return None,
            Ink::Hair => (40, 44, 57),
            Ink::Sheen => (96, 110, 140),
            Ink::Clay => (217, 119, 87),
            Ink::Star if self.glint => (255, 255, 255),
            Ink::Star => (217, 119, 87),
            Ink::Skin => (244, 215, 199),
            Ink::Shade => (222, 180, 162),
            Ink::Eye => (36, 32, 44),
            Ink::Lid => (196, 150, 134),
            Ink::Lip => (200, 112, 116),
            Ink::Mouth => (120, 48, 60),
            Ink::Dress => (206, 212, 226),
            Ink::Fold => (142, 151, 172),
            Ink::Holo(k) => HOLO[(k as usize + self.shimmer) % HOLO.len()],
            Ink::Boot => (247, 247, 250),
            Ink::Sole => (176, 180, 194),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arm {
    Down,
    Hip,
    Up,
    Roof,
    Point,
    Cross,
    Clap,
    /// Hand at her cheek, where the headset mic would be.
    Mic,
    Wave(bool),
}

impl Arm {
    /// Elbow and hand relative to the shoulder, for the arm on the viewer's right.
    fn joints(self) -> ((i32, i32), (i32, i32)) {
        match self {
            Arm::Down => ((1, 3), (1, 6)),
            Arm::Hip => ((2, 2), (-1, 4)),
            Arm::Up => ((2, -3), (3, -6)),
            Arm::Roof => ((3, 0), (3, -3)),
            Arm::Point => ((2, -2), (5, -5)),
            Arm::Cross => ((0, 3), (-4, 5)),
            Arm::Clap => ((1, 3), (-4, 3)),
            Arm::Mic => ((2, 1), (-1, -3)),
            Arm::Wave(out) => ((2, -3), (if out { 4 } else { 2 }, -6)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Pose {
    /// Whole body down (dip) or up (hop), in pixels.
    y: i32,
    /// Whole body sideways (steps).
    x: i32,
    /// Upper body sideways, relative to the hips.
    lean: i32,
    /// Head down (nod).
    nod: i32,
    /// Arms on the viewer's left and right.
    arms: (Arm, Arm),
    /// Feet moved outward, and lifted off the floor.
    feet: (i32, i32),
    lift: (i32, i32),
    eyes_closed: bool,
    mouth_open: bool,
    /// Hair trailing a sideways move: 1 when she moves right, -1 left.
    swing: i32,
}

impl Pose {
    const REST: Pose = Pose {
        y: 0,
        x: 0,
        lean: 0,
        nod: 0,
        arms: (Arm::Down, Arm::Down),
        feet: (0, 0),
        lift: (0, 0),
        eyes_closed: false,
        mouth_open: false,
        swing: 0,
    };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Move {
    Idle,
    Sway,
    Bounce,
    Pump,
    Step,
    Roof,
    Disco,
    Nod,
    Wave,
}

/// Where the music is, from her point of view.
#[derive(Clone, Copy, Debug)]
struct Beat {
    /// Position in beats.
    pos: f64,
    /// Beats since the last kick, and since the last snap with that snap's running index.
    kick: f64,
    snap: f64,
    snap_n: i64,
}

impl Beat {
    fn new(st: &Status, pos: f64) -> Self {
        let step = pos * 4.0;
        let (kick, _) = last_hit(st.kick, step).unwrap_or((f64::INFINITY, 0));
        // Without a backbeat (minimal techno), she answers beats 2 and 4.
        let snaps = if st.snap != 0 || st.kick == 0 { st.snap } else { 1 << 4 | 1 << 12 };
        let (snap, snap_n) = last_hit(snaps, step).unwrap_or((f64::INFINITY, 0));
        Beat { pos, kick, snap, snap_n }
    }
}

/// Beats since the most recent hit at or before 16th note `step` of a repeating one-bar `mask`,
/// and that hit's running index (so alternate hits can alternate arms).
fn last_hit(mask: u16, step: f64) -> Option<(f64, i64)> {
    if mask == 0 {
        return None;
    }
    let i = step.floor() as i64;
    (0..16).map(|back| i - back).find(|j| mask & 1u16 << j.rem_euclid(16) != 0).map(|j| {
        let pos = j.rem_euclid(16) as u32;
        let n = j.div_euclid(16) * mask.count_ones() as i64 + (mask & ((1u16 << pos) - 1)).count_ones() as i64;
        ((step - j as f64) / 4.0, n)
    })
}

/// Pick a move for what's playing. The kind of section sets her repertoire; she moves through
/// it one 8-bar phrase at a time.
fn choose(st: &Status, b: &Beat) -> Move {
    if st.ending && !st.finished {
        return Move::Wave;
    }
    if st.paused || st.finished {
        return Move::Idle;
    }
    if st.bpm <= 0.0 || st.groove < 0.15 {
        return Move::Sway;
    }
    let set: &[Move] = match (st.mode, st.section) {
        ("focus", "pulse") => &[Move::Bounce, Move::Pump, Move::Step],
        ("focus", "arpeggios") => &[Move::Roof, Move::Step, Move::Pump],
        ("focus", "groove") => &[Move::Disco, Move::Step, Move::Pump],
        ("deep", "groove") => &[Move::Pump, Move::Roof, Move::Disco, Move::Bounce],
        ("study", "beat") => &[Move::Nod, Move::Step, Move::Bounce],
        _ => return Move::Sway,
    };
    if st.groove < 0.6 {
        // The drums are still fading in: keep it small.
        return set[0].min_energy();
    }
    let phrase = (b.pos / PHRASE).floor() as i64;
    set[phrase.rem_euclid(set.len() as i64) as usize]
}

impl Move {
    fn min_energy(self) -> Move {
        if self == Move::Nod {
            Move::Nod
        } else {
            Move::Bounce
        }
    }

    fn dances(self) -> bool {
        !matches!(self, Move::Idle | Move::Sway | Move::Wave)
    }
}

fn pose(m: Move, b: &Beat, t: f64) -> Pose {
    let mut p = Pose::REST;
    let dip = (b.kick < 0.3) as i32;
    let beat = b.pos.floor() as i64;
    let phase = b.pos - b.pos.floor();
    match m {
        Move::Idle => {}
        Move::Wave => p.arms = (Arm::Down, Arm::Wave((t * 4.0).floor() as i64 % 2 == 0)),
        Move::Sway => {
            // Weight shifts every two beats, through the middle, nodding on any kick.
            p.lean = [0, 1, 0, -1][(b.pos / 2.0).floor().rem_euclid(4.0) as usize];
            p.nod = (b.kick < 0.3) as i32;
            p.eyes_closed = true;
        }
        Move::Bounce => {
            p.y = dip;
            p.arms = (Arm::Hip, Arm::Hip);
        }
        Move::Pump => {
            p.y = dip;
            p.arms = (Arm::Hip, Arm::Hip);
            if b.snap < 0.45 {
                if b.snap_n % 2 == 0 {
                    p.arms.1 = Arm::Up;
                } else {
                    p.arms.0 = Arm::Up;
                }
            }
        }
        Move::Step => {
            // Step right, touch, step left, touch; clap on the touches.
            let q = beat.rem_euclid(4);
            p.y = dip;
            p.x = if q < 2 { 1 } else { -1 };
            p.feet = match q {
                0 => (1, 0),
                2 => (0, 1),
                _ => (0, 0),
            };
            p.arms = if q % 2 == 1 && phase < 0.4 { (Arm::Clap, Arm::Clap) } else { (Arm::Down, Arm::Down) };
        }
        Move::Roof => {
            p.y = dip;
            p.arms = if b.kick < 0.3 { (Arm::Up, Arm::Up) } else { (Arm::Roof, Arm::Roof) };
        }
        Move::Disco => {
            p.y = dip;
            if (b.pos / 2.0).floor().rem_euclid(2.0) < 1.0 {
                p.arms = (Arm::Hip, Arm::Point);
                p.lean = 1;
                p.feet = (1, 0);
            } else {
                p.arms = (Arm::Hip, Arm::Cross);
                p.lean = -1;
                p.feet = (0, 1);
            }
        }
        Move::Nod => {
            p.y = dip;
            p.nod = (phase < 0.35) as i32;
            p.arms = (Arm::Hip, Arm::Mic);
        }
    }
    if m.dances() && b.pos.rem_euclid(PHRASE) >= PHRASE - 1.0 {
        // The last beat of a phrase: arms up, eyes closed, singing.
        p.arms = (Arm::Up, Arm::Up);
        p.eyes_closed = true;
        p.mouth_open = true;
    }
    p
}

struct Canvas {
    px: [[Ink; W]; H],
    /// Pixels that fell outside; tests check no pose loses any.
    clipped: usize,
}

impl Canvas {
    fn new() -> Self {
        Canvas { px: [[Ink::Clear; W]; H], clipped: 0 }
    }

    fn get(&self, x: i32, y: i32) -> Ink {
        if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) {
            self.px[y as usize][x as usize]
        } else {
            Ink::Clear
        }
    }

    fn put(&mut self, x: i32, y: i32, ink: Ink) {
        if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) {
            self.px[y as usize][x as usize] = ink;
        } else {
            self.clipped += 1;
        }
    }

    fn sprite(&mut self, x: i32, y: i32, rows: &[&str]) {
        for (dy, row) in rows.iter().enumerate() {
            for (dx, ch) in row.chars().enumerate() {
                if ch != '.' {
                    self.put(x + dx as i32, y + dy as i32, ink(ch));
                }
            }
        }
    }

    fn leg(&mut self, hip: (i32, i32), foot: (i32, i32), side: i32) {
        // Feet stay planted, so a lower body bends the knees outward.
        let bend = (LEG - (foot.1 - hip.1)).clamp(0, 2);
        let knee = ((hip.0 + foot.0) / 2 + side * bend, (hip.1 + foot.1) / 2);
        for (a, b) in [(hip, knee), (knee, foot)] {
            for (x, y) in line(a, b) {
                let ink = if y == foot.1 {
                    Ink::Sole
                } else if y >= foot.1 - 3 {
                    Ink::Boot
                } else {
                    Ink::Skin
                };
                self.put(x, y, ink);
                self.put(x + 1, y, ink);
            }
        }
        // Platform soles stick out a pixel.
        self.put(if side < 0 { foot.0 - 1 } else { foot.0 + 2 }, foot.1, Ink::Sole);
    }

    fn arm(&mut self, shoulder: (i32, i32), arm: Arm, side: i32) {
        let ((ex, ey), (hx, hy)) = arm.joints();
        let elbow = (shoulder.0 + side * ex, shoulder.1 + ey);
        let hand = (shoulder.0 + side * hx, shoulder.1 + hy);
        for (x, y) in line(shoulder, elbow).into_iter().chain(line(elbow, hand)) {
            // A darker sleeve where it crosses the dress, so the arm still reads.
            let over = matches!(self.get(x, y), Ink::Dress | Ink::Fold | Ink::Holo(_));
            self.put(x, y, if over { Ink::Fold } else { Ink::Dress });
        }
        self.put(hand.0, hand.1, Ink::Skin);
    }
}

/// Pixels on the segment from `a` to `b` (Bresenham).
fn line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (mut x, mut y) = a;
    let dx = (b.0 - x).abs();
    let dy = -(b.1 - y).abs();
    let sx = if x < b.0 { 1 } else { -1 };
    let sy = if y < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut out = vec![(x, y)];
    while (x, y) != b {
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        out.push((x, y));
    }
    out
}

fn draw(p: &Pose) -> Canvas {
    let mut c = Canvas::new();
    let bx = CX + p.x;
    let ux = bx + p.lean;
    let top = 1 + p.y;
    let neck = top + 8;
    let chest = neck + 1;
    let hips = chest + 4;

    c.leg((bx - 2, hips), (bx - 2 - p.feet.0, FLOOR - p.lift.0), -1);
    c.leg((bx + 1, hips), (bx + 1 + p.feet.1, FLOOR - p.lift.1), 1);
    c.sprite(bx - 3, hips, &SKIRT);
    c.sprite(ux - 2, chest, &BODICE);
    c.sprite(ux - 1, neck, &["sSs"]);

    let hx = ux - 5;
    let hy = top + p.nod;
    c.sprite(hx, hy, &HEAD);
    if p.swing != 0 {
        // The bob's ends trail behind a sideways move.
        let x = if p.swing > 0 { hx - 1 } else { hx + HEAD[0].len() as i32 };
        c.put(x, hy + 5, Ink::Hair);
        c.put(x, hy + 6, Ink::Hair);
    }
    for row in c.px.iter_mut() {
        for px in row.iter_mut() {
            *px = match *px {
                Ink::Eye if p.eyes_closed => Ink::Lid,
                Ink::Lip if p.mouth_open => Ink::Mouth,
                other => other,
            };
        }
    }

    c.arm((ux - SHOULDER, chest), p.arms.0, -1);
    c.arm((ux + SHOULDER, chest), p.arms.1, 1);
    c
}

/// Her pose and colors for what's being heard: `beats` is the beat position now, `t` seconds
/// since the panel opened (the clock for blinking and for swaying without a beat).
fn figure(st: &Status, beats: f64, t: f64) -> (Canvas, Look) {
    let pos = if st.bpm > 0.0 { beats } else { t };
    let b = Beat::new(st, pos);
    let m = choose(st, &b);
    let mut p = pose(m, &b, t);
    // Hair trails behind sideways moves for a moment.
    let earlier = Beat::new(st, pos - 0.15);
    let q = pose(choose(st, &earlier), &earlier, t - 0.1);
    p.swing = (p.x + p.lean - q.x - q.lean).signum();
    if blinking(t) {
        p.eyes_closed = true;
    }
    let look = Look { shimmer: pos.floor().rem_euclid(HOLO.len() as f64) as usize, glint: b.kick < 0.12 };
    (draw(&p), look)
}

/// A blink of about 140 ms at a random moment in every 4 seconds.
fn blinking(t: f64) -> bool {
    let k = (t / 4.0).floor();
    let mut h = (k as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 29;
    let at = k * 4.0 + 0.5 + 3.0 * (h % 1000) as f64 / 1000.0;
    (at..at + 0.14).contains(&t)
}

pub struct Dancer {
    truecolor: bool,
}

impl Dancer {
    pub fn new(truecolor: bool) -> Self {
        Dancer { truecolor }
    }

    /// Her figure as `ROWS` lines of `W` cells. `st` is the snapshot being heard, `beats` the
    /// beat position now and `t` seconds since the panel opened.
    pub fn frame(&self, st: &Status, beats: f64, t: f64) -> Vec<String> {
        let (c, look) = figure(st, beats, t);
        (0..ROWS).map(|row| self.line(&c, &look, row)).collect()
    }

    fn line(&self, c: &Canvas, look: &Look, row: usize) -> String {
        let mut s = String::new();
        let (mut fg, mut bg): (Option<Rgb>, Option<Rgb>) = (None, None);
        for x in 0..W {
            let top = look.rgb(c.px[2 * row][x]);
            let bottom = look.rgb(c.px[2 * row + 1][x]);
            let (ch, want_fg, want_bg) = match (top, bottom) {
                (None, None) => (' ', fg, None),
                (Some(t), None) => ('▀', Some(t), None),
                (None, Some(b)) => ('▄', Some(b), None),
                (Some(t), Some(b)) if t == b => ('█', Some(t), None),
                (Some(t), Some(b)) => ('▀', Some(t), Some(b)),
            };
            if want_bg != bg {
                match want_bg {
                    Some(c) => s += &self.sgr(48, c),
                    None => s += "\x1b[49m",
                }
                bg = want_bg;
            }
            if want_fg != fg {
                if let Some(c) = want_fg {
                    s += &self.sgr(38, c);
                }
                fg = want_fg;
            }
            s.push(ch);
        }
        s += "\x1b[0m";
        s
    }

    fn sgr(&self, layer: u8, (r, g, b): Rgb) -> String {
        if self.truecolor {
            format!("\x1b[{layer};2;{r};{g};{b}m")
        } else {
            format!("\x1b[{layer};5;{}m", xterm256((r, g, b)))
        }
    }
}

/// Nearest color in the xterm 256-color palette (6×6×6 cube or gray ramp).
fn xterm256((r, g, b): Rgb) -> u8 {
    const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| (0..6).min_by_key(|&i| (LEVELS[i] - v as i32).abs()).unwrap();
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let dist = |c: (i32, i32, i32)| (c.0 - r as i32).pow(2) + (c.1 - g as i32).pow(2) + (c.2 - b as i32).pow(2);
    let cube = dist((LEVELS[ri], LEVELS[gi], LEVELS[bi]));
    let avg = (r as i32 + g as i32 + b as i32) / 3;
    let gray = ((avg - 8) / 10).clamp(0, 23);
    let gv = 8 + 10 * gray;
    if dist((gv, gv, gv)) < cube {
        232 + gray as u8
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}

/// Whether the terminal takes 24-bit color.
pub fn truecolor() -> bool {
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    colorterm.contains("truecolor") || colorterm.contains("24bit")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Move; 9] =
        [Move::Idle, Move::Sway, Move::Bounce, Move::Pump, Move::Step, Move::Roof, Move::Disco, Move::Nod, Move::Wave];

    fn focus() -> Status {
        Status {
            mode: "focus",
            section: "pulse",
            bpm: 120.0,
            kick: 0x1111,
            snap: 1 << 4 | 1 << 12,
            groove: 1.0,
            ..Status::default()
        }
    }

    #[test]
    fn last_hit_counts_back_through_the_bar() {
        let two_four = 1 << 4 | 1 << 12;
        assert_eq!(last_hit(two_four, 13.0), Some((0.25, 1)));
        assert_eq!(last_hit(two_four, 17.0), Some((1.25, 1)));
        assert_eq!(last_hit(two_four, 20.0), Some((0.0, 2)));
        assert_eq!(last_hit(0, 3.0), None);
        // Before the first bar, the pattern repeats backwards.
        assert_eq!(last_hit(1, -1.0), Some((3.75, -1)));
    }

    #[test]
    fn she_dips_on_the_kick() {
        let st = focus();
        let on = Beat::new(&st, 4.05);
        let between = Beat::new(&st, 4.5);
        assert_eq!(pose(Move::Bounce, &on, 0.0).y, 1);
        assert_eq!(pose(Move::Bounce, &between, 0.0).y, 0);
    }

    #[test]
    fn snaps_alternate_arms() {
        let st = focus();
        let first = pose(Move::Pump, &Beat::new(&st, 1.1), 0.0);
        let second = pose(Move::Pump, &Beat::new(&st, 3.1), 0.0);
        assert_eq!(first.arms, (Arm::Hip, Arm::Up));
        assert_eq!(second.arms, (Arm::Up, Arm::Hip));
    }

    #[test]
    fn moves_follow_the_music() {
        let st = focus();
        let b = Beat::new(&st, 10.0);
        assert!(choose(&st, &b).dances());
        assert_eq!(choose(&Status { paused: true, ..st }, &b), Move::Idle);
        assert_eq!(choose(&Status { ending: true, ..st }, &b), Move::Wave);
        assert_eq!(choose(&Status { ending: true, paused: true, ..st }, &b), Move::Wave);
        assert_eq!(choose(&Status { ending: true, finished: true, ..st }, &b), Move::Idle);
        assert_eq!(choose(&Status { groove: 0.0, ..st }, &b), Move::Sway);
        assert_eq!(choose(&Status { section: "float", ..st }, &b), Move::Sway);
        assert_eq!(choose(&Status { groove: 0.4, ..st }, &b), Move::Bounce);
        // A new move every 8 bars.
        let moves: Vec<Move> = (0..3).map(|p| choose(&st, &Beat::new(&st, 32.0 * p as f64 + 1.0))).collect();
        assert_eq!(moves, [Move::Bounce, Move::Pump, Move::Step]);
    }

    #[test]
    fn every_pose_stays_on_the_canvas() {
        let st = focus();
        for m in ALL {
            for i in 0..128 {
                let b = Beat::new(&st, i as f64 * 0.125);
                let mut p = pose(m, &b, i as f64 * 0.1);
                for swing in [-1, 0, 1] {
                    p.swing = swing;
                    assert_eq!(draw(&p).clipped, 0, "{m:?} at beat {}", b.pos);
                }
            }
        }
    }

    #[test]
    fn frames_are_full_width_lines() {
        let st = focus();
        for truecolor in [true, false] {
            let lines = Dancer::new(truecolor).frame(&st, 5.0, 1.0);
            assert_eq!(lines.len(), ROWS);
            for l in &lines {
                let mut visible = 0;
                let mut esc = false;
                for ch in l.chars() {
                    match (esc, ch) {
                        (false, '\x1b') => esc = true,
                        (true, 'm') => esc = false,
                        (true, _) => {}
                        (false, _) => visible += 1,
                    }
                }
                assert_eq!(visible, W);
            }
        }
    }

    #[test]
    fn xterm256_maps_known_colors() {
        assert_eq!(xterm256((0, 0, 0)), 16);
        assert_eq!(xterm256((255, 255, 255)), 231);
        assert_eq!(xterm256((217, 119, 87)), 173);
        assert_eq!(xterm256((128, 128, 128)), 244);
    }

    /// `DANCER_SHEET=out.ppm cargo test sheet -- --ignored` draws every move through two bars,
    /// one row per move, for eyeballing the pixel art.
    #[test]
    #[ignore]
    fn sheet() {
        let path = std::env::var("DANCER_SHEET").expect("set DANCER_SHEET to an output .ppm path");
        let bg: Rgb = std::env::var("DANCER_BG").ok().map_or((24, 24, 28), |v| {
            let n = u32::from_str_radix(v.trim_start_matches('#'), 16).unwrap();
            ((n >> 16) as u8, (n >> 8) as u8, n as u8)
        });
        let st = focus();
        let (scale, gap, cols) = (6usize, 4usize, 17usize);
        let (fw, fh) = (W * scale + gap, H * scale + gap);
        let (iw, ih) = (fw * cols, fh * ALL.len());
        let mut img = vec![bg; iw * ih];
        for (r, &m) in ALL.iter().enumerate() {
            for col in 0..cols {
                // Two bars in eighths, then the phrase's last beat.
                let pos = if col == cols - 1 { PHRASE - 0.9 } else { col as f64 * 0.5 + 0.02 };
                let b = Beat::new(&st, pos);
                let p = pose(m, &b, col as f64 * 0.13);
                let c = draw(&p);
                let look = Look { shimmer: pos.floor() as usize % 3, glint: b.kick < 0.12 };
                for y in 0..H {
                    for x in 0..W {
                        if let Some(rgb) = look.rgb(c.px[y][x]) {
                            for sy in 0..scale {
                                for sx in 0..scale {
                                    img[(r * fh + y * scale + sy) * iw + col * fw + x * scale + sx] = rgb;
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut out = format!("P6 {iw} {ih} 255\n").into_bytes();
        for (r, g, b) in img {
            out.extend([r, g, b]);
        }
        std::fs::write(path, out).unwrap();
    }
}
