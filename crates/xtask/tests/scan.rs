use std::fs;
use std::path::PathBuf;
use xtask::scan::{
    GraphFinding, Input, InputError, InputErrorKind, Rule, RuleKey, SymbolFamily, Violation, scan,
};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("scan")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn scan_image_with(rules: &str, image: &str) -> Result<Vec<Violation>, InputError> {
    scan(
        &fixture(rules),
        &fixture(&format!("{image}.nm.txt")),
        &fixture(&format!("{image}.objdump.txt")),
    )
}

fn violations_of(image: &str) -> Vec<Violation> {
    scan_image_with("image.rules", image)
        .unwrap_or_else(|error| panic!("{image} is a readable listing: {error}"))
}

fn rules_hit(image: &str) -> Vec<Rule> {
    violations_of(image).iter().map(Violation::rule).collect()
}

fn refusal(rules: &str, image: &str) -> InputError {
    match scan_image_with(rules, image) {
        Err(error) => error,
        Ok(violations) => panic!("{rules} on {image} was scanned, not refused: {violations:?}"),
    }
}

#[test]
fn the_clean_image_at_the_speed_level_passes_every_rule() {
    assert_eq!(violations_of("clean-speed"), []);
}

#[test]
fn the_clean_image_at_the_size_level_passes_every_rule() {
    assert_eq!(violations_of("clean-size"), []);
}

#[test]
fn a_flash_controller_address_in_a_literal_pool_is_rejected() {
    let violations = violations_of("flash-controller-literal-pool");
    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::FlashControllerConstant {
                value: 0x4003_4010,
                ..
            }
        )),
        "{violations:?}"
    );
}

#[test]
fn a_flash_controller_address_formed_by_movw_and_movt_is_rejected() {
    let violations = violations_of("flash-controller-movw-movt");
    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::FlashControllerConstant { value: 0x4003_4010, function, .. }
                if function == "image::fill_ring"
        )),
        "{violations:?}"
    );
}

#[test]
fn movw_and_movt_interleaved_across_two_registers_still_form_the_flash_controller_address() {
    let violations = violations_of("flash-controller-interleaved");
    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::FlashControllerConstant {
                value: 0x4003_4020,
                ..
            }
        )),
        "{violations:?}"
    );
}

#[test]
fn the_secure_alias_of_the_flash_controller_is_rejected() {
    let violations = violations_of("flash-controller-secure-alias");
    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::FlashControllerConstant {
                value: 0x5003_4008,
                ..
            }
        )),
        "{violations:?}"
    );
}

#[test]
fn the_reset_request_register_in_a_literal_pool_or_built_by_movw_and_movt_is_rejected() {
    for image in ["reset-request-literal-pool", "reset-request-movw-movt"] {
        let violations = violations_of(image);
        assert_eq!(
            violations,
            [Violation::ResetRequestRegisterConstant {
                line: if image.ends_with("pool") { 62 } else { 60 },
                function: "image::fill_ring".to_string(),
                value: 0xE000_ED0C,
            }],
            "{image}"
        );
    }
}

#[test]
fn the_rom_flash_api_entry_named_by_the_rules_is_rejected() {
    let violations = violations_of("rom-flash-api-entry");
    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::RomFlashApiEntryConstant {
                value: 0xFEED_0001,
                ..
            }
        )),
        "{violations:?}"
    );
}

#[test]
fn a_factory_record_address_is_rejected_even_inside_no_accessor() {
    let violations = violations_of("factory-record-outside-accessor");
    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::FactoryPageConstant { value: 0x0003_8400, function, .. }
                if function == "Reset"
        )),
        "{violations:?}"
    );
}

#[test]
fn the_factory_magic_address_outside_the_accessor_is_rejected() {
    let violations = violations_of("factory-magic-outside-accessor");
    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::FactoryAddressOutsideAccessor { value: 0x0003_8200, function, .. }
                if function == "Reset"
        )),
        "{violations:?}"
    );
}

#[test]
fn the_accessor_reading_the_factory_magic_is_allowed_only_while_the_rules_name_it() {
    assert_eq!(violations_of("clean-speed"), []);
    let violations = scan_image_with("no-factory-accessor.rules", "clean-speed")
        .expect("the rules name no accessor on purpose");
    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::FactoryAddressOutsideAccessor { function, .. } if function == "factory_serial"
        )),
        "{violations:?}"
    );
}

#[test]
fn a_store_through_a_register_holding_the_factory_magic_is_rejected_inside_the_accessor() {
    let violations = violations_of("factory-write-in-accessor");
    assert_eq!(
        violations,
        [Violation::FactoryWrite {
            line: 81,
            function: "factory_serial".to_string(),
            address: 0x0003_8200,
        }]
    );
}

#[test]
fn a_store_through_a_register_loaded_from_the_literal_pool_with_the_factory_magic_is_rejected() {
    let violations = violations_of("factory-write-in-accessor-literal-pool");
    assert!(
        matches!(
            violations.as_slice(),
            [Violation::FactoryWrite { address: 0x0003_8200, function, .. }] if function == "factory_serial"
        ),
        "{violations:?}"
    );
}

