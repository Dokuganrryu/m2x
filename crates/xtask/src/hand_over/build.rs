use crate::budget::{PYTHON_VARIABLE, python_for};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::firmware::TARGET as FIRMWARE_TARGET;
const HAND_OVER_FOLDER: &str = "hand-over";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Image {
    Full,
    QuiesceOnly,
    Milestone,
    Milestone2,
    Milestone2NoLimit,
    Milestone4,
    Milestone7,
    Sound1,
    Sound2,
    Sound2Ui,
    Sound3,
}

impl Image {
    pub const ALL: [Image; 11] = [
        Image::Full,
        Image::QuiesceOnly,
        Image::Milestone,
        Image::Milestone2,
        Image::Milestone2NoLimit,
        Image::Milestone4,
        Image::Milestone7,
        Image::Sound1,
        Image::Sound2,
        Image::Sound2Ui,
        Image::Sound3,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Image::Full => "full",
            Image::QuiesceOnly => "quiesce-only",
            Image::Milestone => "milestone",
            Image::Milestone2 => "milestone2",
            Image::Milestone2NoLimit => "milestone2-nolimit",
            Image::Milestone4 => "milestone4",
            Image::Milestone7 => "milestone7",
            Image::Sound1 => "sound1",
            Image::Sound2 => "sound2",
            Image::Sound2Ui => "sound2-ui",
            Image::Sound3 => "sound3",
        }
    }

    pub const fn profile(self) -> &'static str {
        match self {
            Image::Sound2 | Image::Sound2Ui | Image::Sound3 => "sound2",
            Image::Full
            | Image::QuiesceOnly
            | Image::Milestone
            | Image::Milestone2
            | Image::Milestone2NoLimit
            | Image::Milestone4
            | Image::Milestone7
            | Image::Sound1 => "release",
        }
    }

    const fn features(self) -> &'static [&'static str] {
        match self {
            Image::Full => &[],
            Image::QuiesceOnly => &["--features", "quiesce-only"],
            Image::Milestone => &["--features", "milestone"],
            Image::Milestone2 => &["--features", "milestone2"],
            Image::Milestone2NoLimit => &["--features", "milestone2-nolimit"],
            Image::Milestone4 => &["--features", "milestone4"],
            Image::Milestone7 => &["--features", "milestone7"],
            Image::Sound1 => &["--features", "sound1"],
            Image::Sound2 => &["--features", "sound2"],
            Image::Sound2Ui => &["--features", "sound2-ui"],
            Image::Sound3 => &["--features", "sound3"],
        }
    }
}

pub struct Built {
    pub elf: PathBuf,
    pub symbols: String,
    pub instruction_listing: String,
    pub sections: String,
    pub folder: PathBuf,
}

pub struct Workspace {
    root: PathBuf,
}

fn run(step: &str, command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| format!("{step} could not start: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{step} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn rustc_prints(request: &str) -> Result<String, String> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    run(
        "asking rustc",
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

impl Workspace {
    pub fn locate() -> Self {
        Self {
            root: Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."),
        }
    }

    pub fn rows(&self) -> Result<String, String> {
        let path = self.root.join("fixtures/lpc55s28-svd-registers.txt");
        fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))
    }

    pub fn scan_rules(&self) -> Result<String, String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("rules/m2x-image.rules");
        fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))
    }

    pub fn build(&self, image: Image) -> Result<Built, String> {
        let target_dir = self
            .root
            .join("target")
            .join(HAND_OVER_FOLDER)
            .join(image.slug());
        let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
        run(
            "building the image",
            Command::new(cargo)
                .current_dir(&self.root)
                .args(["build", "--profile", image.profile()])
                .args(["--package", "board", "--bin", "board"])
                .args(image.features())
                .args(["--target", FIRMWARE_TARGET, "--target-dir"])
                .arg(&target_dir),
        )?;
        let elf = target_dir
            .join(FIRMWARE_TARGET)
            .join(image.profile())
            .join("board");
        let nm = llvm_tool("llvm-nm")?;
        let objdump = llvm_tool("llvm-objdump")?;
        let built = Built {
            symbols: run(
                "listing symbols",
                Command::new(&nm).arg("--demangle").arg(&elf),
            )?,
            instruction_listing: run(
                "decoding",
                Command::new(&objdump)
                    .args(["-d", "--demangle", "--mcpu=cortex-m33"])
                    .arg(&elf),
            )?,
            sections: run(
                "dumping sections",
                Command::new(&objdump).arg("-s").arg(&elf),
            )?,
            folder: target_dir,
            elf,
        };
        fs::write(built.folder.join("nm.txt"), &built.symbols)
            .map_err(|error| error.to_string())?;
        Ok(built)
    }

    pub fn emulate(&self, plan: &str, folder: &Path) -> Result<String, String> {
        let python = python_for(env::var_os(PYTHON_VARIABLE), &self.root).ok_or_else(|| {
            format!(
                "`cargo xtask hand-over` drives the Unicorn emulator from Python. {PYTHON_VARIABLE} is not set and .venv holds no python. \
                 Create one at the repository root with `python -m venv .venv` and `.venv/Scripts/pip install unicorn==2.1.4`."
            )
        })?;
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("hand-over")
            .join("emulate.py");
        let mut child = Command::new(python)
            .arg("-I")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("the emulator could not start: {error}"))?;
        let mut stdin = child.stdin.take().ok_or("the emulator has no stdin")?;
        let plan_text = plan.to_string();
        let writer = std::thread::spawn(move || stdin.write_all(plan_text.as_bytes()));
        let output = child
            .wait_with_output()
            .map_err(|error| format!("the emulator failed: {error}"))?;
        let _ = writer.join();
        if !output.status.success() {
            return Err(format!(
                "the emulator failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let trace = String::from_utf8_lossy(&output.stdout).into_owned();
        fs::write(folder.join("plan.txt"), plan).map_err(|error| error.to_string())?;
        fs::write(folder.join("trace.txt"), &trace).map_err(|error| error.to_string())?;
        Ok(trace)
    }
}
