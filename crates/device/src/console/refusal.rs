#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotAllowlisted,
    MalformedArguments,
    SettingOutOfRange,
    LineTooLong,
    NotPrintableAscii,
    NotRunning,
}

impl Refusal {
    pub const fn reply(self) -> &'static [u8] {
        match self {
            Self::NotAllowlisted => b"refused: not a command\r\n",
            Self::MalformedArguments => b"refused: bad arguments\r\n",
            Self::SettingOutOfRange => b"refused: out of range\r\n",
            Self::LineTooLong => b"refused: line too long\r\n",
            Self::NotPrintableAscii => b"refused: not printable ascii\r\n",
            Self::NotRunning => b"refused: not running\r\n",
        }
    }
}
