use xtask::scan::{GraphFinding, Input, InputError, InputErrorKind, SymbolFamily, Violation};
use xtask::scan_image::{Encoding, ImageBuild, ImagePlant, Level, Verdict, judge};

const PLANT: &str = "board::firmware::image_plants::name_the_controller_block";
const TICK_PLANT: &str = "board::firmware::image_plants::divide_in_tick";

const LISTING: &str = "board:\tfile format elf32-littlearm
Disassembly of section .text:
20000864 <board::firmware::image_plants::name_the_controller_block>:
20000868: 4801         \tldr\tr0, [pc, #0x4]          @ 0x20000870
2000086e: bd88         \tpop\t{r3, r7, pc}
20000870: 00 40 03 40  \t.word\t0x40034000
200007ba: f244 0000    \tmovw\tr0, #0x4000
200007be: f2c4 0003    \tmovt\tr0, #0x4003
";
const WORD_LINE: usize = 6;
const MOVT_LINE: usize = 8;

fn flash_constant(function: &str, line: usize) -> Violation {
    Violation::FlashControllerConstant {
        line,
        function: function.to_string(),
        value: 0x4003_4000,
    }
}

fn graph(function: &str, finding: GraphFinding) -> Violation {
    Violation::InInterruptGraph {
        root: "audio_tick_interrupt".to_string(),
        function: function.to_string(),
        line: 12,
        finding,
    }
}

fn judged(build: ImageBuild, violations: Vec<Violation>) -> Verdict {
    judge(build, &Ok(violations), LISTING)
}

#[test]
fn the_clean_image_holds_only_with_no_violation() {
    assert_eq!(
        judged(ImageBuild::Clean(Level::Speed), Vec::new()),
        Verdict::Holds
    );
    assert_eq!(
        judged(
            ImageBuild::Clean(Level::BoardForSize),
            vec![flash_constant("board::firmware::main", WORD_LINE)]
        ),
        Verdict::CleanImageFailed { violations: 1 }
    );
}

#[test]
fn a_refused_scan_never_holds_for_any_build() {
    let refusal = || {
        Err(InputError::whole(
            Input::Rules,
            InputErrorKind::RuleWithoutData(xtask::scan::RuleKey::InterruptRoot),
        ))
    };
    for build in [
        ImageBuild::Clean(Level::Speed),
        ImageBuild::Planted(ImagePlant::DivisionInTick),
    ] {
        assert!(matches!(
            judge(build, &refusal(), LISTING),
            Verdict::Refused(_)
        ));
    }
}

#[test]
fn a_plant_the_scan_lets_through_breaks_the_check() {
    for plant in ImagePlant::ALL {
        assert_eq!(
            judged(ImageBuild::Planted(plant), Vec::new()),
            Verdict::PlantPassed
        );
    }
}

#[test]
fn the_literal_pool_plant_holds_when_its_pool_word_fails_inside_the_plant() {
    assert_eq!(
        judged(
            ImageBuild::Planted(ImagePlant::LiteralPoolFlashController),
            vec![flash_constant(PLANT, WORD_LINE)]
        ),
        Verdict::Holds
    );
}

#[test]
fn the_literal_pool_plant_caught_only_as_a_movt_was_not_built_as_a_literal_pool() {
    assert_eq!(
        judged(
            ImageBuild::Planted(ImagePlant::LiteralPoolFlashController),
            vec![flash_constant(PLANT, MOVT_LINE)]
        ),
        Verdict::WrongEncoding(Encoding::LiteralPool)
    );
}

#[test]
fn the_movw_movt_plant_holds_only_when_caught_at_its_movt() {
    let plant = ImageBuild::Planted(ImagePlant::MovwMovtFlashController);
    assert_eq!(
        judged(plant, vec![flash_constant(PLANT, MOVT_LINE)]),
        Verdict::Holds
    );
    assert_eq!(
        judged(
            plant,
            vec![
                flash_constant(PLANT, MOVT_LINE),
                flash_constant(PLANT, WORD_LINE)
            ]
        ),
        Verdict::WrongEncoding(Encoding::MoveWideAndTop)
    );
}

#[test]
fn a_plant_that_also_trips_another_rule_does_not_prove_its_own_rule() {
    let extra = Violation::FlashDriverSymbol {
        line: 95,
        symbol: format!("{PLANT}_flash"),
    };
    assert_eq!(
        judged(
            ImageBuild::Planted(ImagePlant::MovwMovtFlashController),
            vec![flash_constant(PLANT, MOVT_LINE), extra.clone()]
        ),
        Verdict::FailedForAnotherReason(extra)
    );
}

#[test]
fn a_plant_whose_rule_fires_outside_the_planted_code_does_not_prove_the_plant() {
    let elsewhere = flash_constant("dsp::peq::series", MOVT_LINE);
    assert_eq!(
        judged(
            ImageBuild::Planted(ImagePlant::MovwMovtFlashController),
            vec![flash_constant(PLANT, MOVT_LINE), elsewhere.clone()]
        ),
        Verdict::FailedForAnotherReason(elsewhere)
    );
}

#[test]
fn the_symbol_plant_holds_when_only_the_planted_driver_name_fails() {
    assert_eq!(
        judged(
            ImageBuild::Planted(ImagePlant::FlashDriverSymbol),
            vec![Violation::FlashDriverSymbol {
                line: 84,
                symbol: "<board::firmware::image_plants::drivers::flash::FlashGordon>::erase_page"
                    .to_string(),
            }]
        ),
        Verdict::Holds
    );
}

#[test]
fn the_factory_plant_holds_when_the_magic_fails_outside_the_accessor() {
    assert_eq!(
        judged(
            ImageBuild::Planted(ImagePlant::FactoryReadOutsideAccessor),
            vec![Violation::FactoryAddressOutsideAccessor {
                line: 618,
                function:
                    "board::firmware::image_plants::name_the_factory_magic_outside_the_accessor"
                        .to_string(),
                value: 0x0003_8200,
            }]
        ),
        Verdict::Holds
    );
}

#[test]
fn the_division_plant_holds_only_on_a_division_finding() {
    let plant = ImageBuild::Planted(ImagePlant::DivisionInTick);
    assert_eq!(
        judged(
            plant,
            vec![graph(
                TICK_PLANT,
                GraphFinding::Division("udiv".to_string())
            )]
        ),
        Verdict::Holds
    );
    let panic = graph(TICK_PLANT, GraphFinding::Symbol(SymbolFamily::Panic));
    assert_eq!(
        judged(
            plant,
            vec![
                graph(TICK_PLANT, GraphFinding::Division("udiv".to_string())),
                panic.clone()
            ]
        ),
        Verdict::FailedForAnotherReason(panic)
    );
}

#[test]
fn only_the_literal_pool_plant_is_built_with_the_board_crate_at_the_size_level() {
    let levels: Vec<(ImagePlant, Level)> = ImagePlant::ALL
        .into_iter()
        .map(|plant| (plant, plant.level()))
        .collect();
    assert_eq!(
        levels,
        vec![
            (ImagePlant::LiteralPoolFlashController, Level::BoardForSize),
            (ImagePlant::MovwMovtFlashController, Level::Speed),
            (ImagePlant::FlashDriverSymbol, Level::Speed),
            (ImagePlant::FactoryReadOutsideAccessor, Level::Speed),
            (ImagePlant::DivisionInTick, Level::Speed),
            (ImagePlant::BootRequestRegister, Level::Speed),
            (ImagePlant::DebugMailbox, Level::Speed),
        ]
    );
}
