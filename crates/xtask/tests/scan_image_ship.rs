use std::fs;
use std::path::{Path, PathBuf};
use xtask::scan_image::ImagePlant;

const FACTORY_MAGIC_HEX_DIGITS: &str = "38200";
const STATEMENTS_THE_RULES_MAY_MAKE_WITH_NONE: [&str; 2] =
    ["factory-accessor none", "dac-register-zero-writer none"];
const PLACEHOLDER_WORDS: [&str; 5] = ["placeholder", "todo", "tbd", "pending", "0xfeed0001"];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn committed_rules() -> String {
    read(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("rules")
            .join("m2x-image.rules"),
    )
}

fn rules_state(statement: &str) -> bool {
    committed_rules()
        .lines()
        .any(|line| line.trim() == statement)
}

fn rust_sources_under(folder: &Path) -> Vec<(PathBuf, String)> {
    let mut sources = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).expect("a readable folder") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let text = read(&path);
                sources.push((path, text));
            }
        }
    }
    sources
}

#[test]
fn register_zero_writer_none_holds_only_while_no_crate_declares_a_cargo_feature() {
    if !rules_state("dac-register-zero-writer none") {
        return;
    }
    let crates_with_features: Vec<String> = fs::read_dir(workspace().join("crates"))
        .expect("the crates folder")
        .map(|entry| entry.expect("a crate entry").path().join("Cargo.toml"))
        .filter(|manifest| manifest.is_file())
        .filter(|manifest| {
            read(manifest)
                .lines()
                .any(|line| line.trim() == "[features]")
        })
        .map(|manifest| manifest.display().to_string())
        .collect();
    assert_eq!(
        crates_with_features,
        Vec::<String>::new(),
        "the register-0 protocol build of is a cargo feature; replace `dac-register-zero-writer none` with the path of its writer"
    );
}

#[test]
fn factory_accessor_none_holds_only_while_no_board_source_names_the_factory_magic() {
    if !rules_state("factory-accessor none") {
        return;
    }
    let naming: Vec<String> =
        rust_sources_under(&workspace().join("crates").join("board").join("src"))
            .into_iter()
            .filter(|(path, _)| !path.ends_with("image_plants.rs"))
            .filter(|(_, text)| {
                text.to_ascii_lowercase()
                    .replace('_', "")
                    .contains(FACTORY_MAGIC_HEX_DIGITS)
            })
            .map(|(path, _)| path.display().to_string())
            .collect();
    assert_eq!(
        naming,
        Vec::<String>::new(),
        "the factory accessor has landed; replace `factory-accessor none` with its symbol"
    );
}

#[test]
fn the_committed_rules_hold_no_placeholder_and_say_none_only_where_the_format_means_it() {
    let suspicious: Vec<String> = committed_rules()
        .lines()
        .map(str::trim)
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            PLACEHOLDER_WORDS.iter().any(|word| lower.contains(word))
                || (line.ends_with(" none")
                    && !STATEMENTS_THE_RULES_MAY_MAKE_WITH_NONE.contains(line))
        })
        .map(str::to_string)
        .collect();
    assert_eq!(suspicious, Vec::<String>::new());
}

#[test]
fn the_committed_rules_name_the_two_handlers_the_vector_table_holds() {
    let roots: Vec<String> = committed_rules()
        .lines()
        .filter_map(|line| line.strip_prefix("interrupt-root "))
        .map(str::to_string)
        .collect();
    assert_eq!(roots, vec!["audio_tick_interrupt", "usb_interrupt"]);
    let interrupts = read(
        &workspace()
            .join("crates")
            .join("board")
            .join("src")
            .join("firmware")
            .join("interrupts.rs"),
    );
    for root in &roots {
        assert!(
            interrupts.contains(&format!("table[CTIMER0_IRQ] = {root};"))
                || interrupts.contains(&format!("table[USB1_IRQ] = {root};")),
            "{root} sits in the device vector table"
        );
    }
}

#[test]
fn every_image_plant_is_declared_to_check_cfg_and_planted_in_its_own_file() {
    let board = workspace().join("crates").join("board");
    let build_script = read(&board.join("build.rs"));
    let plants = read(&board.join("src").join("firmware").join("image_plants.rs"));
    for plant in ImagePlant::ALL {
        let value = plant.cfg_value();
        assert!(
            build_script.contains(&format!("\\\"{value}\\\"")),
            "build.rs declares {value}"
        );
        assert!(
            plants.contains(&format!("image_plant = \"{value}\"")),
            "image_plants.rs plants {value}"
        );
    }
}
