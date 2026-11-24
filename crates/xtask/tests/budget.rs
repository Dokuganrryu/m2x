use dsp::FramesPerTick;
use xtask::budget::{
    Cell, FunctionCost, Monitoring, Phase, Preset, PricedTrace, Program, Run, TickRun, evaluate,
    read_boot_end, read_traces, same_code,
};
use xtask::price::{Block, Share, TickMode, UsbPath};

const INSTRUCTION_LISTING: &str = "
target/budget.elf:\tfile format elf32-littlearm

Disassembly of section .text:

00000010 <kernel>:
      10: b510         \tpush\t{r4, lr}
      12: 2800         \tcmp\tr0, #0x0
      14: d1fd         \tbne\t0x12 <kernel+0x2>
      16: bd10         \tpop\t{r4, pc}

00000018 <entry>:
      18: b580         \tpush\t{r7, lr}
      1a: f7ff fff9    \tbl\t0x10 <kernel>
      1e: bd80         \tpop\t{r7, pc}
";

const SYMBOLS: &str = "\
00000010 00000008 T kernel
00000018 00000008 T entry
";

#[test]
fn a_trace_is_priced_instruction_by_instruction_with_each_branch_judged_by_its_real_successor() {
    let program = Program::read(INSTRUCTION_LISTING, SYMBOLS).expect("the program reads");
    let traces =
        read_traces("trace one\n18 2\n1a 4\n10 2\n12 2\n14 2\n12 2\n14 2\n16 2\n1e 2\nend\n")
            .expect("the trace reads");
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].label, "one");
    let priced = program
        .price(&traces[0])
        .expect("every executed address is decoded");
    let kernel = priced.function("kernel").expect("kernel ran");
    assert_eq!(kernel.instructions, 6);
    assert_eq!(kernel.cycles, 3 + 1 + 2 + 1 + 1 + 5);
    let entry = priced.function("entry").expect("entry ran");
    assert_eq!(entry.instructions, 3);
    assert_eq!(entry.cycles, 3 + 2 + 5);
    assert_eq!(priced.total_cycles(), 13 + 10);
}

#[test]
fn an_instruction_whose_it_condition_fails_is_missing_from_the_emulator_trace_and_still_costs_one_cycle()
 {
    let instruction_listing = "
00000010 <select>:
      10: bf38         \tit\tlo
      12: 4652         \tmovlo\tr2, r10
      14: bf0c         \tite\teq
      16: 2001         \tmoveq\tr0, #0x1
      18: 2000         \tmovne\tr0, #0x0
      1a: 4770         \tbx\tlr
";
    let symbols = "00000010 0000000c T select\n";
    let program = Program::read(instruction_listing, symbols).expect("the program reads");
    let traces = read_traces("trace t\n10 2\n14 2\n18 2\n1a 2\nend\n").expect("the trace reads");
    let priced = program.price(&traces[0]).expect("the trace prices");
    let select = priced.function("select").expect("select ran");
    assert_eq!(select.instructions, 6);
    assert_eq!(select.cycles, 1 + 1 + 1 + 1 + 1 + 2);
}

#[test]
fn a_runtime_helper_is_charged_to_the_function_that_called_it() {
    let instruction_listing = "
00000010 <__aeabi_memclr4>:
      10: 4770         \tbx\tlr
00000012 <first>:
      12: f7ff fffd    \tbl\t0x10 <__aeabi_memclr4>
      16: 4770         \tbx\tlr
00000018 <second>:
      18: f7ff fffa    \tbl\t0x10 <__aeabi_memclr4>
      1c: 4770         \tbx\tlr
";
    let symbols = "\
00000010 00000002 T __aeabi_memclr4
00000012 00000006 T first
00000018 00000006 T second
";
    let program = Program::read(instruction_listing, symbols).expect("the program reads");
    let traces =
        read_traces("trace t\n12 4\n10 2\n16 2\n18 4\n10 2\n1c 2\nend\n").expect("the trace reads");
    let priced = program.price(&traces[0]).expect("the trace prices");
    assert_eq!(
        priced.function("first").map(|cost| cost.instructions),
        Some(3)
    );
    assert_eq!(
        priced.function("second").map(|cost| cost.instructions),
        Some(3)
    );
    assert_eq!(priced.function("__aeabi_memclr4"), None);
}

