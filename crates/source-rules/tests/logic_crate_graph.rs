use std::fs;
use std::path::{Path, PathBuf};

const LOGIC_CRATES: [&str; 3] = ["dsp", "stream", "device"];

fn crate_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("source-rules sits inside crates")
        .join(name)
}

fn dependency_names(manifest: &str) -> Vec<String> {
    let mut in_dependencies = false;
    let mut names = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]"
                || line == "[build-dependencies]"
                || (line.starts_with("[target.") && line.ends_with(".dependencies]"));
            continue;
        }
        if in_dependencies && let Some((name, _)) = line.split_once('=') {
            names.push(name.trim().to_string());
        }
    }
    names
}

fn rust_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("crate directory is readable") {
        let path = entry.expect("directory entry is readable").path();
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension() == Some("rs".as_ref()) {
            found.push(path);
        }
    }
}

#[test]
fn logic_crates_depend_on_nothing_but_each_other() {
    for logic in LOGIC_CRATES {
        let manifest = fs::read_to_string(crate_dir(logic).join("Cargo.toml"))
            .expect("every logic crate has a manifest");
        for dependency in dependency_names(&manifest) {
            assert!(
                LOGIC_CRATES.contains(&dependency.as_str()),
                "{logic} depends on {dependency}"
            );
        }
    }
}

#[test]
fn logic_crate_libraries_are_no_std_and_forbid_unsafe_code() {
    for logic in LOGIC_CRATES {
        let lib = fs::read_to_string(crate_dir(logic).join("src").join("lib.rs"))
            .expect("every logic crate has a lib.rs");
        assert!(lib.contains("#![no_std]"), "{logic} is not no_std");
        assert!(
            lib.contains("#![forbid(unsafe_code)]"),
            "{logic} does not forbid unsafe code"
        );
    }
}

#[test]
fn no_logic_crate_source_names_the_alloc_crate() {
    for logic in LOGIC_CRATES {
        let mut sources = Vec::new();
        rust_sources(&crate_dir(logic).join("src"), &mut sources);
        assert!(!sources.is_empty(), "{logic} has sources");
        for path in sources {
            let source = fs::read_to_string(&path).expect("source file is UTF-8");
            assert!(
                !source.contains("extern crate alloc") && !source.contains("alloc::"),
                "{} names the alloc crate",
                path.display()
            );
        }
    }
}