#[test]
fn a_symbol_from_a_flash_driver_is_rejected() {
    let violations = violations_of("flash-driver-symbol");
    assert!(
        violations.iter().any(|violation| matches!(
            violation,
            Violation::FlashDriverSymbol { symbol, .. }
                if symbol == "image::lpc55_hal::drivers::flash::erase_page"
        )),
        "{violations:?}"
    );
}

#[test]
fn a_function_the_rules_expect_dead_stripped_is_rejected_when_it_survives_the_link() {
    assert_eq!(
        rules_hit("dead-stripped-symbol-kept"),
        [Rule::DeadStrippedSymbol]
    );
}

#[test]
fn a_register_zero_writer_is_rejected_in_the_normal_build() {
    assert_eq!(
        rules_hit("dac-register-zero-writer"),
        [Rule::DacRegisterZeroWriter]
    );
}

fn graph_findings(image: &str) -> Vec<GraphFinding> {
    violations_of(image)
        .into_iter()
        .filter_map(|violation| match violation {
            Violation::InInterruptGraph { finding, .. } => Some(finding),
            _ => None,
        })
        .collect()
}

#[test]
fn a_hardware_divide_in_the_tick_graph_is_rejected() {
    assert_eq!(
        graph_findings("udiv-in-tick"),
        [GraphFinding::Division("udiv".to_string())]
    );
}

#[test]
fn a_division_helper_in_the_tick_graph_is_rejected() {
    let findings = graph_findings("aeabi-div-in-tick");
    assert!(
        findings.contains(&GraphFinding::Symbol(SymbolFamily::DivisionHelper)),
        "{findings:?}"
    );
}

#[test]
fn a_floating_point_instruction_in_the_tick_graph_is_rejected() {
    let findings = graph_findings("vfp-in-tick");
    assert!(
        findings.contains(&GraphFinding::FloatingPoint("vldr".to_string())),
        "{findings:?}"
    );
}

#[test]
fn a_floating_point_instruction_in_the_tick_graph_also_fails_the_whole_image_rule() {
    let hits = rules_hit("vfp-in-tick");
    assert!(hits.contains(&Rule::InterruptGraph), "{hits:?}");
    assert_eq!(
        hits.iter()
            .filter(|rule| **rule == Rule::FloatingPointInstruction)
            .count(),
        6,
        "{hits:?}"
    );
}

#[test]
fn every_floating_point_instruction_outside_the_interrupt_graph_is_rejected() {
    let at = |line, mnemonic: &str| Violation::FloatingPointInstruction {
        line,
        function: "image::console::band_gain".to_string(),
        mnemonic: mnemonic.to_string(),
    };
    assert_eq!(
        violations_of("vfp-outside-interrupts"),
        [
            at(79, "vmov"),
            at(80, "vldr"),
            at(81, "vcvt.f32.u32"),
            at(82, "vmul.f32"),
            at(83, "vcvt.u32.f32"),
            at(84, "vmov"),
        ]
    );
}

#[test]
fn a_panic_path_in_the_tick_graph_is_rejected() {
    let findings = graph_findings("panic-in-tick");
    assert!(
        findings.contains(&GraphFinding::Symbol(SymbolFamily::Panic)),
        "{findings:?}"
    );
}

#[test]
fn formatting_in_the_tick_graph_is_rejected() {
    let findings = graph_findings("fmt-in-tick");
    assert!(
        findings.contains(&GraphFinding::Symbol(SymbolFamily::Formatting)),
        "{findings:?}"
    );
}

#[test]
fn interrupt_masking_in_the_tick_graph_is_rejected() {
    assert_eq!(
        graph_findings("masking-in-tick"),
        [GraphFinding::InterruptMasking("cpsid".to_string())]
    );
}

#[test]
fn a_critical_section_in_the_tick_graph_is_rejected() {
    assert_eq!(
        graph_findings("critical-section-in-tick"),
        [GraphFinding::Symbol(SymbolFamily::CriticalSection)]
    );
}

#[test]
fn a_libm_function_in_the_tick_graph_is_rejected() {
    let findings = graph_findings("libm-in-tick");
    assert!(
        findings.contains(&GraphFinding::Symbol(SymbolFamily::Libm)),
        "{findings:?}"
    );
}

#[test]
fn an_indirect_call_in_the_tick_graph_is_rejected_because_its_callee_cannot_be_checked() {
    assert_eq!(
        graph_findings("indirect-call-in-tick"),
        [GraphFinding::IndirectCall]
    );
}

#[test]
fn a_graph_finding_names_the_interrupt_root_it_was_reached_from() {
    let violations = violations_of("udiv-in-tick");
    assert!(
        matches!(
            violations.as_slice(),
            [Violation::InInterruptGraph { root, function, .. }]
                if root == "CTIMER0" && function == "image::mix_block"
        ),
        "{violations:?}"
    );
}

#[test]
fn rules_without_a_rom_flash_api_entry_are_refused() {
    let error = refusal("without-rom-flash-api-entry.rules", "clean-speed");
    assert_eq!(error.input, Input::Rules);
    assert_eq!(
        error.kind,
        InputErrorKind::RuleWithoutData(RuleKey::RomFlashApiEntry)
    );
}

