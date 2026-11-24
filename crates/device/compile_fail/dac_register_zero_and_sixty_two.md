# ES9026 registers 0 and 62 cannot be written in a normal build

A config write to a register the factory init uses compiles.

```rust
use device::command::Command;
use device::dac::{ConfigRegister, DacWrite};
let _ = Command::WriteDac(DacWrite::Config(ConfigRegister::Register2));
```

Register 0 has no config variant.

```compile_fail,E0599
use device::command::Command;
use device::dac::{ConfigRegister, DacWrite};
let _ = Command::WriteDac(DacWrite::Config(ConfigRegister::Register0));
```

Register 62 has no config variant.

```compile_fail,E0599
use device::command::Command;
use device::dac::{ConfigRegister, DacWrite};
let _ = Command::WriteDac(DacWrite::Config(ConfigRegister::Register62));
```

No volume, trim or status variant names register 0 or 62 either.

```compile_fail,E0599
let _ = device::dac::VolumeRegister::Register0;
```

```compile_fail,E0599
let _ = device::dac::MasterTrimByte::Register62;
```

```compile_fail,E0599
let _ = device::dac::StatusRegister::Register62;
```

A register cannot be made from a number.

```compile_fail,E0277
let _ = device::dac::ConfigRegister::try_from(0u8);
```

```compile_fail,E0599
let _ = device::dac::DacRegister::from_number(62);
```

A transaction cannot carry a bare register number.

```compile_fail,E0308
let _ = device::dac::DacTransaction { register: 0u8, value: 0x00 };
```

A write cannot carry a bare value for a config register.

```compile_fail,E0061
use device::dac::{ConfigRegister, DacWrite};
let _ = DacWrite::Config(ConfigRegister::Register2, 0x00);
```

Master trim cannot be built louder than -1.00 dB.

```compile_fail,E0423
let _ = device::dac::MasterTrim(0x7FFF_FFFF);
```

```rust
assert!(device::dac::MasterTrim::new(0x7FFF_FFFF).is_none());
```