fn kernel_at(base: u32, address_word: &str, second: &str) -> (String, String) {
    let instruction_listing = format!(
        "
{base:08x} <dsp::kernel>:
    {a:x}: f240 0500    \tmovw\tr5, #{address_word}
    {b:x}: d1fd         \tbne\t{target:#x} <dsp::kernel+0x2>
    {c:x}: {second}
",
        a = base,
        b = base + 4,
        c = base + 6,
        target = base + 2,
    );
    (
        instruction_listing,
        format!("{base:08x} 00000008 T dsp::kernel\n"),
    )
}

#[test]
fn a_function_is_the_image_code_when_only_its_addresses_differ_and_not_when_an_instruction_does() {
    let (bench_text, bench_symbols) = kernel_at(0x100, "0xc960", "4770         \tbx\tlr");
    let (image_text, image_symbols) = kernel_at(0x2000_0100, "0xa418", "4770         \tbx\tlr");
    let (other_text, other_symbols) = kernel_at(0x2000_0100, "0xa418", "bf00         \tnop");
    let bench = Program::read(&bench_text, &bench_symbols).expect("bench reads");
    let image = Program::read(&image_text, &image_symbols).expect("image reads");
    let other = Program::read(&other_text, &other_symbols).expect("other reads");
    assert_eq!(same_code(&bench, &image, "dsp::kernel"), Some(true));
    assert_eq!(same_code(&bench, &other, "dsp::kernel"), Some(false));
    assert_eq!(same_code(&bench, &image, "dsp::absent"), None);
}

#[test]
fn an_executed_address_the_instruction_listing_does_not_hold_refuses_the_trace() {
    let program = Program::read(INSTRUCTION_LISTING, SYMBOLS).expect("the program reads");
    let traces = read_traces("trace one\n18 2\n40 2\nend\n").expect("the trace reads");
    let refusal = program.price(&traces[0]).expect_err("0x40 is not code");
    assert!(refusal.to_string().contains("0x40"), "{refusal}");
}

#[test]
fn the_boot_line_names_where_the_image_stopped_and_the_traces_after_it_still_read() {
    let output = "booted 1c
trace one
18 2
end
";
    let program = Program::read(INSTRUCTION_LISTING, SYMBOLS).expect("the program reads");
    let end = read_boot_end(output)
        .expect("the boot line reads")
        .expect("there is a boot line");
    assert_eq!(program.function_holding(end), "entry");
    assert_eq!(read_traces(output).expect("the traces read").len(), 1);
    assert_eq!(
        read_boot_end(
            "trace one
18 2
end
"
        ),
        Ok(None)
    );
}

#[test]
fn an_executed_width_that_differs_from_the_instruction_listing_refuses_the_trace() {
    let program = Program::read(INSTRUCTION_LISTING, SYMBOLS).expect("the program reads");
    let traces = read_traces("trace one\n18 4\nend\n").expect("the trace reads");
    assert!(program.price(&traces[0]).is_err());
}

fn priced(functions: &[(&str, u32)]) -> PricedTrace {
    PricedTrace {
        per_function: functions
            .iter()
            .map(|(name, cycles)| {
                (
                    name.to_string(),
                    FunctionCost {
                        instructions: *cycles,
                        cycles: *cycles,
                    },
                )
            })
            .collect(),
    }
}

fn tick(frames: FramesPerTick, monitoring: Monitoring, phase: Phase) -> Run {
    Run::BenchTick(TickRun {
        frames,
        monitoring,
        preset: Preset::FiveBands,
        phase,
    })
}

fn cell_of(evaluation: &xtask::budget::Evaluation, block: Block, column: usize) -> Cell {
    evaluation
        .rows
        .iter()
        .find(|row| row.block == block)
        .map(|row| row.cells[column])
        .expect("every block has a row")
}

fn share_at_six(block: Block) -> u32 {
    match block.share(FramesPerTick::Six) {
        Share::Cap(cap) => cap.get(),
        Share::OffAtThisFrameCount => 0,
    }
}

const CAPTURE_SIX: &str = "<dsp::capture::Capture as dsp::tick::FrameCountKernel>::process::<6>";

#[test]
fn a_block_takes_its_worst_traced_tick_and_fails_one_cycle_over_its_share() {
    let traces = [
        (
            tick(FramesPerTick::Six, Monitoring::Off, Phase::Bypass),
            priced(&[(CAPTURE_SIX, 266), ("budget_tick", 100)]),
        ),
        (
            tick(FramesPerTick::Six, Monitoring::Off, Phase::On),
            priced(&[
                (CAPTURE_SIX, share_at_six(Block::CaptureRamp) + 1),
                ("budget_tick", 100),
            ]),
        ),
    ];
    let evaluation = evaluate(&traces);
    assert_eq!(
        cell_of(&evaluation, Block::CaptureRamp, 0),
        Cell::Over {
            measured: share_at_six(Block::CaptureRamp) + 1,
            share: share_at_six(Block::CaptureRamp)
        }
    );
    assert_eq!(evaluation.failures(), 1);
}

#[test]
fn the_monitor_mix_row_is_the_monitored_tick_minus_the_unmonitored_tick_of_the_same_phase() {
    let traces = [
        (
            tick(FramesPerTick::Six, Monitoring::Off, Phase::Bypass),
            priced(&[
                (CAPTURE_SIX, 200),
                (
                    "dsp::playback::bypass_out::<6, dsp::monitor::PlaybackAlone>",
                    100,
                ),
            ]),
        ),
        (
            tick(FramesPerTick::Six, Monitoring::BothInputs, Phase::Bypass),
            priced(&[
                (CAPTURE_SIX, 200),
                (
                    "dsp::playback::bypass_out::<6, dsp::monitor::WithMonitoredInputs>",
                    350,
                ),
            ]),
        ),
    ];
    let evaluation = evaluate(&traces);
    assert_eq!(
        cell_of(&evaluation, Block::MonitorMix(TickMode::Bypass), 0),
        Cell::Within {
            measured: 250,
            share: share_at_six(Block::MonitorMix(TickMode::Bypass))
        }
    );
    assert_eq!(
        cell_of(&evaluation, Block::BypassOut, 0),
        Cell::Within {
            measured: 100,
            share: share_at_six(Block::BypassOut)
        }
    );
    assert_eq!(
        cell_of(&evaluation, Block::DspWorstTick, 0),
        Cell::Within {
            measured: 550,
            share: 6_250
        }
    );
}

#[test]
fn a_block_that_runs_where_d4_turns_it_off_fails() {
    let traces = [(
        tick(FramesPerTick::TwentyFour, Monitoring::Off, Phase::On),
        priced(&[("dsp::biquad::filter_band::<24, 30>", 10)]),
    )];
    let evaluation = evaluate(&traces);
    assert_eq!(
        cell_of(&evaluation, Block::OnPeq, 2),
        Cell::RunsWhereD4TurnsItOff { measured: 10 }
    );
    assert_eq!(
        cell_of(&evaluation, Block::OnPeq, 0),
        Cell::Within {
            measured: 0,
            share: share_at_six(Block::OnPeq)
        }
    );
    assert_eq!(evaluation.failures(), 1);
}

#[test]
fn a_function_no_d3_row_owns_is_a_failure_even_when_every_row_is_within_its_share() {
    let traces = [(
        tick(FramesPerTick::Twelve, Monitoring::Off, Phase::Bypass),
        priced(&[("mystery_helper", 1)]),
    )];
    let evaluation = evaluate(&traces);
    assert_eq!(evaluation.unattributed.len(), 1);
    assert_eq!(evaluation.failures(), 1);
}

#[test]
fn the_usb_interrupt_is_entry_and_exit_plus_its_one_microframe_path_and_joins_the_tick_total_gate()
{
    let traces = [
        (
            Run::BenchUsb(FramesPerTick::Six, UsbPath::Microframe),
            priced(&[("budget_usb_event", 600)]),
        ),
        (
            Run::BenchUsb(FramesPerTick::Six, UsbPath::ControlTransfer),
            priced(&[("budget_usb_event", 1_400)]),
        ),
        (
            tick(FramesPerTick::Six, Monitoring::Off, Phase::Bypass),
            priced(&[
                (CAPTURE_SIX, 200),
                ("budget_tick", 100),
                ("<stream::transport::TickBlocks>::receive", 40),
                ("<stream::transport::TickBlocks>::transmit", 50),
            ]),
        ),
    ];
    let evaluation = evaluate(&traces);
    assert_eq!(
        cell_of(&evaluation, Block::DmaMoves, 0),
        Cell::Within {
            measured: 90,
            share: share_at_six(Block::DmaMoves)
        }
    );
    assert_eq!(
        cell_of(&evaluation, Block::UsbInterrupt, 0),
        Cell::Within {
            measured: 622,
            share: 1_500
        }
    );
    assert_eq!(
        cell_of(&evaluation, Block::TickAndUsb, 0),
        Cell::Within {
            measured: 300 + 22 + 90 + 622,
            share: 8_750
        }
    );
    assert!(matches!(
        cell_of(&evaluation, Block::Usb(UsbPath::ControlTransfer), 0),
        Cell::Within {
            measured: 1_400,
            ..
        } | Cell::Over {
            measured: 1_400,
            ..
        }
    ));
}

#[test]
fn a_usb_path_takes_its_worst_traced_event() {
    let traces = [
        (
            Run::BenchUsb(FramesPerTick::Twelve, UsbPath::Microframe),
            priced(&[("budget_usb_event", 40)]),
        ),
        (
            Run::BenchUsb(FramesPerTick::Twelve, UsbPath::Microframe),
            priced(&[("budget_usb_event", 90)]),
        ),
        (
            Run::BenchUsb(FramesPerTick::Twelve, UsbPath::Microframe),
            priced(&[("budget_usb_event", 60)]),
        ),
    ];
    let evaluation = evaluate(&traces);
    assert_eq!(evaluation.usb[1].cycles, 90);
}
