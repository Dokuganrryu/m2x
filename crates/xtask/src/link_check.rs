use crate::exit::Outcome;
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::firmware::TARGET as FIRMWARE_TARGET;
const PLANTED_BYTES_VARIABLE: &str = "BOARD_PLANTED_BYTES";
const LINKER_ERROR: &str = "rust-lld: error:";
const IMAGE_REGION_LIMIT: Bytes = Bytes(168_532);
const STACK_RESERVE: Bytes = Bytes(8_192);
const LINKER_ALIGNMENT_SLACK: Bytes = Bytes(8);
const SECTION_AFTER_THE_IMAGE: &str = ".panel_buffers";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bytes(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub image: Bytes,
    pub stack: Bytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutRefusal {
    MissingSymbol(&'static str),
    ImageEndsBeforeItStarts,
    StackEndsAboveItsTop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plant {
    OversizeImage,
    StackSqueeze,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Build {
    Clean,
    Planted(Plant),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Holds,
    CleanBuildFailed,
    PlantLinked(Plant),
    FailedForAnotherReason(Plant),
}

fn symbol_address(listing: &str, name: &'static str) -> Result<u32, LayoutRefusal> {
    listing
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let address = fields.next()?;
            let _kind = fields.next()?;
            (fields.next()? == name && fields.next().is_none())
                .then(|| u32::from_str_radix(address, 16).ok())
                .flatten()
        })
        .ok_or(LayoutRefusal::MissingSymbol(name))
}

impl Layout {
    pub fn from_symbols(listing: &str) -> Result<Self, LayoutRefusal> {
        let image_start = symbol_address(listing, "__vector_table")?;
        let data_load = symbol_address(listing, "__sidata")?;
        let data_start = symbol_address(listing, "__sdata")?;
        let data_end = symbol_address(listing, "__edata")?;
        let stack_end = symbol_address(listing, "_stack_end")?;
        let stack_top = symbol_address(listing, "_stack_start")?;
        let image = data_end
            .checked_sub(data_start)
            .and_then(|data| data_load.checked_add(data))
            .and_then(|image_end| image_end.checked_sub(image_start))
            .ok_or(LayoutRefusal::ImageEndsBeforeItStarts)?;
        let stack = stack_top
            .checked_sub(stack_end)
            .ok_or(LayoutRefusal::StackEndsAboveItsTop)?;
        Ok(Self {
            image: Bytes(image),
            stack: Bytes(stack),
        })
    }
}

impl Plant {
    pub const ALL: [Plant; 2] = [Plant::OversizeImage, Plant::StackSqueeze];

    pub const fn assertion(self) -> &'static str {
        match self {
            Plant::OversizeImage => "board: the image is larger than the 168532-byte image region",
            Plant::StackSqueeze => "board: less than 8 KB of stack remains below 0x20030000",
        }
    }

    pub fn bytes_to_break(self, clean: Layout) -> Bytes {
        match self {
            Plant::OversizeImage => Bytes((IMAGE_REGION_LIMIT.0 + 1).saturating_sub(clean.image.0)),
            Plant::StackSqueeze => Bytes(
                (clean.stack.0 + 1 + LINKER_ALIGNMENT_SLACK.0).saturating_sub(STACK_RESERVE.0),
            ),
        }
    }

    const fn cfg_value(self) -> &'static str {
        match self {
            Plant::OversizeImage => "oversize_image",
            Plant::StackSqueeze => "stack_squeeze",
        }
    }
}

fn stopped_only_by(plant: Plant, stderr: &str) -> bool {
    let linker_errors: Vec<&str> = stderr
        .lines()
        .filter(|line| line.contains(LINKER_ERROR))
        .collect();
    linker_errors
        .iter()
        .any(|line| line.contains(plant.assertion()))
        && linker_errors
            .iter()
            .all(|line| line.contains(plant.assertion()) || follows_from(plant, line))
}

fn follows_from(plant: Plant, line: &str) -> bool {
    match plant {
        Plant::OversizeImage => line.contains(SECTION_AFTER_THE_IMAGE),
        Plant::StackSqueeze => false,
    }
}

pub fn judge(build: Build, linked: bool, stderr: &str) -> Verdict {
    match (build, linked) {
        (Build::Clean, true) => Verdict::Holds,
        (Build::Clean, false) => Verdict::CleanBuildFailed,
        (Build::Planted(plant), true) => Verdict::PlantLinked(plant),
        (Build::Planted(plant), false) if stopped_only_by(plant, stderr) => Verdict::Holds,
        (Build::Planted(plant), false) => Verdict::FailedForAnotherReason(plant),
    }
}

