use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const CORE_PREFIXES: [&str; 4] = ["SCB_", "MPU_", "NVIC_", "SYST_"];
const ARM_CORE_DEBUG_SOURCE: &str = "fixtures/arm-core-debug-registers.md";
const ARM_CORE_DEBUG_REGISTERS: [(&str, u64); 3] = [
    ("DEMCR", 0xE000_EDFC),
    ("DWT_CTRL", 0xE000_1000),
    ("DWT_CYCCNT", 0xE000_1004),
];
const ARM_CORE_DEBUG_BITS: [(&str, u32); 3] = [
    ("DEMCR_TRCENA", 24),
    ("DWT_CTRL_CYCCNTENA", 0),
    ("DWT_CTRL_NOCYCCNT", 25),
];

fn arm_core_debug_register(name: &str) -> Option<u64> {
    ARM_CORE_DEBUG_REGISTERS
        .iter()
        .find(|(listed, _)| *listed == name)
        .map(|&(_, address)| address)
}

fn arm_core_debug_bit(name: &str) -> Option<u32> {
    ARM_CORE_DEBUG_BITS
        .iter()
        .find(|(listed, _)| *listed == name)
        .map(|&(_, position)| position)
}
const RESEARCH_NOTE_BITS: [&str; 2] = ["DMA0_FC", "USB1_RAM_"];
const USB_RAM_PREFIX: &str = "USB1_RAM_";
const USB_RAM: std::ops::RangeInclusive<u64> = 0x4010_0000..=0x4010_3FFF;
const PRIVATE_PERIPHERAL_BUS: std::ops::RangeInclusive<u64> = 0xE000_0000..=0xE00F_FFFF;

struct Row {
    name: String,
    cluster_name: Option<String>,
    address: u64,
    steps: Vec<u64>,
    fields: HashMap<String, (u32, u32)>,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("source-rules sits two folders below the root")
        .to_path_buf()
}

fn board_firmware() -> PathBuf {
    repo_root().join("crates/board/src/firmware")
}

fn number(text: &str) -> u64 {
    let text = text.replace('_', "");
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).expect("hex number"),
        None => text.parse().expect("decimal number"),
    }
}

fn leading_number(text: &str) -> &str {
    let end = text
        .char_indices()
        .find(|&(at, c)| !(c.is_ascii_hexdigit() || (at == 1 && c == 'x')))
        .map_or(text.len(), |(at, _)| at);
    &text[..end]
}

fn register_name(svd_name: &str) -> String {
    svd_name
        .split(['[', '('])
        .next()
        .expect("split yields one part")
        .replace("%s", "")
}

fn cluster_register_name(row_line: &str, svd_name: &str) -> Option<String> {
    let (_, after_cluster) = row_line.split_once(").")?;
    let member = after_cluster.split_whitespace().next()?;
    Some(format!(
        "{}_{}",
        register_name(svd_name),
        register_name(member)
    ))
}

fn rows() -> Vec<Row> {
    let text = fs::read_to_string(repo_root().join("fixtures/lpc55s28-svd-registers.txt"))
        .expect("the SVD rows are committed");
    let mut rows: Vec<Row> = Vec::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["REG", peripheral, name, ..] => {
                let address = words
                    .windows(2)
                    .find(|pair| pair[1] == "offset")
                    .map(|pair| number(pair[0]))
                    .expect("a register row has an address before its offset");
                let steps = words
                    .windows(2)
                    .filter(|pair| pair[0] == "step")
                    .map(|pair| number(leading_number(pair[1])))
                    .collect();
                rows.push(Row {
                    name: format!("{peripheral}_{}", register_name(name)),
                    cluster_name: cluster_register_name(line, name)
                        .map(|member| format!("{peripheral}_{member}")),
                    address,
                    steps,
                    fields: HashMap::new(),
                });
            }
            ["FIELD", field, bits, ..] => {
                let bits = bits.trim_matches(['[', ']']);
                let (high, low) = bits.split_once(':').unwrap_or((bits, bits));
                let low: u32 = low.parse().expect("field low bit");
                let high: u32 = high.parse().expect("field high bit");
                rows.last_mut()
                    .expect("a field follows its register")
                    .fields
                    .insert((*field).to_string(), (low, high - low + 1));
            }
            _ => {}
        }
    }
    rows
}

