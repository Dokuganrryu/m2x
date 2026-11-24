use std::env;
use std::ffi::OsString;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const PYTHON_VARIABLE: &str = "M2X_PYTHON";
use crate::firmware::TARGET as FIRMWARE_TARGET;
const BENCH_ENTRIES: [&str; 17] = [
    "budget_init",
    "budget_request",
    "budget_tick",
    "budget_usb_event",
    "budget_usb_console_event",
    "budget_usb_host_starts_a_microframe",
    "budget_usb_host_starts_a_microframe_with_console_bytes_waiting",
    "budget_usb_host_sends_a_playback_packet",
    "budget_usb_host_reads_feedback",
    "budget_usb_host_reads_a_capture_packet",
    "budget_usb_host_reads_the_console_packet",
    "budget_usb_host_sends_console_bytes",
    "budget_usb_host_reads_the_configuration_descriptor",
    "budget_usb_host_sets_the_sampling_frequency",
    "budget_usb_host_reselects_capture",
    "budget_usb_host_continues_the_control_transfer",
    "budget_usb_check",
];

#[derive(Debug)]
pub enum ToolError {
    PythonMissing { python: OsString, detail: String },
    Failed { step: &'static str, detail: String },
}

pub struct Workspace {
    pub root: PathBuf,
}

pub struct Built {
    pub elf: PathBuf,
    pub symbols: String,
    pub instruction_listing: String,
}

impl Workspace {
    pub fn locate() -> Self {
        Self {
            root: Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."),
        }
    }

    fn bench_dir(&self) -> PathBuf {
        self.root.join("crates").join("xtask").join("budget-bench")
    }

    fn bench_target(&self) -> PathBuf {
        self.root.join("target").join("budget-bench")
    }

    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
            .replace(std::path::MAIN_SEPARATOR, "/")
    }

    pub fn image_path(&self) -> PathBuf {
        self.root
            .join("target")
            .join(FIRMWARE_TARGET)
            .join("release")
            .join("board")
    }

    pub fn bench_path(&self) -> PathBuf {
        self.bench_target().join("budget.elf")
    }

    pub fn keep_emulation(&self, files: &[(&str, &String)]) -> Result<(), ToolError> {
        for (name, text) in files {
            std::fs::write(self.bench_target().join(name), text).map_err(|error| {
                ToolError::Failed {
                    step: "keeping the emulator plans and traces",
                    detail: error.to_string(),
                }
            })?;
        }
        Ok(())
    }

    pub fn build_bench(&self) -> Result<Built, ToolError> {
        let manifest = self.bench_dir().join("Cargo.toml");
        run(
            "building the budget bench",
            Command::new(cargo())
                .current_dir(&self.root)
                .env("CARGO_TARGET_DIR", self.bench_target())
                .env("RUSTFLAGS", crate::firmware::RUSTFLAGS)
                .args([
                    "build",
                    "--release",
                    "--target",
                    FIRMWARE_TARGET,
                    "--manifest-path",
                ])
                .arg(manifest),
        )?;
        let archive = self
            .bench_target()
            .join(FIRMWARE_TARGET)
            .join("release")
            .join("libbudget_bench.a");
        let elf = self.bench_path();
        let mut link = Command::new(llvm_tool("rust-lld")?);
        link.args(["-flavor", "gnu", "-T"])
            .arg(self.bench_dir().join("link.x"))
            .arg("--gc-sections");
        for entry in BENCH_ENTRIES {
            link.args(["-u", entry]);
        }
        link.arg(archive).arg("-o").arg(&elf);
        run("linking the budget bench", &mut link)?;
        read_elf(elf)
    }

    pub fn build_image(&self) -> Result<Built, ToolError> {
        run(
            "building the firmware image",
            Command::new(cargo()).current_dir(&self.root).args([
                "build",
                "--release",
                "--package",
                "board",
                "--target",
                FIRMWARE_TARGET,
            ]),
        )?;
        read_elf(self.image_path())
    }
}

