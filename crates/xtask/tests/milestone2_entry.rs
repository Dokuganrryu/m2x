use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use xtask::firmware::TARGET as FIRMWARE_TARGET;
const IMAGE_BASE: u32 = 0x2000_0000;
const FIXED_ENTRY_OFFSET: u32 = 0x200;
const THUMB_BIT: u32 = 1;

fn run(command: &mut Command) -> String {
    let output = command.output().expect("the tool starts");
    assert!(
        output.status.success(),
        "{command:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn llvm_tool(name: &str) -> PathBuf {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let ask = |request: &str| {
        run(Command::new(&rustc).args(["--print", request]))
            .trim()
            .to_string()
    };
    PathBuf::from(ask("sysroot"))
        .join("lib/rustlib")
        .join(ask("host-tuple"))
        .join("bin")
        .join(format!("{name}{}", env::consts::EXE_SUFFIX))
}

fn built(feature: &str, folder: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = root.join("target").join(folder);
    run(
        Command::new(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
            .current_dir(&root)
            .args(["build", "--release", "--package", "board", "--bin", "board"])
            .args([
                "--features",
                feature,
                "--target",
                FIRMWARE_TARGET,
                "--target-dir",
            ])
            .arg(&target_dir),
    );
    target_dir
        .join(FIRMWARE_TARGET)
        .join("release")
        .join("board")
}

fn built_milestone2() -> PathBuf {
    built("milestone2", "milestone2-entry")
}

fn symbol(listing: &str, name: &str) -> u32 {
    listing
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            let address = words.next()?;
            let _kind = words.next()?;
            (words.next()? == name && words.next().is_none())
                .then(|| u32::from_str_radix(address, 16).ok())
                .flatten()
        })
        .unwrap_or_else(|| panic!("the image has no `{name}`"))
}

fn word(image: &[u8], offset: u32) -> u32 {
    let at = offset as usize;
    u32::from_le_bytes(image[at..at + 4].try_into().expect("four bytes"))
}

#[test]
fn the_milestone2_image_enters_the_same_reset_code_from_its_vector_and_from_offset_0x200() {
    let elf = built_milestone2();
    let symbols = run(Command::new(llvm_tool("llvm-nm")).arg(&elf));
    let flat = elf.with_extension("flat.bin");
    run(Command::new(llvm_tool("llvm-objcopy"))
        .args(["-O", "binary"])
        .arg(&elf)
        .arg(&flat));
    let image = fs::read(&flat).expect("the flat image is readable");

    let reset = symbol(&symbols, "Reset") & !THUMB_BIT;
    assert_eq!(symbol(&symbols, "__vector_table"), IMAGE_BASE);
    assert_eq!(
        reset,
        IMAGE_BASE + FIXED_ENTRY_OFFSET,
        "Reset is not at image offset 0x200"
    );
    assert_eq!(
        word(&image, 4),
        IMAGE_BASE + FIXED_ENTRY_OFFSET + THUMB_BIT,
        "the vector table's reset entry does not point at offset 0x200"
    );
    assert!(
        word(&image, FIXED_ENTRY_OFFSET) != 0,
        "offset 0x200 holds no code"
    );
}

const MAIN_CLOCK_SELECT_A: &str = "SYSCON.MAINCLKSELA";
const MAIN_CLOCK_SELECT_B: &str = "SYSCON.MAINCLKSELB";
const FRO_CONTROL: &str = "ANACTRL.FRO192M_CTRL";
const FRO_96_MHZ_ENABLE: u32 = 1 << 30;
const FRO_96_MHZ_SELECT: u32 = 3;

struct Line {
    at: f64,
    text: String,
}

