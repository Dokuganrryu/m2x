# Arm Cortex-M33 core debug registers used by the cycle counter

From the Armv8-M Architecture Reference Manual and CMSIS `core_cm33.h`.

| Address | Register | Bits | Use | Source |
|---|---|---|---|---|
| `0xE000_EDFC` | `DEMCR` | 24 `TRCENA` | enables the DWT | CMSIS `DCB_BASE 0xE000EDF0` + `0x00C` |
| `0xE000_1000` | `DWT_CTRL` | 0 `CYCCNTENA`, 25 `NOCYCCNT` (1 when there is no counter) | DWT control | CMSIS |
| `0xE000_1004` | `DWT_CYCCNT` | | cycle counter | CMSIS |
