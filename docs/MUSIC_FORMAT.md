# Music format — songs as `const` data

Every song in the game is plain Rust data in `src/music.rs` (host-compiled,
unit-tested: the format, the theory helpers that read it, and the songs
themselves). `src/audio.rs` (wasm-only) is the synthesizer that plays it —
Web Audio through `web-sys`, no audio files, no crates. This document is the
authoring reference; the module docs are the API reference.

The `?viz` → MUSICS tab is the tracker: click a song, watch the section
strip and the pattern grid (ties draw as half-height bars, cells shade by
velocity), mute / solo rows, click a column to seek, read the song's global
settings on the info line and each lane's instrument under the grid.

## Overview

```text
SongSpec ─ name, root (Hz), scale (mode), bpm, steps_per_beat
         ├ voices: [Voice; 5]        the instruments of BASS LEAD PAD ARP KEYS
         ├ sections: &[Section]      the arrangement, played in order, looped
         ├ intensity, swing, humanize, sweep
         ├ sidechain: Sidechain      the kick-driven ducker
         └ echo: Echo                the shared tempo-synced delay line

Section ─ label
         ├ bass lead pad arp keys    note lanes: scale degrees | REST | HOLD
         ├ drums perc                percussion lanes: Drum per step
         ├ *_vel                     velocity lanes: 0..=9 per step (empty = full)
         └ *_chord                   voicing lanes: Chord per step (empty = default)
```

Seven channels: `BASS` `LEAD` `PAD` `ARP` `KEYS` (melodic, each with its own
`Voice`) and `DRUMS` `PERC` (percussion, same kit). Lane indices are the
constants of those names; `MELODIC` lists the five melodic ones.

## Notes: scale degrees, rests, ties

Melodic lanes are written as **scale degrees**: `0` is the root, `1` the
next note of the mode up, `7` an octave up (for a 7-note mode), negative
degrees go below the root. A song is therefore in key by construction, and
changing its `scale` re-spells every line.

```rust
bass: &[0, REST, REST, REST, 3, REST, REST, REST],   // root, rest, 4th
```

* `REST` — silence for that step.
* `HOLD` — a **tie**: the previous note sustains through this step. A
  note's length is 1 + the `HOLD`s that follow it (wrapping around the
  looping lane, stopping at the next note). `0, HOLD, HOLD, HOLD` is one
  quarter note; `0, 0, 0, 0` is four retriggered sixteenths.

A held note's envelope is: attack → peak, **held at peak** for the tied
steps, then the lane's usual decay (the "pluck" an untied note is entirely
made of). Lane defaults for that decay tail, in steps: bass 1.9, lead 0.9,
pad 4.0, arp 0.7, keys 1.2 — a `Voice::with_env(attack, gate)` overrides
them.

**Legato.** A note that starts exactly where the lane's previous note ends
(no rest between, a different pitch) is legato; on a voice with
`with_glide(seconds)` it slides in from the previous pitch. Rests break the
legato; repeated pitches don't glide.

Lanes inside a section may differ in length — a 16-step bass loops under a
64-step lead. A section's length is its longest NOTE / drum lane; velocity
and chord lanes only decorate and loop on their own.

## Velocity lanes

`bass_vel` … `perc_vel`: one digit `0..=9` per step (`MAX_VEL` = 9), looping
like the notes; an empty lane plays everything at full. Velocity is linear
amplitude, read where a note starts. `0` skips the note. The retriggered
**side-chain pump**:

```rust
bass:     &[0, 0, 0, 0],
bass_vel: &[3, 6, 8, 9],   // ducked on the kick, swelling back before the next
```

## Chord (voicing) lanes

`bass_chord` … `keys_chord`: one `Chord` per step, looping; empty = the
lane's default (`Triad` for the pad, `Single` for everything else). A
voicing is a set of scale-degree offsets, so it stays in key: `Single`,
`Octave` (root + 8ve), `Power` (root, 5th, 8ve), `Triad`, `Sus2`, `Sus4`,
`Seventh`, `Add9`, `Inv1`, `Inv2` (inversions), `Open` (root, 5th, 10th).
Partials play at 1/√n of the lane level so a chord is about as loud as a
single note.

```rust
keys:       &[REST, REST, 7, REST],
keys_chord: &[Chord::Power],          // every stab a power chord
```

## The kit

Percussion lanes hold `Drum`s: `Silent`, `Kick`, `Hat`, `Snare`, `Clap`,
`OpenHat`, `Tom`, `Rim`, `Crash` — all synthesized, all pre-baked. Two lanes
(`drums`, `perc`) so a hat can ride over a kick and a clap can layer a
snare. A `Kick` on either lane drives the side-chain and stays exactly on
the grid (humanize never moves it).

## Voices (instruments)

One `Voice` per melodic lane, built with a constructor and refined with
`const fn` builders — a song's whole instrument definition is one
expression:

