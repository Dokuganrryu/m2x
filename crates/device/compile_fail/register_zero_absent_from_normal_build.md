# The normal build has no register-0 write and no protocol command

The register-0 session exists only in the protocol build.

```compile_fail,E0432
use device::register_zero::RegisterZeroWrite;
```

```compile_fail,E0432
use device::register_zero::transition;
```

The normal console has no protocol command.

```compile_fail,E0599
let _ = device::console::ConsoleCommand::RegisterZero;
```

The normal command type still has no way to name register 0.

```compile_fail,E0599
let _ = device::command::Command::WriteRegisterZero;
```
