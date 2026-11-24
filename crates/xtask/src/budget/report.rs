use super::evaluate::{Cell, ENTRY_AND_EXIT, Evaluation, Figure, ModeTotal, TickBreakdown};
use super::provenance::Provenance;
use super::run::{FRAME_COUNTS, Monitoring, Phase, Preset};
use crate::price::{Basis, Block, ProxyCycles, Share, USB_INTERRUPT_CAPS, UsbPath};
use std::collections::BTreeMap;
use std::fmt::Write;

pub const LABEL: &str =
    "Cortex-M4 proxy prices of emulated Cortex-M33 instructions until measured on hardware";

const USB_ESTIMATE_FIXED: u32 = 300;
const USB_ESTIMATE_PER_FRAME: u32 = 8;
const USB_ESTIMATE_FEEDBACK: u32 = 30;

fn pct(cycles: u32) -> String {
    ProxyCycles::new(cycles).share_of_whole_tick().to_string()
}

fn cycles(value: u32) -> String {
    ProxyCycles::new(value).to_string()
}

fn margin(value: u32) -> u32 {
    ProxyCycles::new(value).with_unpublished_m33_margin().get()
}

pub fn share_cell(share: Share) -> String {
    match share {
        Share::Cap(cap) => format!("{} ({})", cap, cap.share_of_whole_tick()),
        Share::OffAtThisFrameCount => "off".to_string(),
    }
}

pub const fn basis_text(block: Block) -> &'static str {
    match (block, block.basis()) {
        (Block::DspWorstTick, _) => "R2, 50% of the tick",
        (Block::TickAndUsb, _) => "DSP-1 criterion, 70% of the tick",
        (Block::UsbInterrupt, _) => "decision",
        (
            Block::Usb(
                UsbPath::ControlTransfer
                | UsbPath::ConsoleReceived
                | UsbPath::StartOfFrameWithConsole,
            ),
            _,
        ) => "measured, worst traced event, not in the per-microframe cap",
        (Block::Usb(_), _) => "measured, worst traced microframe",
        (_, Basis::Published) => "M4 figure 12 + 10 in KA001156, M33 ",
        (_, Basis::MeasuredDifference) => {
            "measured, monitored tick minus unmonitored tick, worst phase"
        }
        (_, Basis::Measured | Basis::Cap) => "measured, worst traced tick",
    }
}

pub fn spec_share_table() -> String {
    let mut table = String::from("| Block | 6 frames | 12 frames | 24 frames | Basis |\n");
    table.push_str("|---|---|---|---|---|\n");
    for block in Block::ALL {
        let [six, twelve, twenty_four] = FRAME_COUNTS.map(|frames| share_cell(block.share(frames)));
        let _ = writeln!(
            table,
            "| {} | {six} | {twelve} | {twenty_four} | {} |",
            block.d3_label(),
            basis_text(block)
        );
    }
    table
}

fn cell_text(cell: Cell) -> String {
    match cell {
        Cell::Within { measured, share } => {
            format!(
                "{} of {} ({})",
                cycles(measured),
                cycles(share),
                pct(measured)
            )
        }
        Cell::Over { measured, share } => format!(
            "**FAIL** {} over {} by {} ({})",
            cycles(measured),
            cycles(share),
            cycles(measured - share),
            pct(measured)
        ),
        Cell::Off => "off".to_string(),
        Cell::RunsWhereD4TurnsItOff { measured } => {
            format!(
                "**FAIL** {} where PEQ is off at this frame count",
                cycles(measured)
            )
        }
    }
}

fn phase_of_mode_row(monitoring: Monitoring, preset: Preset, phase: Phase) -> String {
    let preset = match preset {
        Preset::FiveBands => "5 bands",
        Preset::Flat => "flat preset",
    };
    let monitoring = match monitoring {
        Monitoring::Off => "",
        Monitoring::BothInputs => ", inputs monitored",
    };
    format!("{}, {preset}{monitoring}", phase.describe())
}

fn figure_text(figure: Figure) -> String {
    format!(
        "{}, {} ({})",
        cycles(figure.instructions),
        cycles(figure.cycles),
        pct(figure.cycles)
    )
}

pub struct Sources<'a> {
    pub bench: &'a str,
    pub image: &'a str,
}

