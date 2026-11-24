# A buffer the controller owns cannot be touched by software

The control flow below compiles. It is the shape every failing case changes.

```
use device::usb::endpoint::{ByteCount, UsbRam};
let mut ram = UsbRam::new();
let (buffers, _) = ram.split();
let (armed, _) = buffers.serial_out.receive(ByteCount::new(512));
let received = armed.landed(ByteCount::new(500));
let first = received.bytes().next();
let _again = received.release();
let _ = first;
```

An armed OUT buffer has no byte access.

```compile_fail,E0599
use device::usb::endpoint::{ByteCount, UsbRam};
let mut ram = UsbRam::new();
let (buffers, _) = ram.split();
let (armed, _) = buffers.serial_out.receive(ByteCount::new(512));
let _ = armed.bytes();
```

An armed IN buffer cannot be refilled.

```compile_fail,E0599
use device::usb::endpoint::{ByteCount, UsbRam};
let mut ram = UsbRam::new();
let (buffers, _) = ram.split();
let (mut armed, _) = buffers.serial_in.send(ByteCount::new(4));
armed.fill_bytes([1, 2, 3, 4]);
```

Arming gives the buffer away, so the old handle is gone.

```compile_fail,E0382
use device::usb::endpoint::{ByteCount, UsbRam};
let mut ram = UsbRam::new();
let (mut buffers, _) = ram.split();
let _ = buffers.serial_in.send(ByteCount::new(4));
buffers.serial_in.fill_bytes([1, 2, 3, 4]);
```

Bytes read from a landed buffer cannot outlive the buffer going back to the controller.

```compile_fail,E0505
use device::usb::endpoint::{ByteCount, UsbRam};
let mut ram = UsbRam::new();
let (buffers, _) = ram.split();
let (armed, _) = buffers.serial_out.receive(ByteCount::new(512));
let received = armed.landed(ByteCount::new(500));
let mut bytes = received.bytes();
let again = received.release().receive(ByteCount::new(512));
let _ = bytes.next();
let _ = again;
```

No second set of buffers exists while the first is alive.

```compile_fail,E0499
use device::usb::endpoint::UsbRam;
let mut ram = UsbRam::new();
let (first, _) = ram.split();
let (second, _) = ram.split();
let _ = (first, second);
```

Software cannot make a buffer out of thin air.

```compile_fail,E0451
use device::usb::endpoint::{OutBuffer, OutEntry, UsbRam};
let ram = UsbRam::new();
let _ = OutBuffer { ram: &ram, entry: OutEntry::Serial };
```
