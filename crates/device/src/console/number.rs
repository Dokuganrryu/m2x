use super::refusal::Refusal;

const DECIMAL_POINT: u8 = b'.';
const DECIMAL_BASE: u32 = 10;
const TENTHS_DIGITS: usize = 1;
const MOST_EXACT_DECIMAL_DIGITS: usize = 15;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sign {
    Unwritten,
    Plus,
    Minus,
}

struct Decimal<'word> {
    sign: Sign,
    whole: &'word [u8],
    fraction: &'word [u8],
}

impl<'word> Decimal<'word> {
    fn split(word: &'word [u8]) -> Result<Self, Refusal> {
        let (sign, digits) = match word {
            [b'+', rest @ ..] => (Sign::Plus, rest),
            [b'-', rest @ ..] => (Sign::Minus, rest),
            _ => (Sign::Unwritten, word),
        };
        let (whole, fraction) = match digits.iter().position(|&byte| byte == DECIMAL_POINT) {
            Some(point) => {
                let (whole, point_and_fraction) = digits.split_at(point);
                let fraction = &point_and_fraction[1..];
                if fraction.is_empty() {
                    return Err(Refusal::MalformedArguments);
                }
                (whole, fraction)
            }
            None => (digits, &digits[digits.len()..]),
        };
        let all_digits = |part: &[u8]| part.iter().all(u8::is_ascii_digit);
        if whole.is_empty() || !all_digits(whole) || !all_digits(fraction) {
            return Err(Refusal::MalformedArguments);
        }
        Ok(Self {
            sign,
            whole,
            fraction,
        })
    }
}

pub(crate) fn decimal(word: &[u8]) -> Result<f64, Refusal> {
    let decimal = Decimal::split(word)?;
    if decimal.whole.len() > MOST_EXACT_DECIMAL_DIGITS {
        return Err(Refusal::SettingOutOfRange);
    }
    if decimal.whole.len() + decimal.fraction.len() > MOST_EXACT_DECIMAL_DIGITS {
        return Err(Refusal::MalformedArguments);
    }
    let mantissa = decimal
        .whole
        .iter()
        .chain(decimal.fraction)
        .fold(0_u64, |sum, &digit| {
            sum * u64::from(DECIMAL_BASE) + u64::from(digit - b'0')
        });
    let scale = decimal
        .fraction
        .iter()
        .fold(1.0_f64, |scale, _| scale * f64::from(DECIMAL_BASE));
    let magnitude = mantissa as f64 / scale;
    Ok(match decimal.sign {
        Sign::Minus => -magnitude,
        Sign::Plus | Sign::Unwritten => magnitude,
    })
}

pub(crate) fn whole_number(word: &[u8]) -> Result<u32, Refusal> {
    let decimal = Decimal::split(word)?;
    if decimal.sign != Sign::Unwritten || !decimal.fraction.is_empty() {
        return Err(Refusal::MalformedArguments);
    }
    accumulate(decimal.whole)
}

pub(crate) fn tenths(word: &[u8]) -> Result<i32, Refusal> {
    let decimal = Decimal::split(word)?;
    if decimal.fraction.len() > TENTHS_DIGITS {
        return Err(Refusal::MalformedArguments);
    }
    let whole = accumulate(decimal.whole)?;
    let tenths_digit = match decimal.fraction {
        [] => 0,
        fraction => accumulate(fraction)?,
    };
    let magnitude = whole
        .checked_mul(DECIMAL_BASE)
        .and_then(|scaled| scaled.checked_add(tenths_digit))
        .and_then(|magnitude| i32::try_from(magnitude).ok())
        .ok_or(Refusal::SettingOutOfRange)?;
    Ok(match decimal.sign {
        Sign::Minus => -magnitude,
        Sign::Unwritten | Sign::Plus => magnitude,
    })
}

fn accumulate(digits: &[u8]) -> Result<u32, Refusal> {
    digits.iter().try_fold(0u32, |sum, &digit| {
        sum.checked_mul(DECIMAL_BASE)
            .and_then(|scaled| scaled.checked_add(u32::from(digit - b'0')))
            .ok_or(Refusal::SettingOutOfRange)
    })
}