impl fmt::Display for Build {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Build::Clean => f.write_str("clean image"),
            Build::Planted(plant) => write!(f, "planted {}", plant.cfg_value()),
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Holds => f.write_str("holds"),
            Verdict::CleanBuildFailed => f.write_str("the clean image failed to link"),
            Verdict::PlantLinked(_) => f.write_str("linked, so its assertion did not hold"),
            Verdict::FailedForAnotherReason(plant) => write!(
                f,
                "failed, but not by the assertion `{}` alone",
                plant.assertion()
            ),
        }
    }
}

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn locate() -> Self {
        Self {
            root: Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."),
        }
    }

    fn target_dir(&self, build: Build) -> PathBuf {
        match build {
            Build::Clean => self.root.join("target"),
            Build::Planted(_) => self.root.join("target").join("planted"),
        }
    }

    fn clean_image(&self) -> PathBuf {
        self.target_dir(Build::Clean)
            .join(FIRMWARE_TARGET)
            .join("release")
            .join("board")
    }

    fn link(&self, build: Build, planted: Bytes) -> std::io::Result<(bool, String)> {
        let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let mut command = Command::new(cargo);
        command
            .current_dir(&self.root)
            .args(["rustc", "--release", "--package", "board", "--bin", "board"])
            .args(["--target", FIRMWARE_TARGET, "--target-dir"])
            .arg(self.target_dir(build));
        match build {
            Build::Clean => {}
            Build::Planted(plant) => {
                command
                    .env(PLANTED_BYTES_VARIABLE, planted.0.to_string())
                    .args(["--", "--cfg"])
                    .arg(format!("plant=\"{}\"", plant.cfg_value()));
            }
        }
        let output = command.output()?;
        Ok((
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

fn rustc_prints(request: &str) -> std::io::Result<String> {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let output = Command::new(rustc).args(["--print", request]).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn symbol_listing(image: &Path) -> std::io::Result<String> {
    let llvm_nm = PathBuf::from(rustc_prints("sysroot")?)
        .join("lib")
        .join("rustlib")
        .join(rustc_prints("host-tuple")?)
        .join("bin")
        .join(format!("llvm-nm{}", env::consts::EXE_SUFFIX));
    let output = Command::new(llvm_nm).arg(image).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn report(build: Build, verdict: Verdict) -> bool {
    println!("{build}: {verdict}");
    verdict == Verdict::Holds
}

pub fn command() -> Outcome {
    let workspace = Workspace::locate();
    let clean = match workspace.link(Build::Clean, Bytes(0)) {
        Ok((linked, stderr)) => judge(Build::Clean, linked, &stderr),
        Err(error) => {
            eprintln!("cargo could not run: {error}");
            return Outcome::UsageOrInputError;
        }
    };
    if !report(Build::Clean, clean) {
        return Outcome::GateFailed;
    }
    let layout = match symbol_listing(&workspace.clean_image())
        .map(|listing| Layout::from_symbols(&listing))
    {
        Ok(Ok(layout)) => layout,
        Ok(Err(refusal)) => {
            eprintln!("the clean image layout is unreadable: {refusal:?}");
            return Outcome::UsageOrInputError;
        }
        Err(error) => {
            eprintln!("llvm-nm could not run: {error}");
            return Outcome::UsageOrInputError;
        }
    };
    println!(
        "clean image: {} of {} image bytes, {} bytes of stack against a reserve of {}",
        layout.image.0, IMAGE_REGION_LIMIT.0, layout.stack.0, STACK_RESERVE.0
    );
    let mut every_plant_holds = true;
    for plant in Plant::ALL {
        let build = Build::Planted(plant);
        let planted = plant.bytes_to_break(layout);
        let verdict = match workspace.link(build, planted) {
            Ok((linked, stderr)) => judge(build, linked, &stderr),
            Err(error) => {
                eprintln!("cargo could not run: {error}");
                return Outcome::UsageOrInputError;
            }
        };
        println!("{build} with {} planted bytes", planted.0);
        every_plant_holds &= report(build, verdict);
    }
    if every_plant_holds {
        Outcome::GatePassed
    } else {
        Outcome::GateFailed
    }
}
