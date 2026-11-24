# Disclaimer

## No warranty

This software is provided "as is", without warranty of any kind, as stated in [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE). Loading third-party firmware onto a device may void its warranty, and a failed or
wrong load may leave the device unable to boot. You alone are responsible for what you load onto your hardware.

## Trademarks and affiliation

Developed and tested on the NXP LPC55S28 hardware revision of the MOTU M2. Independent open-source project; not
affiliated with or endorsed by MOTU, NXP, ESS Technology, or AKM. MOTU and M2 are trademarks of their owner. They
appear here, and as "M2" in M2X Studio's **Connect M2** button, only to say which hardware the firmware
runs on.

OBS and OBS Studio are trademarks of the OBS Project. The `meter obs` profile is named for the meter scale it
imitates, and implies no affiliation. Equalizer APO is named only to identify the text file format M2X Studio
imports. All other names belong to their respective owners.

## USB identity

The firmware enumerates as `1209:0001`, a test pair from the pid.codes open registry. It never presents the original
manufacturer's USB vendor or product ID.

## Provenance

The code in this repository was written for this project. It contains no vendor firmware, binaries, tools or code.

What it takes from elsewhere:

- **Datasheet facts.** Register addresses, bit fields and timing come from NXP's LPC55S2x data sheet and UM11126
  user manual, the NXP SVD register description (see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)), Arm's
  Armv8-M documentation and CMSIS, and the converter and panel data sheets. Comments cite them by table or section.
- **The board's factory configuration.** Several values are chosen to match how the board is configured when it
  ships, so the hardware behaves as its designers intended. These include pin functions and pad words, clock dividers,
  DMA channel assignments, interrupt priorities, the converter and panel set-up sequences, and the knob and meter
  laws. They were worked out during development by studying the board and its original firmware for
  interoperability. Identifiers and comments that say "factory" mark these values. The factory page is the board's
  own serial and calibration record, which the firmware reads and never writes.
- **Measurements on hardware.** Some timings and orderings were set from what the hardware did during bring-up.
  Comments that say "on hardware" mark them.

This project makes no claim to be a clean-room implementation. Before you redistribute images, or build a product on
this code, get your own legal advice.
