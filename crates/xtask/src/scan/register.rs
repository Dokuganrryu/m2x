#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterValue {
    Unknown,
    Known(u32),
    HighHalfOnly(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterEvent {
    MoveWide(u16),
    MoveTop(u16),
    Load(u32),
    AddImmediate(i64),
    Clobber,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Formed {
    Value(u32),
    HighHalfOnly(u16),
}

impl RegisterValue {
    pub fn next(self, event: RegisterEvent) -> (RegisterValue, Option<Formed>) {
        match (self, event) {
            (
                RegisterValue::Unknown | RegisterValue::Known(_) | RegisterValue::HighHalfOnly(_),
                RegisterEvent::MoveWide(low),
            ) => (RegisterValue::Known(u32::from(low)), None),
            (RegisterValue::Known(value), RegisterEvent::MoveTop(high)) => {
                let formed = (value & 0xFFFF) | (u32::from(high) << 16);
                (RegisterValue::Known(formed), Some(Formed::Value(formed)))
            }
            (
                RegisterValue::Unknown | RegisterValue::HighHalfOnly(_),
                RegisterEvent::MoveTop(high),
            ) => (
                RegisterValue::HighHalfOnly(high),
                Some(Formed::HighHalfOnly(high)),
            ),
            (
                RegisterValue::Unknown | RegisterValue::Known(_) | RegisterValue::HighHalfOnly(_),
                RegisterEvent::Load(value),
            ) => (RegisterValue::Known(value), None),
            (RegisterValue::Known(value), RegisterEvent::AddImmediate(offset)) => {
                let formed = (i64::from(value) + offset) as u32;
                (RegisterValue::Known(formed), Some(Formed::Value(formed)))
            }
            (RegisterValue::Unknown, RegisterEvent::AddImmediate(_)) => {
                (RegisterValue::Unknown, None)
            }
            (RegisterValue::HighHalfOnly(high), RegisterEvent::AddImmediate(_)) => {
                (RegisterValue::HighHalfOnly(high), None)
            }
            (
                RegisterValue::Unknown | RegisterValue::Known(_) | RegisterValue::HighHalfOnly(_),
                RegisterEvent::Clobber,
            ) => (RegisterValue::Unknown, None),
        }
    }
}
