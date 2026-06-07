use std::borrow::Cow;
use std::path::PathBuf;
use std::process::ExitCode;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::ops::ControlFlow;

#[macro_use]
mod fmt;
// pub mod args;

pub struct Config {
    /// The name of the program that was executed.
    pub program: OsString,
    pub root: PathBuf,
    pub help: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            program: "bs".into(),
            root: PathBuf::new(),
            help: Vec::new(),
        }
    }
}

fn main() -> ExitCode {
    let mut config = Config::default();
    let mut args = std::env::args_os();
    if let Some(program) = args.next() {
        if let Some(path) = std::path::Path::new(&program).parent() {
            config.root.push(path);
        }
        config.program = program;
    }

    if std::io::stderr().is_terminal() {
        fmt::set_formatted(true);
    }

    // Bootstrap
    let pre_metadata = std::fs::metadata(&config.program).ok();
    match std::process::Command::new("ninja").current_dir(&config.root).args(["--quiet", "bs"]).status() {
        Err(error) => {
            error!("unable to run `ninja`: {}", error);
            return ExitCode::FAILURE;
        },
        Ok(status) if status.success() => {
            let pre_mtime = pre_metadata.map(|m| m.modified().ok()).flatten();
            let metadata = std::fs::metadata(&config.program).ok();
            let mtime = metadata.map(|m| m.modified().ok()).flatten();
            if mtime.is_none() {
                // If the modified time is unknown fall-back to no bootstrapping.
                warning!("unable to determine mtime of {:?}", config.program);
            } else if pre_mtime < mtime {
                // Program was modified, try to restart in place.
                use std::os::unix::process::CommandExt;
                let error = std::process::Command::new(&config.program).args(std::env::args_os().skip(1)).exec();
                error!("unable to re-exec after rebuilding: {}", error);
                return ExitCode::FAILURE;
            }
        },
        Ok(status) => {
            error!("rebuild failed: {}", status);
            return ExitCode::FAILURE;
        },
    }

    let mut commands = vec![&COMMAND];
    let mut current_arg = 0;
    'parse_args: while let Some(arg) = args.next() {
        if arg.as_encoded_bytes().starts_with(b"-") {
            let Some(arg) = arg.to_str() else {
                error!("flag contains non-unicode data: {hl}{arg:?}{reset}", hl=fmt::bright_cyan(), reset=fmt::reset(), arg=arg);
                hint!("pass the value as a seperate argument");
                return ExitCode::FAILURE;
            };
            let mut key = arg;
            let mut value = None;
            if let Some((k, v)) = arg.split_once('=') {
                key = k;
                value = Some(v);
            }

            for &command in commands.iter().rev() {
                for &opt in command.opts {
                    if !opt.names.contains(&key) {
                        continue;
                    }
                    let mut value = value.map(|s| Cow::Borrowed(OsStr::new(s)));
                    if value.is_none() {
                        value = args.next().map(|s| Cow::Owned(s));
                    }
                    let Some(value) = value else {
                        error!("missing value for option: {hl}{key:?}{reset}", hl=fmt::bright_cyan(), reset=fmt::reset(), key=key);
                        return ExitCode::FAILURE;
                    };
                    match (opt.process)(value, &mut config) {
                        ControlFlow::Continue(()) => continue 'parse_args,
                        ControlFlow::Break(code) => return code,
                    }
                }
                for &switch in command.switches {
                    if switch.name != key {
                        continue;
                    }
                    let None = value else {
                        error!("unexpected value for switch: {hl}{key:?}{reset}", hl=fmt::bright_cyan(), reset=fmt::reset(), key=key);
                        return ExitCode::FAILURE;
                    };
                    match (switch.process)(&mut config) {
                        ControlFlow::Continue(()) => continue 'parse_args,
                        ControlFlow::Break(code) => return code,
                    }
                }
                // no match, try parsing as short switches
                if key.starts_with("--") {
                    continue;
                }
                for short in key[1..].chars() {
                    for &switch in command.switches {
                        if switch.short != Some(short) {
                            continue;
                        }
                        let None = value else {
                            error!("unexpected value for switch: {hl}{key:?}{reset}", hl=fmt::bright_cyan(), reset=fmt::reset(), key=key);
                            return ExitCode::FAILURE;
                        };
                        match (switch.process)(&mut config) {
                            ControlFlow::Continue(()) => continue 'parse_args,
                            ControlFlow::Break(code) => return code,
                        }
                    }
                }
            }
            error!("unexpected argument: {hl}{arg:?}{reset}", hl=fmt::bright_cyan(), reset=fmt::reset(), arg=arg);
            return ExitCode::FAILURE;
        } else {
            let &command = commands.last().unwrap();
            if let Some(arg) = arg.to_str() {
                for &subcommand in command.subcommands {
                    if subcommand.name != arg {
                        continue;
                    }
                    commands.push(subcommand);
                    current_arg = 0;
                    continue 'parse_args;
                }
            }
            let value = arg;
            let Some(&arg) = command.args.get(current_arg) else {
                error!("unexpected argument: {hl}{value:?}{reset}", hl=fmt::bright_cyan(), reset=fmt::reset(), value=value);
                return ExitCode::FAILURE;
            };
            if !arg.repeat {
                current_arg += 1;
            }
            match (arg.process)(value, &mut config) {
                ControlFlow::Continue(()) => continue 'parse_args,
                ControlFlow::Break(code) => return code,
            }
        }
    }

    for command in commands {
        if let ControlFlow::Break(code) = (command.process)(&config) {
            return code;
        }
    }

    ExitCode::SUCCESS
}