pub fn render(
    evaluation: &Evaluation,
    provenance: &BTreeMap<Block, Provenance>,
    sources: &Sources<'_>,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Budget of the audio tick and the USB interrupt\n");
    let _ = writeln!(out, "Every figure is {LABEL}.\n");
    let _ = writeln!(
        out,
        "Bench: `{}`, `crates/xtask/budget-bench` linked from `dsp`, `stream` and `device` with the image's release profile (fat LTO, one codegen unit, panic abort, the default opt level) for thumbv8m.main-none-eabi with target-cpu=cortex-m33 and -fpregs, the image's own target and flags. Its tick body copies `TickContext::run`. Its USB entries call `usb_device_interrupt`, the board's own decoder that the image's `UsbContext::service` calls, over a model of the USB1 registers and endpoint list in emulated memory, after a simulated host has enumerated the device and opened both streams. The decoder reads INTSTAT and the list words, hands `device::usb::service::Service::handle` its events and writes the commands back to the list. Between two starts of frame the host writes a playback packet of N+1 or N-1 frames, reads feedback every 8 microframes and reads a capture packet, with the converters running ahead so that capture packets of N+1 frames occur. The start-of-frame interrupt collects those completions.",
        sources.bench
    );
    let _ = writeln!(
        out,
        "Image: `{}`, the `cargo firmware` build. The Code column says, per row, whether every function the bench priced is instruction for instruction the image's copy, address constants aside.",
        sources.image
    );
    let _ = writeln!(
        out,
        "Pricing: each executed instruction at the Cortex-M4 TRM r0p1 proxy table, a branch by its real successor, an instruction whose IT condition failed (Unicorn does not report it) at 1 cycle, a runtime helper charged to its caller, every row its worst traced tick. The plans and traces are kept in `target/budget-bench/`.\n"
    );
    rows_section(&mut out, evaluation, provenance);
    modes_section(&mut out, evaluation);
    whole_tick_section(&mut out, evaluation);
    image_section(&mut out, evaluation);
    if !evaluation.unattributed.is_empty() {
        let _ = writeln!(out, "## Functions no budget row owns, each a failure\n");
        for (run, function, figure) in &evaluation.unattributed {
            let _ = writeln!(
                out,
                "- `{}` in `{}`: {} cycles",
                function,
                run.label(),
                cycles(figure.cycles)
            );
        }
        out.push('\n');
    }
    let _ = writeln!(
        out,
        "{} of {} gated cells outside their gate, {} unattributed functions. {LABEL}.",
        evaluation
            .rows
            .iter()
            .flat_map(|row| row.cells)
            .filter(|cell| !cell.passes())
            .count(),
        evaluation.rows.len() * 3,
        evaluation.unattributed.len()
    );
    out
}

fn rows_section(
    out: &mut String,
    evaluation: &Evaluation,
    provenance: &BTreeMap<Block, Provenance>,
) {
    let _ = writeln!(out, "## Budget rows, measured of share, proxy cycles\n");
    let _ = writeln!(out, "| Block | 6 frames | 12 frames | 24 frames | Code |");
    let _ = writeln!(out, "|---|---|---|---|---|");
    for row in &evaluation.rows {
        let [six, twelve, twenty_four] = row.cells.map(cell_text);
        let code = match row.block.basis() {
            Basis::Published => "not measured".to_string(),
            Basis::MeasuredDifference => {
                "monitored kernels, linked in the image and reached once thread mode publishes a monitor mix"
                    .to_string()
            }
            Basis::Cap => "sum of rows".to_string(),
            Basis::Measured => provenance
                .get(&row.block)
                .map_or_else(|| "not measured".to_string(), ToString::to_string),
        };
        let _ = writeln!(
            out,
            "| {} | {six} | {twelve} | {twenty_four} | {code} |",
            row.block.d3_label()
        );
    }
    out.push('\n');
}

fn modes_section(out: &mut String, evaluation: &Evaluation) {
    let _ = writeln!(
        out,
        "## DSP share per traced phase, worst of the traced ticks, instructions then proxy cycles\n"
    );
    let _ = writeln!(out, "| Phase | 6 frames | 12 frames | 24 frames |");
    let _ = writeln!(out, "|---|---|---|---|");
    let mut keys: Vec<(Monitoring, Preset, Phase)> = evaluation
        .modes
        .iter()
        .map(|mode| (mode.run.monitoring, mode.run.preset, mode.run.phase))
        .collect();
    keys.sort_by_key(|(monitoring, preset, phase)| (*preset, *monitoring, *phase));
    keys.dedup();
    for (monitoring, preset, phase) in keys {
        let cells = FRAME_COUNTS.map(|frames| {
            evaluation
                .modes
                .iter()
                .find(|mode| {
                    mode.run.frames == frames
                        && mode.run.monitoring == monitoring
                        && mode.run.preset == preset
                        && mode.run.phase == phase
                })
                .map_or_else(String::new, |mode| figure_text(mode.dsp))
        });
        let worst = evaluation.worst.iter().flatten().any(|mode: &ModeTotal| {
            mode.run.monitoring == monitoring
                && mode.run.preset == preset
                && mode.run.phase == phase
        });
        let marker = if worst {
            " **(worst at some frame count)**"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "| {}{marker} | {} | {} | {} |",
            phase_of_mode_row(monitoring, preset, phase),
            cells[0],
            cells[1],
            cells[2]
        );
    }
    out.push('\n');
}

