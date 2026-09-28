//! Built-in terminal commands.
//! Explicit operator shell — no automatic execution, no background jobs.

use std::process::Command;

use chrono::Local;

/// Result of running a single line.
pub struct CommandResult {
    pub stdout: String,
    pub clear: bool,
}

pub fn run_line(line: &str, cwd_display: &str) -> CommandResult {
    let line = line.trim();
    if line.is_empty() {
        return CommandResult {
            stdout: String::new(),
            clear: false,
        };
    }

    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    let args: Vec<&str> = parts.collect();

    match cmd {
        "help" | "?" => CommandResult {
            stdout: help_text(),
            clear: false,
        },
        "clear" | "cls" => CommandResult {
            stdout: String::new(),
            clear: true,
        },
        "date" | "time" => CommandResult {
            stdout: format!("{}\n", Local::now().format("%Y-%m-%d %H:%M:%S %Z")),
            clear: false,
        },
        "whoami" => CommandResult {
            stdout: format!("{}\n", whoami()),
            clear: false,
        },
        "pwd" => CommandResult {
            stdout: format!("{}\n", cwd_display),
            clear: false,
        },
        "echo" => CommandResult {
            stdout: format!("{}\n", args.join(" ")),
            clear: false,
        },
        "status" => CommandResult {
            stdout: status_text(),
            clear: false,
        },
        "uname" => run_external("uname", &["-a"]),
        "uptime" => run_external("uptime", &[]),
        "df" => run_external("df", &["-h"]),
        "free" => run_external("free", &["-h"]),
        "ls" => {
            let path = args.first().copied().unwrap_or(".");
            run_external("ls", &["-la", path])
        }
        "cat" => {
            if args.is_empty() {
                CommandResult {
                    stdout: "usage: cat <file>\n".into(),
                    clear: false,
                }
            } else {
                run_external("cat", &args)
            }
        }
        "ping" => {
            if args.is_empty() {
                CommandResult {
                    stdout: "usage: ping <host>\n".into(),
                    clear: false,
                }
            } else {
                // Short bounded ping
                run_external("ping", &["-c", "3", args[0]])
            }
        }
        "ip" | "ifconfig" => run_external("ip", &["-br", "addr"]),
        "ps" => run_external("ps", &["aux"]),
        "env" => {
            // Only show a few safe vars
            let keys = ["USER", "HOME", "SHELL", "PATH", "LANG"];
            let mut out = String::new();
            for k in keys {
                if let Ok(v) = std::env::var(k) {
                    out.push_str(&format!("{k}={v}\n"));
                }
            }
            CommandResult {
                stdout: out,
                clear: false,
            }
        }
        "version" | "rvn" => CommandResult {
            stdout: "RVN FIELD//OS 0.1.0\nterminal · explicit operator shell\n".into(),
            clear: false,
        },
        // Refuse dangerous patterns early
        "rm" | "mkfs" | "dd" | "shutdown" | "reboot" | "halt" | "poweroff" => CommandResult {
            stdout: format!("refused: '{cmd}' is not enabled in RVN terminal\n"),
            clear: false,
        },
        _ => {
            // Allow a small allow-list style passthrough for simple info commands
            // Everything else is reported as unknown for now (safer default)
            CommandResult {
                stdout: format!(
                    "unknown command: {cmd}\nType 'help' for available commands.\n"
                ),
                clear: false,
            }
        }
    }
}

fn help_text() -> String {
    r#"RVN terminal commands

  help              Show this list
  clear             Clear scrollback
  status            Platform snapshot
  version           RVN version
  date              Current time
  whoami            Current user
  pwd               Working directory label
  echo <text>       Print text
  env               Selected environment vars

  uname             System name
  uptime            System uptime
  df                Disk usage
  free              Memory
  ls [path]         List directory
  cat <file>        Show file
  ip                Addresses
  ping <host>       ICMP (3 packets)
  ps                Processes

Destructive commands are refused by default.
"#
    .into()
}

fn status_text() -> String {
    format!(
        "RVN status\n  time     {}\n  mode     operator shell\n  network  explicit only\n  policy   safe-commands allow-list\n",
        Local::now().format("%H:%M:%S")
    )
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "operator".into())
}

fn run_external(program: &str, args: &[&str]) -> CommandResult {
    match Command::new(program).args(args).output() {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).to_string();
            if !out.stderr.is_empty() {
                text.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            if text.is_empty() {
                text = format!("[{program}] completed with no output\n");
            }
            if !text.ends_with('\n') {
                text.push('\n');
            }
            CommandResult {
                stdout: text,
                clear: false,
            }
        }
        Err(e) => CommandResult {
            stdout: format!("error running {program}: {e}\n"),
            clear: false,
        },
    }
}
