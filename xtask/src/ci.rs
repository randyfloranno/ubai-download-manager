//! `cargo xtask ci [--fast]`: every quality gate of AGENTS.md §5.13, in order,
//! stopping at the first failure. Prints a summary table at the end.

use std::time::{Duration, Instant};

use crate::util;

enum Step {
    Cargo(&'static [&'static str]),
    Internal(fn() -> Result<(), String>),
}

struct Gate {
    name: &'static str,
    step: Step,
    /// Skipped by `--fast`.
    slow_rerun: bool,
}

const ALL: &str = "--all-features";

fn gates() -> Vec<Gate> {
    vec![
        Gate {
            name: "fmt",
            step: Step::Cargo(&["fmt", "--all", "--", "--check"]),
            slow_rerun: false,
        },
        Gate {
            name: "clippy",
            step: Step::Cargo(&[
                "clippy",
                "--workspace",
                "--all-targets",
                ALL,
                "--",
                "-D",
                "warnings",
            ]),
            slow_rerun: false,
        },
        Gate {
            name: "test",
            step: Step::Cargo(&["test", "--workspace", ALL]),
            slow_rerun: false,
        },
        Gate {
            name: "hostile",
            step: Step::Cargo(&["test", "--workspace", ALL, "hostile_"]),
            slow_rerun: true,
        },
        Gate {
            name: "golden",
            step: Step::Cargo(&["test", "--workspace", ALL, "golden_"]),
            slow_rerun: true,
        },
        Gate {
            name: "layers",
            step: Step::Internal(crate::layers::run),
            slow_rerun: false,
        },
        Gate {
            name: "i18n",
            step: Step::Internal(crate::i18n::run),
            slow_rerun: false,
        },
        Gate {
            name: "docs",
            step: Step::Internal(crate::docs::run),
            slow_rerun: false,
        },
        Gate {
            name: "limits",
            step: Step::Internal(crate::limits::run),
            slow_rerun: false,
        },
        Gate {
            name: "deny",
            step: Step::Internal(crate::deny::run),
            slow_rerun: false,
        },
        Gate {
            name: "wasm",
            step: Step::Internal(crate::wasm::run),
            slow_rerun: false,
        },
        Gate {
            name: "glue",
            step: Step::Internal(crate::glue::run),
            slow_rerun: false,
        },
    ]
}

enum Outcome {
    Pass(Duration),
    Fail(Duration),
    Skipped(&'static str),
    NotRun,
}

pub fn run(args: &[String]) -> Result<(), String> {
    let mut fast = false;
    for a in args {
        match a.as_str() {
            "--fast" => fast = true,
            other => return Err(format!("unknown ci option `{other}` (only --fast)")),
        }
    }
    let gates = gates();
    let mut outcomes: Vec<Outcome> = gates.iter().map(|_| Outcome::NotRun).collect();
    let mut failure: Option<(String, String)> = None;
    let started = Instant::now();

    for (gate, outcome) in gates.iter().zip(outcomes.iter_mut()) {
        if fast && gate.slow_rerun {
            *outcome = Outcome::Skipped("--fast");
            continue;
        }
        println!("\n==== ci: {} ====", gate.name);
        let t = Instant::now();
        let res = match &gate.step {
            Step::Cargo(args) => {
                let mut cmd = util::cargo();
                cmd.args(*args);
                util::run_status(cmd)
            }
            Step::Internal(f) => f(),
        };
        match res {
            Ok(()) => *outcome = Outcome::Pass(t.elapsed()),
            Err(e) => {
                *outcome = Outcome::Fail(t.elapsed());
                failure = Some((gate.name.to_string(), e));
                break;
            }
        }
    }

    println!("\n==== ci summary ====");
    println!("{:<10} {:<8} {:>9}", "gate", "result", "time");
    for (gate, outcome) in gates.iter().zip(&outcomes) {
        let (res, time) = match outcome {
            Outcome::Pass(d) => ("PASS", format!("{:.1}s", d.as_secs_f64())),
            Outcome::Fail(d) => ("FAIL", format!("{:.1}s", d.as_secs_f64())),
            Outcome::Skipped(why) => ("skip", why.to_string()),
            Outcome::NotRun => ("-", String::new()),
        };
        println!("{:<10} {:<8} {:>9}", gate.name, res, time);
    }
    println!("total {:.1}s", started.elapsed().as_secs_f64());

    match failure {
        None => {
            println!("ci: all gates green");
            Ok(())
        }
        Some((name, e)) => Err(format!("gate `{name}` failed:\n{e}")),
    }
}
