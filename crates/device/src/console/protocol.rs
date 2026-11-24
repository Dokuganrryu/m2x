use super::line::Words;
use super::number::whole_number;
use super::refusal::Refusal;
use crate::register_zero::{ConfirmationNonce, OperatorStep};

pub(super) fn operator_step(words: &mut Words<'_>) -> Result<OperatorStep, Refusal> {
    match words.next() {
        Some(b"signed") => Ok(OperatorStep::ProtocolSigned),
        Some(b"spare") => Ok(OperatorStep::SpareUnit),
        Some(b"unplugged") => Ok(OperatorStep::OutputsUnplugged),
        Some(b"request") => Ok(OperatorStep::RequestReplay),
        Some(b"abort") => Ok(OperatorStep::Abort),
        Some(b"confirm") => {
            let typed = whole_number(words.next().ok_or(Refusal::MalformedArguments)?)?;
            ConfirmationNonce::new(typed)
                .map(OperatorStep::Confirm)
                .ok_or(Refusal::SettingOutOfRange)
        }
        _ => Err(Refusal::NotAllowlisted),
    }
}
