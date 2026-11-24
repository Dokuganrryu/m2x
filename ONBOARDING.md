# Onboarding

From a fresh clone to your first merged change. Nothing here needs hardware.

## 1. Set up (10 minutes)

```sh
rustup toolchain install stable          # 1.85 or newer
rustup target add thumbv8m.main-none-eabi
rustup component add rustfmt llvm-tools  # the image checks call the sysroot's llvm-objdump and llvm-nm
python -m venv .venv
.venv/Scripts/pip install unicorn==2.1.4 # Linux and macOS: .venv/bin/pip
```

The emulator-backed checks (`cargo xtask budget`, `board-run`, `hand-over`, and the xtask tests that boot an image)
find Python through `M2X_PYTHON` or the `.venv` at the repository root. Node 18 or newer runs the M2X Studio test.

## 2. Prove your setup

```sh
cargo test --workspace           # all host tests
cargo firmware-sound3            # the newest image
cargo xtask scan-image           # the image checks, with planted builds
node tools/m2x-studio/test.mjs
```

All four should pass. If `scan-image` cannot find `llvm-objdump`, the `llvm-tools` component is missing.

## 3. Find your way around

Start from the behaviour you care about and follow it inward:

| You want to change | Start in | Its tests |
|---|---|---|
| How a sample is processed | `crates/dsp/src/pipeline.rs` | `crates/dsp/tests/`, `crates/golden/` |
| A preset curve | `crates/device/src/preset/tables.rs` | `crates/device/tests/preset*.rs` |
| A console command | `crates/device/src/console/command.rs` | `crates/device/tests/console*.rs` |
| A USB descriptor or request | `crates/device/src/usb/` | `crates/device/tests/usb_*.rs` |
| What the LCD shows | `crates/device/src/lcd/` | `crates/device/tests/lcd*.rs` |
| A register write | `crates/board/src/firmware/` | `crates/source-rules/tests/`, `cargo xtask board-run` |
| The browser app | `tools/m2x-studio/index.html` | `tools/m2x-studio/test.mjs` |

[ARCHITECTURE.md](ARCHITECTURE.md) explains the crate boundaries and the audio tick.

## 4. Make a first change

A good first change is a new EQ preset curve:

1. Add a 14-band table and a 7-band fallback to `crates/device/src/preset/tables.rs`.
2. Run `cargo test -p device --features sound3 --test preset_sound3`. The test prints the worst peak if your trim is
   wrong. Set the trim to −⌈peak × 10⌉ / 10.
3. Add the name to `Preset::named` and the console tests.
4. Render it with `cargo run -p device --features sound3 --example preset_impulse -- out` and look at the response.
5. Add it to `BUILTIN` in M2X Studio, and run `node tools/m2x-studio/test.mjs` to check the app agrees on the trim.

## 5. Habits that keep the tree green

- Write the failing test first. Every module has a matching file under its crate's `tests/`.
- Keep logic out of `board`. If a branch can be decided on the host, it belongs in `device`, `stream` or `dsp`.
- No comments except a single-line `// WHY:` that explains a non-obvious reason. `cargo test` enforces this.
- Run `cargo xtask budget` after any change on the audio path.
