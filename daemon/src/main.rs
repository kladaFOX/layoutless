//! layoutless: us ↔ ru layout correction for Hyprland.
//!
//! `layoutless` runs the daemon. `layoutless flip` asks the running daemon to
//! retype the last word in the other layout (bound to Alt+Space).

mod config;
mod control;
mod daemon;
mod detect;
mod eval;
mod hotkey;
mod hypr;
mod input;
mod keymap;
mod output;
mod private;
mod rules;

use std::process::ExitCode;

const USAGE: &str = "usage: layoutless [daemon | flip | auto on|off|toggle | status | reload
                  | rules [list [--json] | add WORD | rm WORD | set KEYS en|ru]
                  | exclude add|rm CLASS | ctl (command on stdin) | subscribe | eval FILE...]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["daemon"] => daemon::run(),
        ["flip"] | ["status"] | ["state"] | ["reload"] | ["auto", "on" | "off" | "toggle"] => {
            control::request(&args.join(" ")).map(|reply| println!("{reply}"))
        }
        ["rules"] | ["rules", "list"] => control::request("rules list").and_then(|r| print_rules(&r)),
        ["rules", "list", "--json"] => control::request("rules list").map(|r| println!("{r}")),
        ["rules", "add" | "rm", _] | ["rules", "set", _, _] => control::request(&args.join(" ")).and_then(|reply| {
            println!("{reply}");
            if reply.starts_with("error") { Err(std::io::Error::other("rules command failed")) } else { Ok(()) }
        }),
        ["exclude", "add" | "rm", _] => control::request(&args.join(" ")).map(|reply| println!("{reply}")),
        ["subscribe"] => control::subscribe(),
        // One command line read from stdin, so words never show up in this
        // process's arguments (`ps`, /proc). The bar widget uses this.
        ["ctl"] => read_command().and_then(|cmd| control::request(&cmd)).map(|reply| println!("{reply}")),
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

/// `rules list` for people: one rule per line, the word as it belongs first.
fn print_rules(reply: &str) -> std::io::Result<()> {
    let v: serde_json::Value = serde_json::from_str(reply).map_err(|_| std::io::Error::other(reply.to_string()))?;
    let rules = v["rules"].as_array().cloned().unwrap_or_default();
    if rules.is_empty() {
        println!("no rules yet");
    }
    for r in rules {
        let (en, ru) = (r["en"].as_str().unwrap_or(""), r["ru"].as_str().unwrap_or(""));
        let (word, other) = if r["lang"] == "en" { (en, ru) } else { (ru, en) };
        println!("{word:<20} ← {other}");
    }
    Ok(())
}

fn read_command() -> std::io::Result<String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let cmd = line.trim();
    if cmd.is_empty() {
        return Err(std::io::Error::other("ctl: expected a command on stdin"));
    }
    Ok(cmd.to_string())
}