fn emulate(elf: &Path, selectors: (u32, u32)) -> Vec<Line> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let python = xtask::budget::python_for(env::var_os(xtask::budget::PYTHON_VARIABLE), &root)
        .expect("a Python with unicorn, from M2X_PYTHON or .venv");
    let plan = format!(
        "load {}\nentry 0x20000200\nbreadcrumb 0x2002d1a0\ncrystal ready\nhost frames\nmain-clock {} {}\nrun 1000\n",
        elf.display(),
        selectors.0,
        selectors.1
    );
    let mut child = Command::new(python)
        .arg("-I")
        .arg(root.join("crates/xtask/board-model/board_model.py"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the board model starts");
    std::io::Write::write_all(&mut child.stdin.take().expect("stdin"), plan.as_bytes())
        .expect("the plan is written");
    let output = child.wait_with_output().expect("the board model ends");
    assert!(output.status.success(), "the board model failed");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (at, text) = line.split_once(' ')?;
            Some(Line {
                at: at.parse().ok()?,
                text: text.to_string(),
            })
        })
        .collect()
}

fn writes_to<'a>(trace: &'a [Line], register: &'a str) -> impl Iterator<Item = (f64, u32)> + 'a {
    trace.iter().filter_map(move |line| {
        let value = line.text.strip_prefix("write ")?.strip_prefix(register)?;
        let value = value.trim().strip_prefix("0x")?;
        Some((line.at, u32::from_str_radix(value, 16).ok()?))
    })
}

fn assert_the_mpu_is_never_enabled(trace: &[Line]) {
    let last_flash_read = trace
        .iter()
        .filter(|line| line.text.starts_with("flash-read"))
        .map(|line| line.at)
        .fold(f64::MIN, f64::max);
    let written: Vec<(f64, &str)> = trace
        .iter()
        .filter(|line| line.text.starts_with("mpu enable") || line.text == "mpu disable")
        .map(|line| (line.at, line.text.as_str()))
        .collect();
    assert!(
        written
            .iter()
            .all(|(at, text)| *text == "mpu disable" && *at <= last_flash_read),
        "MPU_CTRL is written outside the factory read's save and restore: {written:?}"
    );
    assert_eq!(written.len(), 2, "the factory read's save and restore only");
}

fn first(trace: &[Line], text: &str) -> f64 {
    trace
        .iter()
        .find(|line| line.text == text)
        .unwrap_or_else(|| panic!("the trace has no `{text}`"))
        .at
}

#[test]
fn a_core_handed_over_on_the_12_mhz_fro_is_on_the_96_mhz_fro_before_usb_connects() {
    let trace = emulate(&built_milestone2(), (0, 0));
    assert_the_mpu_is_never_enabled(&trace);
    let selects: Vec<(f64, u32)> = writes_to(&trace, MAIN_CLOCK_SELECT_A).collect();
    assert_eq!(
        selects.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
        [FRO_96_MHZ_SELECT],
        "exactly one main clock switch, to the 96 MHz FRO"
    );
    let switched = selects[0].0;
    assert_eq!(writes_to(&trace, MAIN_CLOCK_SELECT_B).count(), 0);
    assert!(
        writes_to(&trace, FRO_CONTROL)
            .any(|(at, value)| at <= switched && value & FRO_96_MHZ_ENABLE != 0),
        "the 96 MHz output is enabled before it is selected"
    );
    assert!(first(&trace, "clock main 96000000") < first(&trace, "usb connect"));
    assert!(
        trace
            .iter()
            .filter(|line| line.text.starts_with("flash-read"))
            .all(|line| line.at < switched),
        "every flash read comes before the switch"
    );
}

#[test]
fn a_core_handed_over_on_any_other_clock_keeps_its_selectors() {
    for selectors in [(1, 2), (3, 0), (1, 0), (0, 1)] {
        let trace = emulate(&built_milestone2(), selectors);
        assert_the_mpu_is_never_enabled(&trace);
        assert_eq!(
            writes_to(&trace, MAIN_CLOCK_SELECT_A).count()
                + writes_to(&trace, MAIN_CLOCK_SELECT_B).count(),
            0,
            "hand-over {selectors:?}"
        );
        first(&trace, "usb connect");
    }
}

const SCB_CCR_ADDRESS: u32 = 0xE000_ED14;
const HAND_OVER_ROW_BYTES: u32 = 12;

