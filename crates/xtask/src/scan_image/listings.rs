use super::{ImageBuild, Level};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::firmware::TARGET as FIRMWARE_TARGET;
const SCAN_IMAGE_FOLDER: &str = "scan-image";

pub struct Listings {
    pub symbols: String,
    pub instruction_listing: String,
    pub sections: String,
    pub folder: PathBuf,
}

pub struct Workspace {
    root: PathBuf,
}

impl ImageBuild {
    fn slug(self) -> String {
        match self {
            ImageBuild::Clean(Level::Speed) => "clean-speed".to_string(),
            ImageBuild::Clean(Level::BoardForSize) => "clean-board-size".to_string(),
            ImageBuild::Planted(plant) => format!("planted-{}", plant.cfg_value()),
        }
    }

    fn rustc_arguments(self) -> Vec<String> {
        let level = match self {
            ImageBuild::Clean(level) => level,
            ImageBuild::Planted(plant) => plant.level(),
        };
        let mut arguments = match level {
            Level::Speed => Vec::new(),
            Level::BoardForSize => vec!["-C".to_string(), "opt-level=z".to_string()],
        };
        if let ImageBuild::Planted(plant) = self {
            arguments.extend([
                "--cfg".to_string(),
                "image_plant".to_string(),
                "--cfg".to_string(),
                format!("image_plant=\"{}\"", plant.cfg_value()),
            ]);
        }
        arguments
    }
}

impl Workspace {
    pub fn locate() -> Self {
        Self {
            root: Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."),
        }
    }

    pub fn rules(&self, name: &str) -> std::io::Result<String> {
        fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("rules")
                .join(name),
        )
    }

    fn target_dir(&self, build: ImageBuild) -> PathBuf {
        match build {
            ImageBuild::Clean(Level::Speed) => self.root.join("target"),
            ImageBuild::Clean(Level::BoardForSize) | ImageBuild::Planted(_) => {
                self.root.join("target").join(SCAN_IMAGE_FOLDER)
            }
        }
    }

    pub fn build_and_list(&self, build: ImageBuild) -> Result<Listings, String> {
        let target_dir = self.target_dir(build);
        let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let output = Command::new(cargo)
            .current_dir(&self.root)
            .args(["rustc", "--release", "--package", "board", "--bin", "board"])
            .args(["--target", FIRMWARE_TARGET, "--target-dir"])
            .arg(&target_dir)
            .arg("--")
            .args(build.rustc_arguments())
            .output()
            .map_err(|error| format!("cargo could not run: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "the build failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let image = target_dir
            .join(FIRMWARE_TARGET)
            .join("release")
            .join("board");
        let listings = Listings {
            symbols: llvm_tool("llvm-nm", &["--demangle"], &image)?,
            instruction_listing: llvm_tool(
                "llvm-objdump",
                &["-d", "--demangle", "--mcpu=cortex-m33"],
                &image,
            )?,
            sections: llvm_tool("llvm-objdump", &["-s"], &image)?,
            folder: self
                .root
                .join("target")
                .join(SCAN_IMAGE_FOLDER)
                .join("listings")
                .join(build.slug()),
        };
        listings.save()?;
        Ok(listings)
    }
}

impl Listings {
    fn save(&self) -> Result<(), String> {
        let write = |name: &str, text: &str| {
            fs::write(self.folder.join(name), text)
                .map_err(|error| format!("{}: {error}", self.folder.join(name).display()))
        };
        fs::create_dir_all(&self.folder)
            .map_err(|error| format!("{}: {error}", self.folder.display()))?;
        write("nm.txt", &self.symbols)?;
        write("objdump.txt", &self.instruction_listing)?;
        write("sections.txt", &self.sections)
    }
}

fn rustc_prints(request: &str) -> Result<String, String> {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let output = Command::new(rustc)
        .args(["--print", request])
        .output()
        .map_err(|error| format!("rustc could not run: {error}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn llvm_tool(name: &str, arguments: &[&str], image: &Path) -> Result<String, String> {
    let tool = PathBuf::from(rustc_prints("sysroot")?)
        .join("lib")
        .join("rustlib")
        .join(rustc_prints("host-tuple")?)
        .join("bin")
        .join(format!("{name}{}", env::consts::EXE_SUFFIX));
    let output = Command::new(&tool)
        .args(arguments)
        .arg(image)
        .output()
        .map_err(|error| format!("{} could not run: {error}", tool.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{name} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
