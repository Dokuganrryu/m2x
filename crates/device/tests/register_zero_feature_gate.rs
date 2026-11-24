use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const PROTOCOL_FEATURE: &str = "register-zero-protocol";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/device sits two levels below the workspace root")
        .to_path_buf()
}

fn manifests() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut found = vec![root.join("Cargo.toml")];
    for entry in fs::read_dir(root.join("crates")).expect("the crates folder") {
        let manifest = entry.expect("a crate folder").path().join("Cargo.toml");
        if manifest.is_file() {
            found.push(manifest);
        }
    }
    found
}

fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("")
}

struct Manifest {
    features: BTreeMap<String, Vec<String>>,
    dependency_lines: Vec<String>,
}

fn quoted_items(text: &str) -> Vec<String> {
    text.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

fn read(path: &Path) -> Manifest {
    let text = fs::read_to_string(path).expect("a readable manifest");
    let mut section = String::new();
    let mut features = BTreeMap::new();
    let mut dependency_lines = Vec::new();
    let mut open_feature: Option<(String, String)> = None;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if let Some((name, mut gathered)) = open_feature.take() {
            gathered.push_str(line);
            if line.contains(']') {
                features.insert(name, quoted_items(&gathered));
            } else {
                open_feature = Some((name, gathered));
            }
            continue;
        }
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_string();
            continue;
        }
        if section == "features" {
            if let Some((name, value)) = line.split_once('=') {
                let name = name.trim().trim_matches('"').to_string();
                if value.contains(']') {
                    features.insert(name, quoted_items(value));
                } else {
                    open_feature = Some((name, value.to_string()));
                }
            }
        } else if section.contains("dependencies") {
            dependency_lines.push(format!("[{section}] {line}"));
        }
    }
    Manifest {
        features,
        dependency_lines,
    }
}

fn enables_protocol(item: &str) -> bool {
    item == PROTOCOL_FEATURE || item.ends_with(&format!("/{PROTOCOL_FEATURE}"))
}

fn default_closure(features: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut pending: Vec<String> = features.get("default").cloned().unwrap_or_default();
    while let Some(item) = pending.pop() {
        if seen.insert(item.clone()) {
            if let Some(more) = features.get(&item) {
                pending.extend(more.iter().cloned());
            }
        }
    }
    seen
}

#[test]
fn the_register_zero_protocol_feature_is_never_a_default_feature_of_any_crate() {
    for path in manifests() {
        let manifest = read(&path);
        let defaults = default_closure(&manifest.features);
        assert!(
            !defaults.iter().any(|item| enables_protocol(item)),
            "{} turns on {PROTOCOL_FEATURE} by default: {defaults:?}",
            path.display()
        );
        for line in &manifest.dependency_lines {
            assert!(
                !line.contains(PROTOCOL_FEATURE),
                "{} enables {PROTOCOL_FEATURE} on a dependency unconditionally: {line}",
                path.display()
            );
        }
    }
}

#[test]
fn the_device_crate_declares_the_protocol_feature_with_no_other_feature_behind_it() {
    let manifest = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    assert_eq!(
        manifest.features.get(PROTOCOL_FEATURE),
        Some(&Vec::new()),
        "device declares {PROTOCOL_FEATURE} = []"
    );
    assert!(!manifest.features.contains_key("default"));
}

#[test]
fn the_gate_reads_a_default_that_reaches_the_feature_through_another_feature() {
    let mut features = BTreeMap::new();
    features.insert("default".to_string(), vec!["bench".to_string()]);
    features.insert(
        "bench".to_string(),
        vec![format!("device/{PROTOCOL_FEATURE}")],
    );
    assert!(
        default_closure(&features)
            .iter()
            .any(|item| enables_protocol(item))
    );
}
