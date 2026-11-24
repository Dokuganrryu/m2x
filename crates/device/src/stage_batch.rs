use crate::{boot, rate_change};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchRole {
    StageCommand,
    StageSafetyEvent,
    OutsideTheStage,
}

impl BatchRole {
    pub const fn stops_after_a_stage_refusal(self) -> bool {
        match self {
            BatchRole::StageCommand => true,
            BatchRole::StageSafetyEvent | BatchRole::OutsideTheStage => false,
        }
    }

    pub const fn refusal_belongs_to_the_stage(self) -> bool {
        match self {
            BatchRole::StageCommand | BatchRole::StageSafetyEvent => true,
            BatchRole::OutsideTheStage => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Refused,
    NotRefused,
}

pub trait StageOutput: Copy {
    fn batch_role(self) -> BatchRole;
}

impl StageOutput for boot::Output {
    fn batch_role(self) -> BatchRole {
        match self {
            boot::Output::Command(_) => BatchRole::StageCommand,
            boot::Output::Phantom(_) => BatchRole::StageSafetyEvent,
            boot::Output::Supervisor(_) | boot::Output::Log(_) | boot::Output::StartLcd => {
                BatchRole::OutsideTheStage
            }
        }
    }
}

impl StageOutput for rate_change::Output {
    fn batch_role(self) -> BatchRole {
        match self {
            rate_change::Output::Board(_) | rate_change::Output::SendAnalogWordAtSpeed(_) => {
                BatchRole::StageCommand
            }
            rate_change::Output::HoldVolumeAtFull
            | rate_change::Output::RampVolumeFromFull
            | rate_change::Output::ResetStreams(_)
            | rate_change::Output::SwapCoefficients(_)
            | rate_change::Output::ClearDspState
            | rate_change::Output::ClockValidity(_)
            | rate_change::Output::RaiseFault(_) => BatchRole::OutsideTheStage,
        }
    }
}

pub fn run_stage_batch<Output: StageOutput>(
    outputs: &[Output],
    mut run: impl FnMut(Output) -> Refusal,
) -> Refusal {
    let mut stage_refused = false;
    for &output in outputs {
        let role = output.batch_role();
        if stage_refused && role.stops_after_a_stage_refusal() {
            continue;
        }
        let refusal = run(output);
        stage_refused |= role.refusal_belongs_to_the_stage() && refusal == Refusal::Refused;
    }
    if stage_refused {
        Refusal::Refused
    } else {
        Refusal::NotRefused
    }
}
