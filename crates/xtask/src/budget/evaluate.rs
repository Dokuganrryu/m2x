use super::attribution::{Attribution, attribute};
use super::program::{FunctionCost, PricedTrace};
use super::run::{FRAME_COUNTS, Monitoring, PHASES, Phase, Preset, Run, TickRun};
use crate::price::{Block, Share, TickMode, UsbPath};
use dsp::FramesPerTick;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Figure {
    pub instructions: u32,
    pub cycles: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickBreakdown {
    pub blocks: BTreeMap<Block, Figure>,
    pub monitored_kernels: Figure,
    pub context: Figure,
    pub dma_moves: Figure,
    pub unattributed: BTreeMap<String, Figure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Within { measured: u32, share: u32 },
    Over { measured: u32, share: u32 },
    Off,
    RunsWhereD4TurnsItOff { measured: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub block: Block,
    pub cells: [Cell; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeTotal {
    pub run: TickRun,
    pub dsp: Figure,
    pub context: Figure,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub rows: Vec<Row>,
    pub modes: Vec<ModeTotal>,
    pub worst: [Option<ModeTotal>; 3],
    pub usb: [Figure; 3],
    pub unattributed: Vec<(Run, String, Figure)>,
    pub image_ticks: Vec<(Phase, TickBreakdown)>,
    pub bench_ticks_like_the_image: Vec<(Phase, TickBreakdown)>,
    pub image_usb_idle: Figure,
}

pub const ENTRY_AND_EXIT: u32 = 22;

impl Figure {
    fn of(cost: &FunctionCost) -> Self {
        Self {
            instructions: cost.instructions,
            cycles: cost.cycles,
        }
    }

    fn plus(self, other: Figure) -> Self {
        Self {
            instructions: self.instructions + other.instructions,
            cycles: self.cycles + other.cycles,
        }
    }

    fn worst(self, other: Figure) -> Self {
        if other.cycles > self.cycles {
            other
        } else {
            self
        }
    }
}

impl TickBreakdown {
    pub fn of(priced: &PricedTrace, phase: Phase) -> Self {
        let mut breakdown = Self::default();
        for (function, cost) in &priced.per_function {
            let figure = Figure::of(cost);
            match attribute(function, phase) {
                Attribution::Dsp(block) => {
                    let entry = breakdown.blocks.entry(block).or_default();
                    *entry = entry.plus(figure);
                }
                Attribution::MonitoredKernel => {
                    breakdown.monitored_kernels = breakdown.monitored_kernels.plus(figure)
                }
                Attribution::TickContext => breakdown.context = breakdown.context.plus(figure),
                Attribution::DmaMoves => breakdown.dma_moves = breakdown.dma_moves.plus(figure),
                Attribution::Unattributed => {
                    breakdown.unattributed.insert(function.clone(), figure);
                }
            }
        }
        breakdown
    }

    pub fn dsp(&self) -> Figure {
        self.blocks
            .values()
            .fold(self.monitored_kernels, |sum, figure| sum.plus(*figure))
    }

    fn block(&self, block: Block) -> u32 {
        self.blocks.get(&block).map_or(0, |figure| figure.cycles)
    }
}

pub fn evaluate(traces: &[(Run, PricedTrace)]) -> Evaluation {
    let mut ticks: Vec<(TickRun, Vec<TickBreakdown>)> = Vec::new();
    let mut usb: BTreeMap<(usize, UsbPath), Figure> = BTreeMap::new();
    let mut image_ticks: BTreeMap<Phase, TickBreakdown> = BTreeMap::new();
    let mut image_usb_idle = Figure::default();
    let mut unattributed = Vec::new();
    for (run, priced) in traces {
        match *run {
            Run::BenchTick(tick) => {
                let breakdown = TickBreakdown::of(priced, tick.phase);
                for (function, figure) in &breakdown.unattributed {
                    unattributed.push((*run, function.clone(), *figure));
                }
                match ticks.iter_mut().find(|(run, _)| *run == tick) {
                    Some((_, samples)) => samples.push(breakdown),
                    None => ticks.push((tick, vec![breakdown])),
                }
            }
            Run::BenchUsb(frames, path) => {
                let figure = Figure {
                    instructions: priced.total_instructions(),
                    cycles: priced.total_cycles(),
                };
                let entry = usb.entry((frames.count(), path)).or_default();
                *entry = entry.worst(figure);
            }
            Run::ImageTick(phase) => {
                let breakdown = TickBreakdown::of(priced, phase);
                let worse = image_ticks.get(&phase).is_none_or(|seen| {
                    breakdown.dsp().plus(breakdown.context).cycles
                        > seen.dsp().plus(seen.context).cycles
                });
                if worse {
                    image_ticks.insert(phase, breakdown);
                }
            }
            Run::ImageUsbIdle => {
                image_usb_idle = image_usb_idle.worst(Figure {
                    instructions: priced.total_instructions(),
                    cycles: priced.total_cycles(),
                });
            }
        }
    }
    let modes: Vec<ModeTotal> = ticks
        .iter()
        .map(|(run, samples)| worst_sample(*run, samples))
        .collect();
    let worst = FRAME_COUNTS.map(|frames| {
        modes
            .iter()
            .filter(|mode| mode.run.frames == frames)
            .max_by_key(|mode| mode.dsp.cycles)
            .copied()
    });
    let usb_figures = FRAME_COUNTS.map(|frames| {
        UsbPath::EVERY_MICROFRAME
            .iter()
            .filter_map(|path| usb.get(&(frames.count(), *path)))
            .fold(Figure::default(), |sum, figure| sum.plus(*figure))
    });
    let rows = Block::ALL
        .iter()
        .map(|block| Row {
            block: *block,
            cells: FRAME_COUNTS.map(|frames| {
                cell(
                    *block,
                    frames,
                    measured(*block, frames, &ticks, &modes, &worst, &usb),
                )
            }),
        })
        .collect();
    Evaluation {
        rows,
        modes,
        worst,
        usb: usb_figures,
        unattributed,
        image_ticks: image_ticks.into_iter().collect(),
        bench_ticks_like_the_image: bench_ticks_like_the_image(&ticks),
        image_usb_idle,
    }
}

fn bench_ticks_like_the_image(
    ticks: &[(TickRun, Vec<TickBreakdown>)],
) -> Vec<(Phase, TickBreakdown)> {
    ticks
        .iter()
        .filter(|(run, _)| {
            run.frames == FramesPerTick::Six
                && run.monitoring == Monitoring::Off
                && run.preset == Preset::Flat
        })
        .filter_map(|(run, samples)| {
            samples
                .iter()
                .max_by_key(|sample| sample.dsp().plus(sample.context).cycles)
                .map(|sample| (run.phase, sample.clone()))
        })
        .collect()
}

fn worst_sample(run: TickRun, samples: &[TickBreakdown]) -> ModeTotal {
    let (dsp, context) = samples.iter().fold(
        (Figure::default(), Figure::default()),
        |(dsp, context), sample| (dsp.worst(sample.dsp()), context.worst(sample.context)),
    );
    ModeTotal { run, dsp, context }
}

fn measured(
    block: Block,
    frames: FramesPerTick,
    ticks: &[(TickRun, Vec<TickBreakdown>)],
    modes: &[ModeTotal],
    worst: &[Option<ModeTotal>; 3],
    usb: &BTreeMap<(usize, UsbPath), Figure>,
) -> u32 {
    let column = column_of(frames);
    let worst_dsp = worst[column].map_or(0, |mode| mode.dsp.cycles);
    let worst_tick = modes
        .iter()
        .filter(|mode| mode.run.frames == frames)
        .map(|mode| mode.dsp.cycles + mode.context.cycles)
        .max()
        .unwrap_or(0);
    let unmonitored_samples = || {
        ticks
            .iter()
            .filter(move |(run, _)| run.frames == frames && run.monitoring == Monitoring::Off)
            .flat_map(|(run, samples)| samples.iter().map(move |sample| (run.phase, sample)))
    };
    let fixed = |share: Share| match share {
        Share::Cap(cap) => cap.get(),
        Share::OffAtThisFrameCount => 0,
    };
    let dma_moves = ticks
        .iter()
        .filter(|(run, _)| run.frames == frames)
        .flat_map(|(_, samples)| samples.iter().map(|sample| sample.dma_moves.cycles))
        .max()
        .unwrap_or(0);
    let usb_interrupt = ENTRY_AND_EXIT
        + UsbPath::EVERY_MICROFRAME
            .iter()
            .filter_map(|path| usb.get(&(frames.count(), *path)))
            .map(|figure| figure.cycles)
            .sum::<u32>();
    match block {
        Block::TickEntryAndExit => fixed(block.share(frames)),
        Block::DmaMoves => dma_moves,
        Block::TickContext => ticks
            .iter()
            .filter(|(run, _)| run.frames == frames)
            .flat_map(|(_, samples)| samples.iter().map(|sample| sample.context.cycles))
            .max()
            .unwrap_or(0),
        Block::Dispatch(mode) => unmonitored_samples()
            .filter(|(phase, _)| phase.mode() == mode)
            .map(|(_, sample)| sample.block(block))
            .max()
            .unwrap_or(0),
        Block::MonitorMix(mode) => monitor_mix(mode, frames, modes),
        Block::DspWorstTick => worst_dsp,
        Block::TickAndUsb => worst_tick + ENTRY_AND_EXIT + dma_moves + usb_interrupt,
        Block::Usb(path) => usb
            .get(&(frames.count(), path))
            .map_or(0, |figure| figure.cycles),
        Block::UsbInterrupt => usb_interrupt,
        Block::CaptureRamp
        | Block::BypassOut
        | Block::OnIntake
        | Block::OnPeq
        | Block::OnLimiterPeak
        | Block::OnPeqOffIntakeWithPeak
        | Block::OnLimiterRamp
        | Block::FadeOut
        | Block::FadeOutOfOn
        | Block::SeedOnEntry => unmonitored_samples()
            .map(|(_, sample)| sample.block(block))
            .max()
            .unwrap_or(0),
    }
}

fn monitor_mix(mode: TickMode, frames: FramesPerTick, modes: &[ModeTotal]) -> u32 {
    let total = |monitoring: Monitoring, phase: Phase| {
        modes
            .iter()
            .find(|total| {
                total.run
                    == TickRun {
                        frames,
                        monitoring,
                        preset: Preset::FiveBands,
                        phase,
                    }
            })
            .map(|total| total.dsp.cycles)
    };
    PHASES
        .iter()
        .filter(|phase| phase.mode() == mode)
        .filter_map(|phase| {
            Some(
                total(Monitoring::BothInputs, *phase)?
                    .saturating_sub(total(Monitoring::Off, *phase)?),
            )
        })
        .max()
        .unwrap_or(0)
}

fn cell(block: Block, frames: FramesPerTick, measured: u32) -> Cell {
    match block.share(frames) {
        Share::Cap(cap) if measured <= cap.get() => Cell::Within {
            measured,
            share: cap.get(),
        },
        Share::Cap(cap) => Cell::Over {
            measured,
            share: cap.get(),
        },
        Share::OffAtThisFrameCount if measured == 0 => Cell::Off,
        Share::OffAtThisFrameCount => Cell::RunsWhereD4TurnsItOff { measured },
    }
}

const fn column_of(frames: FramesPerTick) -> usize {
    match frames {
        FramesPerTick::Six => 0,
        FramesPerTick::Twelve => 1,
        FramesPerTick::TwentyFour => 2,
    }
}

impl Cell {
    pub const fn passes(self) -> bool {
        matches!(self, Cell::Within { .. } | Cell::Off)
    }
}

impl Evaluation {
    pub fn failures(&self) -> usize {
        let cells = self
            .rows
            .iter()
            .flat_map(|row| row.cells)
            .filter(|cell| !cell.passes())
            .count();
        cells + self.unattributed.len()
    }
}
