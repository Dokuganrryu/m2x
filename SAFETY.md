# Safety

This firmware drives real analog hardware: a DAC, an ADC, headphone and line outputs, and 48 V phantom power. A
mistake here can damage ears, speakers, microphones or the device itself. The rules below are what the code
enforces, and they are the bar for every change.

## What can go wrong

| Risk | Consequence | What prevents it |
|---|---|---|
| A flash erase or program from the image | The device may no longer boot, with no proven way back | The image runs from RAM and has no flash path. `cargo xtask scan-image` fails any build whose linked image names the flash controller, a ROM flash API entry or the boot-request register, and planted builds prove the scan catches each one |
| Writing the factory page | Loss of the board's serial number and calibration | The factory page is read through one accessor and never written; the scan rejects a read of it anywhere else |
| Phantom power raised by software | 48 V on a microphone that cannot take it | Phantom is off at boot and no console command can raise it. In the `sound1`, `sound2`, `sound2-ui` and `sound3` builds the executor holds it locked off: P1_4 stays low and the phantom bits never reach the preamp's analog word, while the front-panel buttons only toggle the 48V LEDs (`device::silent::phantom_held`). The `milestone7` build holds every output and phantom off. Only the default full build lets the two front-panel buttons raise phantom |
| A full-scale burst at a preset change | Hearing damage, tweeter damage | Coefficients change only at a block boundary with a crossfade; each preset's trim keeps every steady tone at or below 0 dBFS; the limiter sits last in the chain |
| A pop at power-up or at a rate change | Same as above | The outputs stay muted until the analog parts are configured, and every rate change runs a quiesce and un-mute sequence (`device::quiesce`, `device::rate_change`) |
| A stuck audio interrupt | Silence or a stuck tone | The tick has no division, panic, allocation, interrupt masking or FPU use; `cargo xtask budget` prices its worst case in instructions |

## Rules for contributors

- **No flash code, ever.** Do not add any path that erases, programs or reconfigures on-chip flash. `scan-image`
  will refuse it, and a reviewer will too.
- **Hardware stays behind `board`.** Logic belongs in a host-testable crate with a test. `board` only turns
  decisions into register writes.
- **Phantom power is a physical decision.** Never let software raise it, and keep the sound builds' hold in place.
- **Every new hot-path code path gets a budget run.** Run `cargo xtask budget` and include the before and after
  instruction counts in the pull request.
- **Stay on the right hardware.** The image targets the NXP LPC55S28 revision only. Other revisions use other parts.

## Loading an image

This repository builds and checks firmware images. It does not include a tool to load one onto a device, and no
instructions for doing so. The built-in update path of a device like this writes flash, and a failed or wrong write
may leave it unable to boot. If you do load an image onto your own hardware, the risk is yours alone: see
[DISCLAIMER.md](DISCLAIMER.md).

## Reporting a safety problem

Open an issue titled `safety:` with the build feature, the steps and what you observed. If the problem could harm
someone who follows the documentation, say so in the first line.
