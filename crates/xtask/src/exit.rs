use std::process::ExitCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    GatePassed,
    GateFailed,
    UsageOrInputError,
}

impl From<Outcome> for ExitCode {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::GatePassed => ExitCode::SUCCESS,
            Outcome::GateFailed => ExitCode::from(1),
            Outcome::UsageOrInputError => ExitCode::from(2),
        }
    }
}
