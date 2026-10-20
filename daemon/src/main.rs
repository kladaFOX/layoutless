//! layoutless: us ↔ ru layout correction for Hyprland.
//!
//! `layoutless` runs the daemon. `layoutless flip` asks the running daemon to
//! retype the last word in the other layout (bound to Alt+Space).

mod control;
mod daemon;
mod detect;
mod eval;
mod hypr;
mod input;
mod keymap;
mod output;

use std::process::ExitCode;

const USAGE: &str = "usage: layoutless [daemon | flip | auto on|off|toggle | status | eval FILE...]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["daemon"] => daemon::run(),
        ["flip"] | ["status"] | ["auto", "on" | "off" | "toggle"] => {
            control::request(&args.join(" ")).map(|reply| println!("{reply}"))
        }
        ["eval", ..] => eval::run(&args[1..]),
        ["-h" | "--help"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("layoutless: {e}");
            ExitCode::FAILURE
        }
    }
}