#[test]
fn every_rule_key_left_without_data_is_refused() {
    for (rules, key) in [
        ("without-interrupt-roots.rules", RuleKey::InterruptRoot),
        ("without-factory-accessor.rules", RuleKey::FactoryAccessor),
        (
            "without-dac-register-zero-writer.rules",
            RuleKey::DacRegisterZeroWriter,
        ),
        ("without-dead-stripped.rules", RuleKey::DeadStripped),
    ] {
        assert_eq!(
            refusal(rules, "clean-speed").kind,
            InputErrorKind::RuleWithoutData(key),
            "{rules}"
        );
    }
}

#[test]
fn an_unknown_rule_key_is_refused_with_its_line() {
    let error = refusal("unknown-key.rules", "clean-speed");
    assert_eq!(
        (error.input, error.line, error.kind),
        (
            Input::Rules,
            Some(7),
            InputErrorKind::UnknownRuleKey("allow-flash-writes".to_string())
        )
    );
}

#[test]
fn an_unreadable_rom_flash_api_entry_is_refused() {
    let error = refusal("unreadable-rom-flash-api-entry.rules", "clean-speed");
    assert_eq!(
        error.kind,
        InputErrorKind::UnreadableAddress("FEED-0001".to_string())
    );
}

#[test]
fn an_interrupt_root_missing_from_the_listing_is_refused() {
    let error = refusal("root-not-in-listing.rules", "clean-speed");
    assert_eq!(error.kind, InputErrorKind::NotInListing("USB0".to_string()));
}

fn refusal_of_listing(nm: &str, objdump: &str) -> InputError {
    scan(&fixture("image.rules"), &fixture(nm), &fixture(objdump))
        .expect_err("the listing is refused")
}

#[test]
fn an_unrecognised_section_is_refused() {
    let error = refusal_of_listing("clean-size.nm.txt", "unrecognised-section.objdump.txt");
    assert_eq!(
        (error.input, error.kind),
        (
            Input::InstructionListing,
            InputErrorKind::UnrecognisedSection(".ramfunc".to_string())
        )
    );
}

#[test]
fn an_unreadable_instruction_listing_line_is_refused_with_its_line() {
    let error = refusal_of_listing("clean-speed.nm.txt", "unreadable-line.objdump.txt");
    assert_eq!(
        (error.input, error.line, error.kind),
        (
            Input::InstructionListing,
            Some(15),
            InputErrorKind::UnreadableLine
        )
    );
}

#[test]
fn an_unreadable_symbol_line_is_refused_with_its_line() {
    let error = refusal_of_listing("unreadable.nm.txt", "clean-speed.objdump.txt");
    assert_eq!(
        (error.input, error.line, error.kind),
        (Input::Symbols, Some(4), InputErrorKind::UnreadableLine)
    );
}

#[test]
fn a_listing_decoded_without_the_cortex_m33_is_refused_at_its_first_undecoded_word() {
    let error = refusal_of_listing("vfp-without-cpu.nm.txt", "vfp-without-cpu.objdump.txt");
    assert_eq!(error.kind, InputErrorKind::UndecodedInstruction);
}

#[test]
fn mangled_listings_are_refused_because_the_symbol_rules_read_demangled_paths() {
    let symbols = refusal_of_listing("clean-speed-mangled.nm.txt", "clean-speed.objdump.txt");
    assert_eq!(symbols.input, Input::Symbols);
    assert!(matches!(symbols.kind, InputErrorKind::MangledName(_)));
    let instruction_listing =
        refusal_of_listing("clean-speed.nm.txt", "clean-speed-mangled.objdump.txt");
    assert_eq!(instruction_listing.input, Input::InstructionListing);
    assert!(matches!(
        instruction_listing.kind,
        InputErrorKind::MangledName(_)
    ));
}

#[test]
fn an_interrupt_graph_reaching_a_function_whose_body_is_missing_is_refused() {
    let error = refusal_of_listing("clean-speed.nm.txt", "missing-callee-body.objdump.txt");
    assert_eq!(
        error.kind,
        InputErrorKind::NotInListing("image::fill_ring".to_string())
    );
}

#[test]
fn a_symbol_table_given_as_the_instruction_listing_is_refused() {
    let error = refusal_of_listing("clean-speed.nm.txt", "clean-speed.nm.txt");
    assert_eq!(
        (error.input, error.kind),
        (Input::InstructionListing, InputErrorKind::NotAnArmListing)
    );
}

#[test]
fn empty_inputs_are_refused() {
    assert_eq!(
        scan(
            &fixture("image.rules"),
            "",
            &fixture("clean-speed.objdump.txt")
        )
        .expect_err("no symbols")
        .kind,
        InputErrorKind::Empty
    );
    assert_eq!(
        scan(&fixture("image.rules"), &fixture("clean-speed.nm.txt"), "")
            .expect_err("no instruction_listing")
            .kind,
        InputErrorKind::NotAnArmListing
    );
}