fn constants(source: &str, kind: &str) -> Vec<(String, String)> {
    source
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("pub const ")?;
            let (name, rest) = rest.split_once(": ")?;
            let value = rest.strip_prefix(kind)?.strip_prefix(" = ")?;
            Some((name.to_string(), value.trim_end_matches(';').to_string()))
        })
        .collect()
}

fn map_source() -> String {
    fs::read_to_string(board_firmware().join("map.rs")).expect("the board register map exists")
}

fn row_for_register<'a>(rows: &'a [Row], name: &str) -> Option<&'a Row> {
    rows.iter()
        .find(|row| row.name == name || row.cluster_name.as_deref() == Some(name))
        .or_else(|| rows.iter().find(|row| format!("{}0", row.name) == name))
}

#[test]
fn the_clock_gate_step_from_index_to_set_register_is_the_counted_one() {
    let source =
        fs::read_to_string(board_firmware().join("clock_gates.rs")).expect("the clock gates exist");
    let steps = lines_naming(&source, &["bytes_after("]);
    assert_eq!(
        steps,
        [
            "SYSCON_AHBCLKCTRL0.bytes_after(index * SYSCON_AHBCLKCTRLSET_INDEX_STEP),",
            "SYSCON_AHBCLKCTRLSET0.bytes_after(index * SYSCON_AHBCLKCTRLSET_INDEX_STEP),",
        ],
        "clock_gates.rs steps from an index to a status register and to a set register"
    );
    let map = constants(&map_source(), "usize");
    let step = map
        .iter()
        .find(|(name, _)| name == "SYSCON_AHBCLKCTRLSET_INDEX_STEP")
        .map(|(_, value)| number(value))
        .expect("the map names the set register step");
    assert_eq!(step, 4, "one 32-bit set register follows another");
    let registers = constants(&map_source(), "Register");
    let address = |wanted: &str| {
        registers
            .iter()
            .find(|(name, _)| name == wanted)
            .map(|(_, value)| {
                number(
                    value
                        .strip_prefix("Register::at(")
                        .and_then(|rest| rest.strip_suffix(')'))
                        .expect("a register is built with Register::at"),
                )
            })
            .unwrap_or_else(|| panic!("the map names {wanted}"))
    };
    assert_eq!(
        address("SYSCON_AHBCLKCTRL2") - address("SYSCON_AHBCLKCTRL0"),
        2 * step,
        "the status registers step as the set registers do"
    );
}

#[test]
fn every_peripheral_register_in_the_board_map_sits_at_its_t23_svd_address() {
    let rows = rows();
    let registers = constants(&map_source(), "Register");
    assert!(!registers.is_empty());
    for (name, value) in registers {
        let address = number(
            value
                .strip_prefix("Register::at(")
                .and_then(|rest| rest.strip_suffix(')'))
                .expect("a register is built with Register::at"),
        );
        if CORE_PREFIXES.iter().any(|prefix| name.starts_with(prefix)) {
            assert!(
                PRIVATE_PERIPHERAL_BUS.contains(&address),
                "{name} is a core register outside the private peripheral bus"
            );
            continue;
        }
        if let Some(listed) = arm_core_debug_register(&name) {
            assert_eq!(
                listed, address,
                "{name} address against Arm core debug list"
            );
            continue;
        }
        if name.starts_with(USB_RAM_PREFIX) {
            assert!(
                USB_RAM.contains(&address),
                "{name} is a USB RAM word outside the 16 KB of UM 44.1"
            );
            continue;
        }
        let row = row_for_register(&rows, &name)
            .unwrap_or_else(|| panic!("{name} has no row in the SVD register table"));
        assert_eq!(row.address, address, "{name} address");
    }
}

