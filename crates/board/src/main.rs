#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(target_os = "none")]
mod firmware;

#[cfg(not(target_os = "none"))]
fn main() -> std::process::ExitCode {
    eprintln!("the board crate is the firmware image; build it with `cargo firmware`");
    std::process::ExitCode::from(2)
}
