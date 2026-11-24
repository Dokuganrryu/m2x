# M2X Studio

A single-file control app for a device running the m2x firmware (the `sound2-ui` build and later). It talks to the
firmware's CDC console over Web Serial. There is nothing to install, it runs only while the tab is open, and it
never touches the audio path: every action is a console command the firmware already accepts.

## Start it

- **No hardware:** open `index.html` in any Chromium browser. Before you connect, an emulated device answers in the
  firmware's own reply formats, so every panel works offline.
- **With a device:** on Windows, double-click `launch-m2x-studio.bat`. It serves this folder on
  `http://127.0.0.1:8765` with Python's built-in `http.server` and opens it. On other systems, run
  `python -m http.server 8765 --bind 127.0.0.1` here and open `http://127.0.0.1:8765/`. Click **Connect M2** and pick
  the device (USB `1209:0001`).

Chromium exposes `navigator.serial` only to secure contexts (`https://`, `http://localhost`,
`http://127.0.0.1`). On a `file://` page the app shows a banner and stays in demo mode.

## What it does

- **14-band EQ graph.** Drag a node to set frequency and gain, scroll over it for Q, right-click to cycle peak, low
  shelf and high shelf, and double-click to reset the gain. The bounds are the firmware's: 20 Hz to 20 kHz, ±12 dB,
  Q 0.3 to 10. The curve uses the same RBJ cookbook formulas as `crates/dsp/src/peq/cookbook.rs`, at 48 or 96 kHz.
- **Zero-clip trim.** The app finds the largest boost over 512 points from 20 Hz to 20 kHz and sets
  trim = −⌈peak × 10⌉ / 10, the rule the built-in presets use (−2.6 dB for EQ1, −4.2 dB for EQ2). A manual slider
  covers −18 to 0 dB.
- **Apply.** Sends `peq begin`, then one `peq N type Hz dB Q` line per band (`peq off N` for a disabled band), then
  `trim`, then `peq commit`. Every step must answer `ok` or `preset …`. On any refusal it stops before the commit,
  so a half-applied curve never plays.
- **Presets.** RAW, EQ1 and EQ2 send `preset flat`, `preset clarity` and `preset warm`, and the graph shows the
  firmware's band tables. Save your own curves in the browser, rename or delete them, and export or import a JSON
  backup.
- **APO text import.** Drop or paste a parametric EQ text file in the Equalizer APO format (`Preamp:` and
  `Filter: ON PK|LSC|HSC Fc … Gain … Q …` lines). Fourteen bands or fewer inside the bounds load as they are. More
  bands, or values out of bounds, go through a bounded Levenberg–Marquardt fit into 14 bands, with the max and RMS
  error shown and the original curve drawn dashed. **Sample APO .txt** loads a 16-band example to show the fit.
- **Screen and limiter.** `meter obs|studio`, `lcd motion on|off`, and `lim <ceiling dBFS> <release ms>` (ceiling
  −20.0 to 0.0, release 5 to 1000 ms; the firmware default is −1.0 dB and 50 ms).
- **Telemetry**, polled once a second while connected and paused while the tab is hidden: core clock, sample rate,
  bands and path, audio tick load against its cycle budget, buffer fill with summed underruns and overruns, I2C
  health, the five knobs, phantom and monitor state, and the core supply.
- **Console drawer.** Send any command and watch the raw TX/RX log.

## Protocol facts it relies on

From `crates/device/src/console/` and `crates/board/src/firmware/console.rs`:

- **Lines.** One command per line, at most 64 printable ASCII characters, ended by `\n` or `\r`. Replies end with
  `\r\n`. A set command answers `ok`; an error answers `refused: <reason>`.
- **`cycles`** prints 8 lines (tick and USB, 44.1 and 48 kHz families, bypass and DSP). The 48 kHz family covers
  96 kHz too.
- **`counters` resets the counts it reports**, so the app sums them. **`meter reset` is never polled**, because it
  would clear the peaks the LCD draws from.
- Commands that only newer builds have (`power nmpa` and `meter obs|studio` need `sound3`; `lcd motion` needs
  `sound2-ui`) answer `refused` on older ones. The app logs that and carries on.