#[test]
fn every_arm_core_debug_entry_has_its_row_in_the_fixture() {
    let note = fs::read_to_string(repo_root().join(ARM_CORE_DEBUG_SOURCE))
        .expect("the fixture is readable");
    let row_for = |register: &str| {
        note.lines()
            .find(|line| line.contains(&format!("| `{register}` |")))
            .unwrap_or_else(|| panic!("{register} has no row in {ARM_CORE_DEBUG_SOURCE}"))
    };
    for (name, address) in ARM_CORE_DEBUG_REGISTERS {
        let hex = format!("{address:08X}");
        let written = format!("`0x{}_{}`", &hex[..4], &hex[4..]);
        assert!(row_for(name).contains(&written), "{name} at {written}");
    }
    for (name, position) in ARM_CORE_DEBUG_BITS {
        let (register, field) = ARM_CORE_DEBUG_REGISTERS
            .iter()
            .filter_map(|(register, _)| {
                Some((*register, name.strip_prefix(register)?.strip_prefix('_')?))
            })
            .max_by_key(|(register, _)| register.len())
            .unwrap_or_else(|| panic!("{name} belongs to no listed register"));
        assert!(
            row_for(register).contains(&format!("{position} `{field}`")),
            "{name} at bit {position}"
        );
    }
}

#[test]
fn every_peripheral_bit_in_the_board_map_is_a_one_bit_t23_svd_field_at_that_position() {
    let rows = rows();
    for (name, value) in constants(&map_source(), "u32") {
        let Some(position) = value.strip_prefix("1 << ") else {
            continue;
        };
        if CORE_PREFIXES
            .iter()
            .chain(RESEARCH_NOTE_BITS.iter())
            .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        let position: u32 = position.parse().expect("bit position");
        if let Some(listed) = arm_core_debug_bit(&name) {
            assert_eq!(listed, position, "{name} bit against Arm core debug list");
            continue;
        }
        let found = rows.iter().find_map(|row| {
            let field = row
                .cluster_name
                .as_ref()
                .and_then(|cluster| name.strip_prefix(&format!("{cluster}_")))
                .or_else(|| name.strip_prefix(&format!("{}_", row.name)))?;
            row.fields.get(field)
        });
        assert_eq!(
            found,
            Some(&(position, 1)),
            "{name} is not a one-bit field at {position}"
        );
    }
}

#[test]
fn every_array_step_in_the_board_map_matches_the_t23_svd_dimension_step() {
    let rows = rows();
    let steps = constants(&map_source(), "usize");
    let mut checked = 0;
    for (name, value) in steps {
        let (register, index) = if let Some(register) = name.strip_suffix("_PORT_STEP") {
            (register, 0)
        } else if let Some(register) = name.strip_suffix("_PIN_STEP") {
            (register, 1)
        } else if let Some(register) = name.strip_suffix("_INDEX_STEP") {
            (register, 0)
        } else {
            continue;
        };
        let row = row_for_register(&rows, register)
            .unwrap_or_else(|| panic!("{name} steps a register with no SVD row"));
        assert_eq!(row.steps.get(index), Some(&number(&value)), "{name}");
        checked += 1;
    }
    assert!(checked >= 3);
}

const SCAN_PLANTS_THAT_THE_IMAGE_SCAN_MUST_CATCH: &str = "image_plants.rs";

fn board_sources() -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut pending = vec![repo_root().join("crates/board/src")];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("board sources are readable") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension() == Some("rs".as_ref()) {
                let name = path
                    .file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .to_string();
                found.push((name, fs::read_to_string(&path).expect("source is readable")));
            }
        }
    }
    found
}

#[test]
fn only_mmio_touches_volatile_memory_or_core_instructions() {
    for (file, source) in board_sources() {
        if file == "mmio.rs" {
            continue;
        }
        for word in ["read_volatile", "write_volatile", "asm!"] {
            assert!(!source.contains(word), "{file} uses {word}");
        }
    }
}