fn instructions(elf: &Path, from: u32, to: u32) -> Vec<String> {
    run(Command::new(llvm_tool("llvm-objdump"))
        .arg("-d")
        .arg(format!("--start-address={from:#x}"))
        .arg(format!("--stop-address={to:#x}"))
        .arg(elf))
    .lines()
    .filter_map(|line| {
        let (address, rest) = line.trim().split_once(':')?;
        (address.len() == 8).then_some(())?;
        u32::from_str_radix(address, 16).ok()?;
        let mut fields = rest
            .split(char::from(9))
            .map(str::trim)
            .filter(|field| !field.is_empty());
        let _bytes = fields.next()?;
        Some(fields.collect::<Vec<_>>().join(" "))
    })
    .collect()
}

fn hand_over_rows(image: &[u8], table: u32) -> Vec<u32> {
    let mut rows = Vec::new();
    let mut at = table - IMAGE_BASE;
    while word(image, at) != 0 {
        rows.push(word(image, at));
        at += HAND_OVER_ROW_BYTES;
    }
    rows
}

fn table_of(image: &[u8], pre_init_code: &[String]) -> u32 {
    pre_init_code
        .iter()
        .find_map(|instruction| {
            let literal = instruction
                .strip_prefix("ldr r0, [pc, ")?
                .split("@ 0x")
                .nth(1)?;
            u32::from_str_radix(literal.split_whitespace().next()?, 16).ok()
        })
        .map(|literal| word(image, literal - IMAGE_BASE))
        .expect("__pre_init loads its table address from a literal")
}

#[test]
fn the_milestone2_nolimit_image_clears_the_stack_limit_first_and_runs_no_ccr_row() {
    let elf = built("milestone2-nolimit", "milestone2-nolimit-entry");
    let symbols = run(Command::new(llvm_tool("llvm-nm")).arg(&elf));
    let flat = elf.with_extension("flat.bin");
    run(Command::new(llvm_tool("llvm-objcopy"))
        .args(["-O", "binary"])
        .arg(&elf)
        .arg(&flat));
    let image = fs::read(&flat).expect("the flat image is readable");
    let reset = symbol(&symbols, "Reset") & !THUMB_BIT;
    let pre_init = symbol(&symbols, "__pre_init") & !THUMB_BIT;
    assert_eq!(reset, IMAGE_BASE + FIXED_ENTRY_OFFSET);

    let reset_code = instructions(&elf, reset, pre_init);
    let limit_set = reset_code
        .iter()
        .position(|instruction| instruction == "msr msplim, r0")
        .expect("cortex-m-rt's Reset sets MSPLIM");
    assert!(
        reset_code[limit_set + 1].starts_with("bl ")
            && reset_code[limit_set + 1].contains("<__pre_init>"),
        "Reset calls __pre_init right after it sets the limit: {reset_code:?}"
    );
    let pre_init_code = instructions(&elf, pre_init, pre_init + 0x20);
    assert_eq!(
        pre_init_code[..2],
        ["movs r0, #0x0", "msr msplim, r0"],
        "__pre_init clears the limit first: {pre_init_code:?}"
    );

    let rows = hand_over_rows(&image, table_of(&image, &pre_init_code));

    let limited = built_milestone2();
    let limited_symbols = run(Command::new(llvm_tool("llvm-nm")).arg(&limited));
    let limited_flat = limited.with_extension("flat.bin");
    run(Command::new(llvm_tool("llvm-objcopy"))
        .args(["-O", "binary"])
        .arg(&limited)
        .arg(&limited_flat));
    let limited_image = fs::read(&limited_flat).expect("the milestone2 flat image is readable");
    let limited_pre_init = symbol(&limited_symbols, "__pre_init") & !THUMB_BIT;
    let limited_rows = hand_over_rows(
        &limited_image,
        table_of(
            &limited_image,
            &instructions(&limited, limited_pre_init, limited_pre_init + 0x20),
        ),
    );
    assert_eq!(
        limited_rows[0], SCB_CCR_ADDRESS,
        "milestone2's table opens with SCB_CCR"
    );
    assert_eq!(
        rows,
        limited_rows[1..],
        "the milestone2 table less its SCB_CCR row"
    );
    assert!(!rows.contains(&SCB_CCR_ADDRESS), "the table writes SCB_CCR");
}
