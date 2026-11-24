use std::fs;
use std::path::{Path, PathBuf};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn read(path: PathBuf) -> String {
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn release_profile(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[profile.release]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
fn the_image_builds_for_the_soft_float_target_with_the_flags_the_budget_bench_uses() {
    let config = read(repository().join(".cargo").join("config.toml"));
    let section = format!("[target.{}]", xtask::firmware::TARGET);
    let rustflags = config
        .lines()
        .skip_while(|line| line.trim() != section)
        .find_map(|line| line.trim().strip_prefix("rustflags = "))
        .unwrap_or_else(|| panic!("{section} with rustflags in .cargo/config.toml"));
    let flags: Vec<&str> = rustflags
        .trim_matches(['[', ']'])
        .split(',')
        .map(|flag| flag.trim().trim_matches('"'))
        .collect();
    assert_eq!(
        flags,
        xtask::firmware::RUSTFLAGS
            .split_whitespace()
            .collect::<Vec<_>>()
    );
    assert_eq!(xtask::firmware::TARGET, "thumbv8m.main-none-eabi");
    assert!(xtask::firmware::RUSTFLAGS.ends_with("-C target-feature=-fpregs"));
    for alias in ["firmware = ", "firmware-clippy = "] {
        let line = config
            .lines()
            .find(|line| line.starts_with(alias))
            .unwrap_or_else(|| panic!("alias {alias}"));
        let target = line
            .split_whitespace()
            .skip_while(|word| *word != "--target")
            .nth(1)
            .map(|word| word.trim_end_matches('"'));
        assert_eq!(target, Some(xtask::firmware::TARGET), "{line}");
    }
}

#[test]
fn the_budget_bench_builds_with_the_release_profile_of_the_image() {
    let image = release_profile(&read(repository().join("Cargo.toml")));
    let bench = release_profile(&read(
        repository()
            .join("crates")
            .join("xtask")
            .join("budget-bench")
            .join("Cargo.toml"),
    ));
    assert!(
        image.iter().any(|line| line.starts_with("lto")),
        "{image:?}"
    );
    assert_eq!(bench, image);
}

#[test]
fn the_budget_command_names_the_missing_python_package_and_the_variable_that_fixes_it() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("budget")
        .env(
            xtask::budget::PYTHON_VARIABLE,
            repository().join("no-such-python"),
        )
        .output()
        .expect("the xtask binary runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("unicorn"), "{stderr}");
    assert!(stderr.contains(xtask::budget::PYTHON_VARIABLE), "{stderr}");
}

#[test]
fn the_budget_python_is_the_variable_then_the_repository_environment_then_none() {
    let root = std::env::temp_dir().join(format!("m2x-budget-python-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let named = std::ffi::OsString::from("named-python");
    assert_eq!(xtask::budget::python_for(None, &root), None);
    let environment = root.join(".venv").join("Scripts");
    fs::create_dir_all(&environment).expect("the environment directory is made");
    fs::write(environment.join("python.exe"), b"").expect("the python file is made");
    assert_eq!(
        xtask::budget::python_for(None, &root),
        Some(environment.join("python.exe").into_os_string())
    );
    assert_eq!(
        xtask::budget::python_for(Some(named.clone()), &root),
        Some(named)
    );
    fs::remove_dir_all(&root).expect("the scratch root is removed");
}
