use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::gate::ParkFunction;
use crate::budget::{PYTHON_VARIABLE, python_for};

use crate::firmware::TARGET as FIRMWARE_TARGET;
// WHY: Fault::enter stores the code and the park loop runs in it or in whichever of these the compiler keeps out of line
const PARK_ENTRY: &str = "board::firmware::fault::enter";
const PARK_CALLEES: [&str; 5] = [
    "board::firmware::fault::park",
    "board::firmware::fault::end",
    "board::firmware::watchdog::feed_while_parked",
    "board::firmware::watchdog::feed",
    "board::firmware::watchdog::feed_now",
];

pub struct Workspace {
    pub root: PathBuf,
}

pub struct Image {
    pub elf: PathBuf,
    pub reset: u32,
    pub breadcrumb: u32,
    pub usb_notices: u32,
    pub park: Vec<ParkFunction>,
}

impl Workspace {
    pub fn locate() -> Self {
        Self {
            root: Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."),
        }
    }

    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
            .replace(std::path::MAIN_SEPARATOR, "/")
    }

    pub fn output_dir(&self) -> PathBuf {
        self.root.join("target").join("board-run")
    }

    pub fn build_image(&self) -> Result<Image, String> {
        run(
            "building the firmware image",
            Command::new(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
                .current_dir(&self.root)
                .args([
                    "build",
                    "--release",
                    "--package",
                    "board",
                    "--target",
                    FIRMWARE_TARGET,
                ]),
        )?;
        let elf = self
            .root
            .join("target")
            .join(FIRMWARE_TARGET)
            .join("release")
            .join("board");
        let symbols = run(
            "listing the image's symbols",
            Command::new(llvm_tool("llvm-nm")?)
                .args(["-C", "--defined-only", "--print-size"])
                .arg(&elf),
        )?;
        let find_sized = |name: &str| {
            symbol(&symbols, name).ok_or_else(|| format!("the image has no symbol `{name}`"))
        };
        let find = |name: &str| find_sized(name).map(|(address, _)| address);
        let park_function = |name: &'static str, (start, size): (u32, u32)| ParkFunction {
            name,
            start,
            end: start + size,
        };
        let mut park = vec![park_function(PARK_ENTRY, find_sized(PARK_ENTRY)?)];
        park.extend(PARK_CALLEES.into_iter().filter_map(|name| {
            symbol(&symbols, name).map(|location| park_function(name, location))
        }));
        Ok(Image {
            reset: find("Reset")?,
            breadcrumb: find("BREADCRUMB")?,
            usb_notices: find("board::firmware::handovers::USB_NOTICES (.0)")
                .or_else(|_| find("board::firmware::handovers::USB_NOTICES"))?,
            park,
            elf,
        })
    }

    pub fn python(&self) -> Result<OsString, String> {
        python_for(env::var_os(PYTHON_VARIABLE), &self.root).ok_or_else(|| {
            format!(
                "`cargo xtask board-run` drives Unicorn from Python and found none: set {PYTHON_VARIABLE}, or run `python -m venv .venv` and `.venv/Scripts/pip install unicorn==2.1.4` at the repository root"
            )
        })
    }

    pub fn emulate(&self, plan: &str) -> Result<String, String> {
        let script = self
            .root
            .join("crates")
            .join("xtask")
            .join("board-model")
            .join("board_model.py");
        let mut child = Command::new(self.python()?)
            .arg("-I")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("starting the board model failed: {error}"))?;
        let mut stdin = child.stdin.take().ok_or("the board model has no stdin")?;
        let plan = plan.to_string();
        let writer = std::thread::spawn(move || stdin.write_all(plan.as_bytes()));
        let output = child
            .wait_with_output()
            .map_err(|error| format!("running the board model failed: {error}"))?;
        let _ = writer.join();
        if !output.status.success() {
            return Err(format!(
                "the board model failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn keep(&self, name: &str, text: &str) -> Result<(), String> {
        let directory = self.output_dir();
        std::fs::create_dir_all(&directory)
            .and_then(|()| std::fs::write(directory.join(name), text))
            .map_err(|error| format!("keeping {name} failed: {error}"))
    }
}

fn symbol(listing: &str, name: &str) -> Option<(u32, u32)> {
    listing.lines().find_map(|line| {
        let mut words = line.splitn(4, ' ');
        let address = words.next()?;
        let size = words.next()?;
        let _kind = words.next()?;
        (words.next()? == name).then(|| {
            Some((
                u32::from_str_radix(address, 16).ok()?,
                u32::from_str_radix(size, 16).ok()?,
            ))
        })?
    })
}

fn rustc_prints(request: &str) -> Result<String, String> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    run(
        "asking rustc for its sysroot",
        Command::new(rustc).args(["--print", request]),
    )
    .map(|text| text.trim().to_string())
}

fn llvm_tool(name: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(rustc_prints("sysroot")?)
        .join("lib")
        .join("rustlib")
        .join(rustc_prints("host-tuple")?)
        .join("bin")
        .join(format!("{name}{}", env::consts::EXE_SUFFIX)))
}

fn run(step: &str, command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| format!("{step} failed: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{step} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
