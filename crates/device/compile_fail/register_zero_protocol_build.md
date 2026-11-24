# The protocol build forms register 0 only through the session, with the factory value only

The session hands out the write.

```rust
use device::phantom::PhantomGrant;
use device::register_zero::{
    BoardRecord, ConfirmationNonce, Event, Output, State, TransmitBuffers, VolumeEvidence,
    register_zero_factory_bus_bytes, transition,
};
let nonce = ConfirmationNonce::from_boot_entropy(7);
let record = BoardRecord {
    volume: VolumeEvidence::FullyAttenuatedAndAckedThisBoot,
    transmit: TransmitBuffers::Zeroed,
    phantom: PhantomGrant::NONE,
};
let walk = [
    Event::ProtocolSigned,
    Event::SpareConfirmed,
    Event::OutputsUnplugged,
    Event::ReplayRequested(record),
    Event::NonceTyped(nonce, record),
];
let mut state = State::locked(nonce);
let mut bytes = None;
for event in walk {
    let (next, outputs) = transition(state, event);
    for output in outputs.as_slice() {
        if let Output::WriteRegisterZero(write) = output {
            bytes = Some(register_zero_factory_bus_bytes(*write));
        }
    }
    state = next;
}
assert_eq!(bytes, Some([0x00, 0x00]));
```

The write cannot be built outside the session.

```compile_fail,E0423
let _ = device::register_zero::RegisterZeroWrite(());
```

```compile_fail,E0599
let _ = device::register_zero::RegisterZeroWrite::new(0x00);
```

The write carries no value, so no other value can be sent.

```compile_fail,E0599
let _ = device::register_zero::RegisterZeroWrite::with_value(0x01);
```

```compile_fail,E0061
use device::register_zero::{register_zero_factory_bus_bytes, RegisterZeroWrite};
fn send(write: RegisterZeroWrite) -> [u8; 2] {
    register_zero_factory_bus_bytes(write, 0x01)
}
```

The armed state cannot be forged to skip the walk.

```compile_fail,E0451
use device::phantom::Milliseconds;
use device::register_zero::{Arming, ConfirmationNonce, State};
let nonce = ConfirmationNonce::from_boot_entropy(7);
let _ = State::NonceIssued(Arming { nonce, waited: Milliseconds(0) });
```

```compile_fail,E0451
use device::register_zero::{Replay, State};
let _ = State::Replaying(Replay { step: 0, wait: 0 });
```

Register 62 has no path in the protocol build either.

```compile_fail,E0599
let _ = device::register_zero::Output::WriteRegisterSixtyTwo;
```

```compile_fail,E0599
let _ = device::register_zero::OperatorStep::Write(62);
```

```compile_fail,E0599
let _ = device::dac::ConfigRegister::Register62;
```

```compile_fail,E0599
let _ = device::dac::ConfigRegister::Register0;
```