pub fn help(config: &Config) {
    eprintln!("Usage: {} <subcommand>", config.program.display());
    COMMAND.help(config);
}

struct Command {
    name: &'static str,
    description: &'static str,
    help: &'static str,
    opts: &'static [&'static Opt],
    switches: &'static [&'static Switch],
    args: &'static [&'static Arg],
    subcommands: &'static [&'static Command],
    process: fn(config: &Config) -> ControlFlow<ExitCode>,
}
impl Command {
    pub fn help(&self, _config: &Config) {
        if !self.name.is_empty() {
            eprintln!("    {}", self.name);
        }
        if !self.help.is_empty() {
            for line in self.help.lines() {
                eprintln!("    {}", line);
            }
        }
        if !self.opts.is_empty() || ! self.switches.is_empty() {
            eprintln!("Options:");
        }
        for &opt in self.opts {
            for (i, name) in opt.names.iter().enumerate() {
                if i == opt.names.len() - 1 {
                    eprintln!("    {: <15} {}", name, opt.description);
                } else {
                    eprintln!("    {}", name);
                }
            }
            for line in opt.help.lines() {
                eprintln!("        {}", line);
            }
        }
        if !self.subcommands.is_empty() {
            eprintln!("Subcommands:");
        }
        for &subcommand in self.subcommands {
            eprintln!("    {: <15} {}", subcommand.name, subcommand.description);
            for line in subcommand.help.lines() {
                eprintln!("        {}", line);
            }
        }
    }
}

const COMMAND: Command = Command {
    name: "",
    description: "",
    help: "The bluemetal build system.",
    opts: &[
        &Opt {
            names: &["--ansi-escape"],
            description: "control the use of ANSI escape codes",
            help: "={always,never}\nEnable or disable ANSI escape codes for rich text formattin.\nThe default is to enable only if stderr is attached to a terminal.",
            process: |value, _| {
                match value.to_str() {
                    Some("never") => {
                        fmt::set_formatted(false);
                    },
                    Some("always") => {
                        fmt::set_formatted(true);
                    },
                    _ => {
                        let hl = fmt::bright_cyan();
                        let reset = fmt::reset();
                        error!("invalid value for {hl}--ansi-escape{reset}: {hl}{value:?}{reset}", hl=hl, reset=reset, value=value);
                        hint!("valid values are: {hl}always{reset}, {hl}never{reset}", hl=hl, reset=reset);
                        return ControlFlow::Break(ExitCode::FAILURE);
                    },
                }
                ControlFlow::Continue(())
            },
        },
    ],
    switches: &[],
    args: &[],
    process: |_| ControlFlow::Continue(()),
    subcommands: &[
        &Command {
            name: "help",
            description: "print this help message",
            help: "",
            opts: &[],
            switches: &[],
            args: &[
                &Arg {
                    name: "COMMAND",
                    repeat: true,
                    process: |topic, config| {
                        let Ok(topic) = topic.into_string() else {
                            error!("invalid help topic: {}");
                            error!("unexpected argument: {hl}{value:?}{reset}", hl=fmt::bright_cyan(), reset=fmt::reset(), value=value);
                            ControlFlow::Continue(())
                        };
                        config.help.push(topic.into_string());
                        ControlFlow::Continue(())
                    },
                },
            ],
            subcommands: &[],
            process: |config| {
                help(config);
                ControlFlow::Break(ExitCode::SUCCESS)
            },
        },
        &Command {
            name: "config",
            description: "",
            help: "",
            opts: &[],
            switches: &[],
            args: &[],
            subcommands: &[],
            process: |_| ControlFlow::Continue(()),
        },
        &Command {
            name: "pkg",
            description: "",
            help: "",
            opts: &[],
            switches: &[],
            args: &[
                &Arg {
                    name: "OUTPUT",
                    repeat: false,
                    process: |_, _| {
                        ControlFlow::Continue(())
                    },
                },
            ],
            subcommands: &[],
            process: |_| ControlFlow::Continue(()),
        },
    ],
};

/// Option arguments.
///
/// Examples:
/// - `--name='John Doe'`
/// - `--name 'John Doe'`
/// - `-n 'John Doe'`
struct Opt {
    names: &'static [&'static str],
    description: &'static str,
    help: &'static str,
    process: fn(Cow<OsStr>, &mut Config) -> ControlFlow<ExitCode>,
}

/// Flag arguments.
///
/// Examples:
/// - `--apply`
/// - `-a`
/// - `-ab`
struct Switch {
    name: &'static str,
    short: Option<char>,
    process: fn(&mut Config) -> ControlFlow<ExitCode>,
}

/// Postional arguments.
struct Arg {
    name: &'static str,
    repeat: bool,
    process: fn(OsString, &mut Config) -> ControlFlow<ExitCode>,
}
