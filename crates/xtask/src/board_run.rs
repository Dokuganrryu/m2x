mod gate;
mod scenario;
mod toolchain;
mod trace;

pub use gate::{
    AfterFault, AnalogWords, Check, Expectation, ExpectedEnding, FenceProbe, MainClock,
    ParkFunction, RateChange, Recorded, Silence, Verdict, furthest_boot_step, judge,
};
pub use scenario::{FENCE, RATE_CHANGE_FOLLOW_UP, SCENARIOS, Scenario};
pub use trace::{
    Access, Ending, Entry, Event, I2cStatus, I2cTransaction, Level, Nanoseconds, Trace, TraceError,
};

use crate::exit::Outcome;
use device::usb::handover::rate_request_word;
use dsp::SampleRate;
use std::fmt::Write;
use toolchain::{Image, Workspace};

const PROVES: &str = "\
What this proves. The image's own instructions ran from Reset, so its boot logic and every register
access it made are checked against the documents as this model reads them: the bus traffic, the
order of the boot steps, the fault paths and the MPU fence as the image programs it.
What it cannot prove. That the documents are right about the hardware, the timing of real silicon,
anything analog, or any behaviour listed below as a guess.";

pub fn command() -> Outcome {
    match run_scenarios() {
        Ok((report, passed)) => {
            print!("{report}");
            if passed {
                Outcome::GatePassed
            } else {
                Outcome::GateFailed
            }
        }
        Err(message) => {
            eprintln!("{message}");
            Outcome::UsageOrInputError
        }
    }
}

pub fn plan(image: &Image, scenario: &Scenario) -> String {
    let mut plan = format!(
        "load {}\nentry {:#x}\nbreadcrumb {:#x}\n",
        image.elf.display(),
        image.reset,
        image.breadcrumb
    );
    let mut generation = 0;
    for line in scenario.board {
        match host_rate_request(line) {
            Some((rate, at)) => {
                generation += 1;
                let word = rate_request_word(generation, rate);
                let _ = writeln!(
                    plan,
                    "handover-write {:#x} {word:#x} {at}",
                    image.usb_notices
                );
            }
            None => {
                plan.push_str(line);
                plan.push('\n');
            }
        }
    }
    let _ = writeln!(plan, "run {}", scenario.run_milliseconds);
    for probe in scenario.expectation.fence {
        let access = match probe.access {
            Access::Read => "read",
            Access::Write => "write",
            Access::Fetch => "fetch",
        };
        let _ = writeln!(plan, "probe-fence {:#x} {access}", probe.address);
    }
    plan
}

fn host_rate_request(line: &str) -> Option<(SampleRate, u64)> {
    let ["host-requests-rate", hertz, "at", milliseconds] =
        line.split_whitespace().collect::<Vec<_>>()[..]
    else {
        return None;
    };
    let hertz: u32 = hertz.parse().ok()?;
    let rate = SampleRate::ALL
        .into_iter()
        .find(|rate| rate.hz() == hertz)?;
    Some((rate, milliseconds.parse().ok()?))
}

fn run_scenarios() -> Result<(String, bool), String> {
    let workspace = Workspace::locate();
    workspace.python()?;
    let image: Image = workspace.build_image()?;
    let mut report = format!(
        "board-run: {} booted from Reset under Unicorn against the board model\n{PROVES}\n\n",
        workspace.relative(&image.elf)
    );
    let mut failed = 0;
    let mut judged = 0;
    let mut guesses = Vec::new();
    for scenario in &SCENARIOS {
        let plan = plan(&image, scenario);
        let output = workspace.emulate(&plan)?;
        workspace.keep(&format!("{}.plan", scenario.name), &plan)?;
        workspace.keep(&format!("{}.trace", scenario.name), &output)?;
        let trace = Trace::parse(&output).map_err(|error| error.to_string())?;
        if guesses.is_empty() {
            guesses = trace.guesses();
        }
        let verdicts = judge(&trace, &scenario.expectation, &image.park);
        let failures = verdicts
            .iter()
            .filter(|verdict| verdict.failure.is_some())
            .count();
        failed += failures;
        judged += verdicts.len();
        let _ = writeln!(
            report,
            "scenario {}: reached {}, ended {:?}, {} of {} checks pass\n  today: {}",
            scenario.name,
            furthest_boot_step(&trace).unwrap_or("nothing"),
            trace.ending(),
            verdicts.len() - failures,
            verdicts.len(),
            scenario.today
        );
        for verdict in &verdicts {
            match &verdict.failure {
                None => {
                    let _ = writeln!(report, "  pass  {}", verdict.check.name());
                }
                Some(failure) => {
                    let _ = writeln!(report, "  FAIL  {}: {failure}", verdict.check.name());
                }
            }
        }
    }
    let _ = writeln!(
        report,
        "\nAssumptions the model makes that no document backs:"
    );
    for guess in &guesses {
        let _ = writeln!(report, "  - {guess}");
    }
    let _ = writeln!(report, "\nFollow-up: {RATE_CHANGE_FOLLOW_UP}");
    let _ = writeln!(
        report,
        "\n{} of {judged} checks pass in {} scenarios; traces in {}",
        judged - failed,
        SCENARIOS.len(),
        workspace.relative(&workspace.output_dir())
    );
    Ok((report, failed == 0))
}
