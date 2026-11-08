# soundel

Generative focus soundscapes in your terminal, modeled on Endel's Focus. Everything is
synthesized live on your machine: offline, endless, no account. No Endel audio or code is
included; their streams were only measured to calibrate the engine.

## Install

```sh
cargo install --path . --root ~/.local    # puts `soundel` in ~/.local/bin
```

## Use

```sh
soundel                        # Focus, until you press q
soundel focus --timer          # Pomodoro: 25 min focus, 5 min break, long break after 4
soundel focus --for 90m        # fades out after 90 minutes
soundel study                  # lo-fi beats
soundel deep                   # minimal techno
soundel noise --color brown    # deep, brown, pink, grey, white, blue, violet, green
soundel tones --freq 432       # pentatonic pad tuned to a frequency (default 528 Hz)
soundel binaural --beat 14     # binaural beat (default 40 Hz); headphones only
soundel relax                  # beatless; also what plays during timer breaks
soundel chime start            # cue sounds: start, end, break, done
soundel render focus --for 10m -o focus.wav
soundel focus --no-ui &        # background; stop with kill or fg + Ctrl-C
```

While playing:

| Key | Does |
|---|---|
| ← → | Fluid ↔ Structured (noise: darker ↔ brighter) |
| ↑ ↓ | Easy ↔ Intense (noise: still ↔ moving) |
| + − | volume |
| space | pause |
| n | next section |
| q | quit (fades out) |

Useful options: `-x`/`-y` set the tunepad (0–1), `--hour` pretends it's another time of day,
`--heart-rate 66` sets Focus to twice your resting heart rate, `--seed` reproduces a session,
`--work/--break/--long-break/--rounds` shape the timer. `soundel --help` lists everything.

## Claudia

[Claudia](https://claudia.gallery/), the pop singer from the music videos made with Claude, dances
next to the panel in her desktop-dancer look: black bob with one clay-orange streak, star clip,
holographic silver dress, white platform boots. She dips on every kick, answers the snaps with her
arms and changes moves every 8 bars: bounce, pump, step-touch, raise the roof, disco point, and a
lo-fi nod in `study`. With no beat she sways with her eyes closed. She stands still when paused and
waves when you quit.

She follows what reaches your ears, not what the engine is computing. Each status snapshot is
stamped with the moment its audio will be heard, including the latency your output device reports
(about 160 ms for AirPods), and the panel replays the snapshots on that schedule. If she still looks
early or late, nudge her with `--dancer-delay 40` (milliseconds; negative moves her earlier).
`--no-dancer` hides her, and she steps aside on its own in terminals narrower than 65 columns or
shorter than 16 rows.

## How Focus works

Measured from ~210 minutes of Endel's Focus renders (different hours, tunepad positions and
a full 90-minute session), then rebuilt as a generative engine:

- **Everything loops.** Inside a section the drum pattern is fixed: kick on every beat, a short,
  wide snap on 2 and 4 that alternates between two tunings bar to bar, hats on the offbeats.
  The bass repeats a one-bar figure, and the melody is a two-bar figure that recurs in 70–90%
  of bars, changing at most one note every 8 bars. Predictable parts fade into the background,
  which is what lets you focus. (The first version generated random notes; it measured fine but
  was hard to focus to.)
- **Blended, not catchy.** Melody notes rise only a few dB above the pads instead of popping out.
- **Key:** C major pentatonic for the whole session. The sub-bass root rotates through
  G, D, C and A, each held for 2–10 minutes.
- **Pulse:** a clean sine kick with a pitch drop (160 → 55 Hz) at 120 BPM, twice a 60 bpm
  resting heart rate. The beat never fully disappears; sparse sections keep a loose kick.
- **Getting into flow:** pads and the root start immediately; the beat fades in over ~40 s;
  melody stays softer for the first 3 minutes.
- **Holding flow:** a new section every 2–10 minutes on a bar line (pulse, arpeggios, float,
  groove), with busy and sparse sections alternating.
- **Long sessions:** after 45 minutes the beat goes half-time and the mix drops 1.5 dB.
- **Time of day:** daytime gets four-on-the-floor; evening, night and early morning get
  half-time and a darker tone.
- **Tunepad:** Fluid ↔ Structured sets how much rhythm there is (loose kick vs. steady kick,
  bass figure and 16th-note arpeggios). Easy ↔ Intense moves the spectrum between about
  −17 and −6 dB/octave and brings up the hats and snap.
- **Sound:** dark and warm (most energy at 60–600 Hz), wide stereo, around −19 dBFS RMS.

## Development

```sh
cargo test            # DSP, music theory, timer, sessions and every mode
cargo build --release
```
