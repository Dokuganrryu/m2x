use std::env;
use std::process::ExitCode;
use xtask::exit::Outcome;

const USAGE: &str = "usage: xtask price <listing>...
       xtask scan <rules> <llvm-nm listing> <llvm-objdump listing>
       xtask link-check
       xtask scan-image
       xtask budget
       xtask board-run
       xtask hand-over";

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    match arguments.split_first() {
        Some((command, listings)) if command == "price" && !listings.is_empty() => {
            xtask::price::command(listings).into()
        }
        Some((command, [rules, symbols, instruction_listing])) if command == "scan" => {
            xtask::scan::command(rules, symbols, instruction_listing).into()
        }
        Some((command, [])) if command == "link-check" => xtask::link_check::command().into(),
        Some((command, [])) if command == "scan-image" => xtask::scan_image::command().into(),
        Some((command, [])) if command == "budget" => xtask::budget::command().into(),
        Some((command, [])) if command == "board-run" => xtask::board_run::command().into(),
        Some((command, [])) if command == "hand-over" => xtask::hand_over::command().into(),
        _ => {
            eprintln!("{USAGE}");
            Outcome::UsageOrInputError.into()
        }
    }
}