#[test]
fn only_the_map_builds_a_register_or_a_memory_address() {
    for (file, source) in board_sources() {
        if file == "map.rs" || file == "mmio.rs" {
            continue;
        }
        for word in ["Register::at(", "MemoryBytes::at("] {
            assert!(
                !source.contains(word),
                "{file} builds an address with {word}"
            );
        }
    }
}

#[test]
fn no_board_source_outside_the_scan_plants_names_the_flash_controller_aoreg1_or_the_debug_mailbox()
{
    for (file, source) in board_sources() {
        if file == SCAN_PLANTS_THAT_THE_IMAGE_SCAN_MUST_CATCH {
            continue;
        }
        let plain = source.replace('_', "").to_ascii_lowercase();
        for alias in [
            "0x40034",
            "0x50034",
            "0x40020084",
            "0x50020084",
            "0x4009c0",
            "0x5009c0",
        ] {
            assert!(!plain.contains(alias), "{file} names {alias}");
        }
    }
}

fn dma_descriptor_files() -> Vec<(String, String)> {
    board_sources()
        .into_iter()
        .filter(|(_, source)| source.contains("source_end") || source.contains("destination_end"))
        .collect()
}

fn lines_naming<'a>(source: &'a str, words: &[&str]) -> Vec<&'a str> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| words.iter().any(|word| line.contains(word)))
        .collect()
}

#[test]
fn every_dma_end_address_comes_from_a_map_register_or_a_static_buffer() {
    let registers: Vec<String> = constants(&map_source(), "Register")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let is_map_register = |name: &str| registers.iter().any(|register| register == name);
    let mut files: Vec<String> = dma_descriptor_files()
        .into_iter()
        .map(|(file, _)| file)
        .collect();
    files.sort();
    assert_eq!(
        files,
        ["audio_dma.rs", "dma.rs", "lcd_link.rs"],
        "a new file fills a DMA descriptor; extend this rule to it"
    );
    for (file, source) in dma_descriptor_files() {
        for line in lines_naming(&source, &["source_end", "destination_end"]) {
            assert!(
                !line.contains(" as u32") && !line.contains("0x") && !line.contains('+'),
                "{file} forms a DMA end address in place: {line}"
            );
            if let Some((_, value)) = line.split_once("_end = ") {
                let value = value.trim_end_matches(';');
                let register = value.strip_suffix(".address()");
                assert!(
                    register.is_some_and(is_map_register)
                        || value == "channel.transfer_and_ends()"
                        || value == "match run.source {",
                    "{file} binds a DMA end to {value}"
                );
            }
        }
    }
    let audio = fs::read_to_string(board_firmware().join("audio_dma.rs")).expect("audio_dma.rs");
    assert!(audio.contains(
        "enum Direction {\n    Transmit(&'static DmaBuffer, Register),\n    Receive(Register, &'static DmaBuffer),\n}"
    ));
    for line in lines_naming(&audio, &["=> Direction::", "Direction::Receive(I2S"]) {
        let arguments = line
            .split_once("Direction::")
            .and_then(|(_, rest)| rest.split_once('('))
            .map(|(_, arguments)| arguments.trim_end_matches(&[')', ','][..]))
            .expect("a Direction constructor");
        for argument in arguments.split(", ") {
            assert!(
                is_map_register(argument)
                    || argument.starts_with("&AUDIO.")
                    || argument == "&RECEIVE_FROM_FC7_DRAINED",
                "audio_dma.rs points a channel at {argument}"
            );
        }
    }
    for line in lines_naming(&audio, &["last_word_address", "fifo."]) {
        assert!(
            ["buffer.last_word_address() as u32,", "fifo.address(),"].contains(&line),
            "audio_dma.rs forms a DMA end address as {line}"
        );
    }
    let lcd = fs::read_to_string(board_firmware().join("lcd_link.rs")).expect("lcd_link.rs");
    for line in lines_naming(&lcd, &["first_word:"]) {
        assert!(
            [
                "first_word: u32,",
                "first_word: words.as_ptr() as u32,",
                "first_word: core::ptr::from_ref(word) as u32,",
            ]
            .contains(&line),
            "lcd_link.rs starts a run at {line}"
        );
    }
}