fn read_elf(elf: PathBuf) -> Result<Built, ToolError> {
    let symbols = run(
        "listing symbols",
        Command::new(llvm_tool("llvm-nm")?)
            .args(["-C", "-S", "-n", "--defined-only"])
            .arg(&elf),
    )?;
    let instruction_listing = run(
        "decoding",
        Command::new(llvm_tool("llvm-objdump")?)
            .args(["-d", "-C", "--mcpu=cortex-m33"])
            .arg(&elf),
    )?;
    Ok(Built {
        elf,
        symbols,
        instruction_listing,
    })
}

const REPOSITORY_ENVIRONMENT: &str = ".venv";
const ENVIRONMENT_PYTHONS: [[&str; 2]; 2] = [["Scripts", "python.exe"], ["bin", "python"]];

pub fn python_for(variable: Option<OsString>, root: &Path) -> Option<OsString> {
    variable.or_else(|| {
        ENVIRONMENT_PYTHONS
            .iter()
            .map(|[directory, python]| {
                root.join(REPOSITORY_ENVIRONMENT)
                    .join(directory)
                    .join(python)
            })
            .find(|python| python.is_file())
            .map(PathBuf::into_os_string)
    })
}

impl Workspace {
    pub fn python(&self) -> Result<OsString, ToolError> {
        python_for(env::var_os(PYTHON_VARIABLE), &self.root).ok_or_else(|| {
            ToolError::PythonMissing {
                python: OsString::from(REPOSITORY_ENVIRONMENT),
                detail: format!(
                    "{PYTHON_VARIABLE} is not set and {} holds no python",
                    self.relative(&self.root.join(REPOSITORY_ENVIRONMENT))
                ),
            }
        })
    }

    pub fn check_python(&self) -> Result<(), ToolError> {
        let python = self.python()?;
        let missing = |detail: String| ToolError::PythonMissing {
            python: python.clone(),
            detail,
        };
        let output = Command::new(&python)
            .args(["-I", "-c", "import unicorn; print(unicorn.__version__)"])
            .output()
            .map_err(|error| missing(error.to_string()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(missing(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ))
        }
    }
}

pub fn emulate(workspace: &Workspace, plan: &str) -> Result<String, ToolError> {
    let script = workspace
        .root
        .join("crates")
        .join("xtask")
        .join("budget-bench")
        .join("emulate.py");
    let mut child = Command::new(workspace.python()?)
        .arg("-I")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ToolError::Failed {
            step: "starting the emulator",
            detail: error.to_string(),
        })?;
    let mut stdin = child.stdin.take().ok_or(ToolError::Failed {
        step: "starting the emulator",
        detail: "no stdin".to_string(),
    })?;
    let plan = plan.to_string();
    let writer = std::thread::spawn(move || stdin.write_all(plan.as_bytes()));
    let output = child
        .wait_with_output()
        .map_err(|error| ToolError::Failed {
            step: "running the emulator",
            detail: error.to_string(),
        })?;
    let _ = writer.join();
    if !output.status.success() {
        return Err(ToolError::Failed {
            step: "running the emulator",
            detail: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn cargo() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn rustc_prints(request: &str) -> Result<String, ToolError> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    run(
        "asking rustc for its sysroot",
        Command::new(rustc).args(["--print", request]),
    )
    .map(|text| text.trim().to_string())
}

fn llvm_tool(name: &str) -> Result<PathBuf, ToolError> {
    Ok(PathBuf::from(rustc_prints("sysroot")?)
        .join("lib")
        .join("rustlib")
        .join(rustc_prints("host-tuple")?)
        .join("bin")
        .join(format!("{name}{}", env::consts::EXE_SUFFIX)))
}

fn run(step: &'static str, command: &mut Command) -> Result<String, ToolError> {
    let output = command.output().map_err(|error| ToolError::Failed {
        step,
        detail: error.to_string(),
    })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(ToolError::Failed {
            step,
            detail: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::PythonMissing { python, detail } => write!(
                f,
                "`cargo xtask budget` drives the Unicorn emulator from Python, and `{}` cannot import `unicorn` ({detail}). \
                 Create one at the repository root with `python -m venv .venv` and `.venv/Scripts/pip install unicorn==2.1.4`, \
                 or set {PYTHON_VARIABLE} to a python that has it.",
                python.to_string_lossy()
            ),
            ToolError::Failed { step, detail } => write!(f, "{step} failed:\n{detail}"),
        }
    }
}