```rust
Voice::stack(Wave::Sawtooth, 0.0, 12.0, 0.85, 5)   // five saws, ±12 cents, wide
    .with_filter(700.0, 2600.0, 1.1, 0.0, 1.1)      // opens 700 → 2600 Hz over 1.1 s
    .with_reverb(0.45),
```

| field / builder | meaning |
| --- | --- |
| `wave` | `Sine` `Triangle` `Square` `Sawtooth`, or `Noise` (the degree is ignored; the envelope + filter shape it — risers, wind) |
| `pan` (`Voice::panned`) | stereo position −1 … 1 (a `StereoPannerNode` per lane) |
| `detune`, `unison`, `width` (`Voice::wide` = 2, `Voice::stack` = n) | a unison stack of 1–7 oscillators spread over ±detune cents and ±width around the pan; a wide voice bakes to a stereo buffer |
| `with_filter(cutoff, peak, attack, decay, q)` | a per-note LOWPASS envelope: from `cutoff` up to `peak` over `attack` (instant if 0), back down over `decay` (stays open if 0); `q` = resonance. `attack > 0` is the pad bloom, `decay > 0` the stab "wow" |
| `with_vibrato(rate, depth_cents, delay)` | a sine LFO on the pitch fading in after `delay` |
| `with_env(attack, gate)` | amplitude envelope override: attack seconds, decay tail in steps |
| `with_drive(0..1)` | lane-level tanh soft clip after the panner (chords intermodulate; quiet parts are lifted) |
| `with_echo(0..1)` / `with_reverb(0..1)` | send levels into the song's echo line / the hall |
| `with_sub(0..1)` | a sine an octave under the lowest partial (centred, unfiltered) |
| `with_glide(seconds)` | legato portamento (see above) |

## Song-level settings

| field | meaning |
| --- | --- |
| `root`, `scale` | tonic in Hz + mode (`MINOR`, `DORIAN`, `HARMONIC_MINOR`, `PHRYGIAN`, `PHRYGIAN_DOMINANT`, `LOCRIAN`, or any `&[semitone offsets]`) |
| `bpm`, `steps_per_beat` | tempo and grid (4 = sixteenths) |
| `intensity` | overall level / punch (~0.5 lounge … 1.2 boss); also lowers the bus sweep's peak |
| `swing` | 0 straight … 1 full triplet: every odd sixteenth is delayed by up to a third of a step |
| `humanize` | seconds (≤ 0.02): every note but the kicks lands up to this early / late |
| `sweep` | depth of the bus lowpass's once-per-bar wah: 1 = closes to 420 Hz at the bar lines, 0 = stays open |
| `sidechain` | `Sidechain::new(depth, release_beats)` or `Sidechain::OFF`: every kick drops the melodic lanes to `1 − depth` in 4 ms, exponential recovery over `release_beats` |
| `echo` | `Echo::new(steps, feedback, tone_hz)` (`Echo::DOTTED` = dotted eighths): one shared delay line the voices' `echo` sends feed, repeats fed back through a darkening lowpass |

## Arrangement

`sections` is an ordered list; a section can appear several times (that is
how a refrain comes back) at zero cost. The scheduler plays them back to
back and loops the whole list. Author a section with `..Section::EMPTY` so
unused lanes default to empty:

```rust
const VERSE: Section = Section {
    label: "verse",
    bass: &[...], bass_vel: PUMP,
    lead: &[...],
    pad: PAD_BARS, pad_chord: &[Chord::Seventh],
    drums: BEAT, perc: RIDE, perc_vel: RIDE_VEL,
    ..Section::EMPTY
};
```

Add the finished `SongSpec` to `SONGS` (ascending darkness) and, if it
belongs to the tower, to `song_for_floor`.

## What the engine does with it

* **Look-ahead scheduling** on the audio clock (`LOOKAHEAD` 0.15 s); swing
  and humanize are applied per note at schedule time.
* **Pre-baking.** Every distinct voice the song can play — a `MusicKey`:
  `Note { lane, degree, len, chord, from }` or `Drum(kind)`, enumerated by
  `music_keys` (≤ 96 per song, tested) — is rendered once through an
  `OfflineAudioContext` by the same builders the live path uses, at its
  exact pitch (stereo for a wide voice), and then fired as one
  `AudioBufferSourceNode` per note. Velocity is a play-time `GainNode` only
  on notes below full; the glide origin enters the key only on gliding
  voices. Unbaked notes fall back to live synthesis; the queue is combat
  SFX → the current song → rare SFX, and the loading screen waits for it.
* **The bus.** Per melodic lane: panner → drive shaper → (echo / hall
  sends) → side-chain ducker → music bus → lowpass (swept per bar by
  `sweep`) → limiter → out. Drums go straight into the bus (never ducked,
  never driven).

## Checks

`cargo test --lib music` runs the format's native tests: ties, legato,
velocity and chord lanes, tracker cells, kit enumeration, every song's
well-formedness (ranges of every knob, no orphan `HOLD`s, unique names) and
the bake-set invariant (every schedulable note has a key; ≤ 96 keys). Run
with `-- --nocapture` for the per-song voice counts.
