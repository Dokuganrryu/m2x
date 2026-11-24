# Contributing

Thanks for your interest. Read [SAFETY.md](SAFETY.md) first: some of its rules are hard limits, and a pull request
that crosses them will not be merged.

## Before you open a pull request

```sh
cargo fmt --check
cargo test --workspace
cargo firmware-sound3 && cargo xtask scan-image
node tools/m2x-studio/test.mjs
```

The CI workflow runs the same commands (it is started by hand while the repository is private). A change on the audio path also needs `cargo xtask budget`, with the before and after
instruction counts in the description.

## Code rules

These rules are tests in `crates/source-rules`, so `cargo test` catches a break:

- **Comments.** None except single-line `// WHY:` comments that give a reason the code cannot show. Names and types
  carry the rest.
- **Crate graph.** Only `board` touches registers. `dsp` depends on nothing in the workspace, `stream` on `dsp`, and
  `device` on both.
- **Hot path.** Nothing reached from the audio tick or the USB interrupt may divide, use the FPU, mask interrupts,
  call indirectly, panic, format, allocate or call libm. `cargo xtask scan-image` checks the linked image.
- **No flash path.** See [SAFETY.md](SAFETY.md).

Beyond the rules: keep modules deep (a small interface over real work), prefer one obvious way to do a thing, and
make the type system refuse invalid states where it can, the way `device::usb::identity` refuses any vendor ID other
than the open one.

## Pull requests

- One change per pull request. A refactor and a behaviour change go in separate pull requests.
- Say what changed and why, how you tested it, and what you did not test.
- Hardware observations are welcome. Say which build you ran and exactly what you saw.

## License

By contributing, you agree that your contribution is dual-licensed under MIT or Apache-2.0, as the rest of the
project is, with no additional terms.
