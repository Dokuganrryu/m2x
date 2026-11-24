mod attribution;
mod evaluate;
mod plan;
mod program;
mod provenance;
mod report;
mod run;
mod toolchain;

pub use evaluate::{Cell, Evaluation, Figure, ModeTotal, Row, TickBreakdown, evaluate};
pub use program::{
    Executed, FunctionCost, PricedTrace, Program, ProgramError, Trace, read_boot_end, read_traces,
};
pub use provenance::{Provenance, provenance_by_block, same_code};
pub use report::{LABEL, spec_share_table};
pub use run::{Monitoring, Phase, Preset, Run, TickRun};
pub use toolchain::{PYTHON_VARIABLE, python_for};

use crate::exit::Outcome;
use std::fmt::Display;
use toolchain::Workspace;

pub fn command() -> Outcome {
    match measure() {
        Ok(passed) if passed => Outcome::GatePassed,
        Ok(_) => Outcome::GateFailed,
        Err(message) => {
            eprintln!("{message}");
            Outcome::UsageOrInputError
        }
    }
}

fn refuse(error: impl Display) -> String {
    error.to_string()
}

fn measure() -> Result<bool, String> {
    let workspace = Workspace::locate();
    workspace.check_python().map_err(refuse)?;
    let bench_build = workspace.build_bench().map_err(refuse)?;
    let image_build = workspace.build_image().map_err(refuse)?;
    let bench =
        Program::read(&bench_build.instruction_listing, &bench_build.symbols).map_err(refuse)?;
    let image =
        Program::read(&image_build.instruction_listing, &image_build.symbols).map_err(refuse)?;
    let plan = plan::bench_plan(&bench_build.elf, &bench)
        .map_err(|missing| format!("the bench has no symbol `{}`", missing.0))?;
    let image_plan = plan::image_plan(&image_build.elf, &image, &image_build.symbols)
        .map_err(|missing| format!("the image has no symbol `{}`", missing.0))?;
    let bench_output = toolchain::emulate(&workspace, &plan).map_err(refuse)?;
    let image_output = toolchain::emulate(&workspace, &image_plan).map_err(refuse)?;
    workspace
        .keep_emulation(&[
            ("bench-plan.txt", &plan),
            ("bench-traces.txt", &bench_output),
            ("image-plan.txt", &image_plan),
            ("image-traces.txt", &image_output),
        ])
        .map_err(refuse)?;
    check_the_image_booted(&image, &image_output)?;
    let mut priced = Vec::new();
    for (program, output) in [(&bench, &bench_output), (&image, &image_output)] {
        for trace in read_traces(output).map_err(refuse)? {
            let run = Run::from_label(&trace.label).ok_or_else(|| {
                format!("the emulator printed an unknown trace `{}`", trace.label)
            })?;
            priced.push((run, program.price(&trace).map_err(refuse)?));
        }
    }
    let evaluation = evaluate(&priced);
    let provenance = provenance_by_block(&priced, &bench, &image);
    print!(
        "{}",
        report::render(
            &evaluation,
            &provenance,
            &report::Sources {
                bench: &workspace.relative(&bench_build.elf),
                image: &workspace.relative(&image_build.elf),
            },
        )
    );
    Ok(evaluation.failures() == 0)
}

fn check_the_image_booted(image: &Program, output: &str) -> Result<(), String> {
    let end = read_boot_end(output)
        .map_err(refuse)?
        .ok_or("the emulator printed no `booted` line for the image")?;
    match image.function_holding(end) {
        plan::IMAGE_MAIN_LOOP => Ok(()),
        holder => Err(format!(
            "the image's boot ended at {end:#x} in `{holder}`, not in its main loop `{}`, so its interrupts would run without the contexts the boot installs",
            plan::IMAGE_MAIN_LOOP
        )),
    }
}