fn whole_tick_section(out: &mut String, evaluation: &Evaluation) {
    let row = |block: Block| {
        evaluation
            .rows
            .iter()
            .find(|row| row.block == block)
            .map(|row| {
                row.cells.map(|cell| match cell {
                    Cell::Within { measured, .. }
                    | Cell::Over { measured, .. }
                    | Cell::RunsWhereD4TurnsItOff { measured } => measured,
                    Cell::Off => 0,
                })
            })
            .unwrap_or([0; 3])
    };
    let dsp = row(Block::DspWorstTick);
    let tick_and_usb = row(Block::TickAndUsb);
    let usb = row(Block::UsbInterrupt);
    let tick_total = [0, 1, 2].map(|column| tick_and_usb[column].saturating_sub(usb[column]));
    let estimate = FRAME_COUNTS.map(|frames| {
        USB_ESTIMATE_FIXED + USB_ESTIMATE_PER_FRAME * frames.count() as u32 + USB_ESTIMATE_FEEDBACK
    });
    let _ = writeln!(out, "## Whole tick\n");
    let _ = writeln!(out, "| Item | 6 frames | 12 frames | 24 frames |");
    let _ = writeln!(out, "|---|---|---|---|");
    let line = |out: &mut String, label: &str, values: [u32; 3]| {
        let [a, b, c] = values.map(|value| format!("{} ({})", cycles(value), pct(value)));
        let _ = writeln!(out, "| {label} | {a} | {b} | {c} |");
    };
    line(out, "DSP share, worst tick, cap 6,250", dsp);
    line(
        out,
        "Tick total. Worst DSP tick and its tick context, entry and exit, DMA buffer moves",
        tick_total,
    );
    line(
        out,
        "USB interrupt, measured. Entry and exit plus the microframe interrupt",
        usb,
    );
    line(
        out,
        "USB interrupt, budget estimate 300 + 8 per frame + 30",
        estimate,
    );
    line(
        out,
        "Tick total plus the USB interrupt, cap 8,750",
        tick_and_usb,
    );
    line(
        out,
        "Tick total plus the USB cap of 1,500, 1,600 and 1,800, reported, not gated",
        [0, 1, 2].map(|column| tick_total[column] + USB_INTERRUPT_CAPS[column]),
    );
    line(
        out,
        "DSP share, worst tick, proxy x1.5, reported, not gated",
        dsp.map(margin),
    );
    line(
        out,
        "Tick total, proxy x1.5, reported, not gated",
        tick_total.map(margin),
    );
    line(
        out,
        "USB interrupt, proxy x1.5, reported, not gated",
        usb.map(margin),
    );
    let microframe = row(Block::Usb(UsbPath::Microframe));
    let console_start = row(Block::Usb(UsbPath::StartOfFrameWithConsole));
    let control = row(Block::Usb(UsbPath::ControlTransfer));
    let console_received = row(Block::Usb(UsbPath::ConsoleReceived));
    line(
        out,
        "USB interrupt in a microframe that also carries the heaviest occasional event, reported, not gated",
        [0, 1, 2].map(|column| {
            usb[column] - microframe[column]
                + microframe[column].max(console_start[column])
                + control[column].max(console_received[column])
        }),
    );
    let _ = writeln!(
        out,
        "\nThe microframe USB row at 6, 12 and 24 frames: {}. Entry and exit add {ENTRY_AND_EXIT} once.\n",
        FRAME_COUNTS
            .iter()
            .zip(&evaluation.usb)
            .map(|(frames, figure)| format!(
                "{} instructions and {} cycles at {}",
                cycles(figure.instructions),
                cycles(figure.cycles),
                frames.count()
            ))
            .collect::<Vec<_>>()
            .join("; ")
    );
}

fn image_section(out: &mut String, evaluation: &Evaluation) {
    let _ = writeln!(
        out,
        "## The image's own handlers, emulated from `Reset`

The image boots at 48 kHz, 6 frames, with the flat preset, no monitor published, both inputs live and no USB stream, and its tick runs what that state reaches. Its DMA positions read the zeroed peripheral memory, so every tick moves the same blocks. The mode switch byte is written in RAM to reach On and the fades. Each cell is the image's figure, then the bench's for the same phase at 6 frames with the flat preset and no monitor. Entry and exit are in neither.
"
    );
    let _ = writeln!(
        out,
        "| Phase | DSP rows, image / bench | Tick context, image / bench | Rows that differ, image / bench |"
    );
    let _ = writeln!(out, "|---|---|---|---|");
    for (phase, image) in &evaluation.image_ticks {
        let Some((_, bench)) = evaluation
            .bench_ticks_like_the_image
            .iter()
            .find(|(bench_phase, _)| bench_phase == phase)
        else {
            continue;
        };
        let differing: Vec<String> = Block::ALL
            .iter()
            .filter_map(|block| {
                let of = |breakdown: &TickBreakdown| {
                    breakdown
                        .blocks
                        .get(block)
                        .map_or(0, |figure| figure.cycles)
                };
                (of(image) != of(bench))
                    .then(|| format!("{} {} / {}", block.id(), of(image), of(bench)))
            })
            .collect();
        let _ = writeln!(
            out,
            "| {} | {} / {} | {} / {} | {} |",
            phase.describe(),
            cycles(image.dsp().cycles),
            cycles(bench.dsp().cycles),
            cycles(image.context.cycles),
            cycles(bench.context.cycles),
            if differing.is_empty() {
                "none".to_string()
            } else {
                differing.join(", ")
            }
        );
    }
    let _ = writeln!(
        out,
        "
`usb_interrupt` with no stream selected: {}.
",
        figure_text(evaluation.image_usb_idle)
    );
}
