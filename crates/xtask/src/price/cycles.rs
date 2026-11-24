use std::fmt;
use std::iter::Sum;
use std::ops::Add;

pub const WHOLE_TICK: ProxyCycles = ProxyCycles(12_500);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProxyCycles(u32);

impl ProxyCycles {
    pub const fn new(cycles: u32) -> Self {
        Self(cycles)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn with_unpublished_m33_margin(self) -> Self {
        Self((self.0 * 3).div_ceil(2))
    }

    pub const fn share_of_whole_tick(self) -> TenthsOfAPercent {
        TenthsOfAPercent((self.0 * 1000 + WHOLE_TICK.0 / 2) / WHOLE_TICK.0)
    }
}

impl Add for ProxyCycles {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sum for ProxyCycles {
    fn sum<I: Iterator<Item = Self>>(cycles: I) -> Self {
        cycles.fold(Self(0), Add::add)
    }
}

impl fmt::Display for ProxyCycles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_with_thousands_separators(f, self.0)
    }
}

fn write_with_thousands_separators(f: &mut fmt::Formatter<'_>, value: u32) -> fmt::Result {
    let digits = value.to_string();
    let first_group = digits.len() % 3;
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && index % 3 == first_group {
            f.write_str(",")?;
        }
        write!(f, "{digit}")?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TenthsOfAPercent(u32);

impl fmt::Display for TenthsOfAPercent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}%", self.0 / 10, self.0 % 10)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstructionCount(u32);

impl InstructionCount {
    pub const fn new(count: u32) -> Self {
        Self(count)
    }

    pub const fn count(self) -> u32 {
        self.0
    }
}

impl fmt::Display for InstructionCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_with_thousands_separators(f, self.0)
    }
}
