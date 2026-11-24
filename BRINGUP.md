# Bring-up

How the firmware went from nothing to a 150 MHz, 14-band audio image on a board with no spare unit and no debugger,
and how to extend it the same way. The rule throughout: **nothing reaches the hardware until the emulator has
booted it against a model of the board and every check passes.**

## The ladder

Each rung is a Cargo feature in `crates/board/Cargo.toml`. Each one turns on exactly one new capability on top of
the rung below, so when something fails on the board, there is only one new thing to suspect.

| Rung | Adds | Why at this point |
|---|---|---|
| `milestone` | Quiesce, clock input, USB attach, analog parts held off, power rails, the screen | Proves the image boots and can talk at all, with every audio part held off |
| `milestone2` | Reset at image offset 0x200, no MPU fence, the clock it was handed | Nothing reads flash, so the fence guards nothing and only adds an untested step before USB |
| `milestone4` | The hand-over from a running previous image: stop its DMA, timer, interrupts and USB first | The loader starts the image over live peripherals, so the quiesce has to be proven before anything else runs |
| `milestone7` | Every boot stage, with all outputs, DAC volume and phantom power filtered to off | Exercises the whole sequence, including converter set-up, with nothing audible or powered |
| `sound1` | Audio plays: the output pin and DAC volume follow the state machines, 5 EQ bands | First sound, with phantom power still held off by the executor |
| `sound2` | 14-band presets with zero-clip trims | The full EQ, sized to fit (the board crate builds at `opt-level = "s"`) |
| `sound2-ui` | The meter screen: preset pill, ballistics, dirty-region scheduling | The UI arrives once the audio underneath it is stable |
| `sound3` | 150 MHz with voltage scaling, all 14 bands at 88.2 and 96 kHz, OBS-style meters | The riskiest change, a clock and voltage raise, comes last |

### USB and the screen come up before the clocks and converters

With no debugger, the only ways to see a fault are the USB console and the LCD. So the boot brings them up first, on
the clock it was handed, and raises PLL1 only after `UsbAttach` and `StartLcd`. A core clock that never locks then
leaves USB, the console and the screen alive to report the stage and the fault code (`crates/device/src/boot.rs`).
The boot also holds for 500 ms until the screen is lit (`SCREEN_LIT_BEFORE_THE_CLOCKS`), so a later stage that hangs
the core leaves its stage number on the glass. An eight-byte breadcrumb in a no-init section survives a warm reset,
so the next boot can report where the last one died.

## The emulator and board model

`crates/xtask/board-model/board_model.py` boots the real ELF on [Unicorn](https://www.unicorn-engine.org/) as a
Cortex-M33 in Thumb mode. Around the core it models the parts of the board the firmware touches: clocks and PLL
lock bits, the converter I2C bus with its DAC, ADC and expander, the SPI panel bus, DMA0, the timer, the USB device
controller, the PMC, the MPU and the watchdog. The model reads a plan on stdin (`load`, `entry`, `adc 13`,
`dac silent-after 4`, `pll-lock never`, `run 3000`, …) and prints a trace of every bus transaction and register write.
It judges nothing.

Three `xtask` commands drive it:

| Command | Proves |
|---|---|
| `cargo xtask board-run` | 32 scenarios in `crates/xtask/src/board_run/scenario.rs`: a clean boot, every peripheral that never answers, PLLs that never lock, a hung I2C transfer, a bus fault, a smaller MPU, an inherited watchdog, a ten-second soak, a host rate change. `gate.rs` checks each trace: the ending, the fault record, the MPU fence, pin holds, silence after a fault and the boot order |
| `cargo xtask budget` | The worst-case audio tick and USB interrupt in instructions and cycles, at each frame count and clock, against the cycle budget |
| `cargo xtask hand-over` | Each image, started over a modelled still-running previous image, writes no byte of a live DMA buffer before stopping that channel, and reaches `main` with everything quiesced |

Every modelled behaviour is listed in the `BEHAVIORS` table near the top of `board_model.py`, with its source: a
data sheet, the factory configuration, a measurement on hardware, or "guess". A guess is visible, not hidden.

### Adding a check before touching hardware

1. **Model the behaviour.** Add a plan verb to `board_model.py`, document it in the module docstring, and add an
   `BEHAVIORS` row saying where the behaviour comes from.
2. **Write the scenario.** Add a `Scenario` to `SCENARIOS` in `crates/xtask/src/board_run/scenario.rs`, with the
   plan lines and the `Expectation` the trace must meet.
3. **Gate it.** If no existing check covers the new behaviour, add one to `crates/xtask/src/board_run/gate.rs` and a
   test for the gate itself in `crates/xtask/tests/board_run_gate.rs`.
4. **Run it.** `cargo xtask board-run` must pass on every image before the change goes near a device.

## Lessons from the bare metal

- **No FPU in Non-secure state.** The image runs Non-secure, where a VFP instruction is a NOCP UsageFault unless the
  Secure side grants CP10 and CP11. The target uses `-fpregs`, so LLVM cannot emit one. `scan-image` rejects any
  `v`-prefixed mnemonic in the linked image, and `f64` design code uses software routines off the hot path.
- **Fixed-point biquads with headroom.** Samples run in Q2.29 with 64-bit accumulators, so a band clamps only past
  four times full scale. The tests prove that every built-in preset keeps each prefix's L1 norm under 4, so no
  full-scale signal reaches a clamp and the cascade stays exactly linear. The trim is folded into the first band's
  feedforward coefficients, so it costs no cycles.
- **Power the converters in order.** The ADC enable pin is held low for 5 ms, then both converter enables go high and 100 ms pass before
  the first converter bus transfer. Before that, every DAC write goes unanswered. The output-connect pin goes high
  last, after the DAC volume is at full attenuation.
- **Raise the voltage before the clock.** The 150 MHz raise programs the DCDC and LDO trims for the part's process
  corner, read from its NMPA words, before PLL1 relocks. Fields are only ever raised, never lowered. A single-word
  flash read is allowed only at or below 100 MHz, so the factory page and NMPA words are copied to RAM once at boot,
  and nothing reads flash afterwards. If the raise fails, the core stays at 100 MHz and runs a 7-band fallback.
- **Count SysTick wraps from the live reload.** A fixed 100 MHz period against the 150 MHz reload once turned one
  wrap into billions of cycles. The cycle clock now reads the period from the live reload, so a bounded wait lasts
  its wall time at any clock.
- **Give the audio DMA priority.** The audio channels run at DMA priority 0 and the screen channel lower, so audio
  always wins arbitration. The screen's descriptor chain is capped at 24 descriptors.
- **Draw in the V-blank.** The screen tracks dirty regions and sends them in one DMA chain started on the panel's
  tearing-effect edge, so a change lands inside one V-blank with no tearing. Pixels for a batch never pass one
  2,560-word arena.
- **Order the USB start-up.** PORTMODE, the PHY writes and the PHY PLL lock wait come before the first device
  controller access. In the other order the device never arrives on the bus.
