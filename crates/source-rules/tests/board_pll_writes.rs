use std::fs;
use std::path::{Path, PathBuf};

const PLL_DIVIDER_REQUEST_FIELDS: [&str; 4] = ["_NREQ", "_PREQ", "_MREQ", "_MD_REQ"];

fn board_firmware() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("source-rules sits two folders below the root")
        .join("crates/board/src/firmware")
}

fn board_sources() -> Vec<(String, String)> {
    fs::read_dir(board_firmware())
        .expect("board firmware sources are readable")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension() == Some("rs".as_ref()))
        .map(|path| {
            (
                path.file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .to_string(),
                fs::read_to_string(&path).expect("source is readable"),
            )
        })
        .collect()
}

fn pll_registers_named_in(source: &str) -> impl Iterator<Item = &str> {
    source.match_indices("SYSCON_PLL").map(move |(at, _)| {
        let rest = &source[at..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        &rest[..end]
    })
}

#[test]
fn the_board_follows_factory_and_never_sets_a_pll_divider_request_bit() {
    for (file, source) in board_sources() {
        for register in pll_registers_named_in(&source) {
            assert!(
                !PLL_DIVIDER_REQUEST_FIELDS
                    .iter()
                    .any(|field| register.ends_with(field)),
                "{file} names {register}, a request bit the factory configuration never sets"
            );
        }
    }
}

#[test]
fn every_pll_register_write_is_a_full_word_that_ignores_what_the_boot_rom_left() {
    for (file, source) in board_sources() {
        for register in pll_registers_named_in(&source) {
            for leftover_read in [".read()", ".set("] {
                assert!(
                    !source.contains(&format!("{register}{leftover_read}")),
                    "{file} keeps bits of {register} from before the write"
                );
            }
        }
    }
}
