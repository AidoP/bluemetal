#!/usr/bin/env -S rustc -o bs -O --edition=2024
use std::cell::RefCell;
use std::ffi::OsString;
use std::path::PathBuf;
use std::path::Path;
use std::io::IsTerminal;
use std::process::ExitCode;

// mod build;
mod dag;
mod db;
mod log;
mod job;
pub mod tool;

pub use tool::{
    Command,
    Rust,
    Phony,
};
pub use job::JobPool;

pub use dag::*;
pub use log::Sev;

pub struct Config {
    program: OsString,
    stream: RefCell<Box<dyn std::io::Write>>,
    color: bool,
    verbose: bool,
    debug: bool,
    /// Allow the build system to rebuild and re-exec itself.
    /// Typically only disabled by the build system to prevent recursive rebuilds.
    bootstrap: bool,

    source_root: PathBuf,
}
impl Config {
    /// Log a message, and return the configured severity of that message.
    ///
    /// If a message is [`Sev::Fatal`] it will not be downgraded.
    pub fn log<F: std::fmt::Display>(&self, id: Option<&str>, sev: Sev, f: F) -> Sev {
        let mut accent = "";
        let mut highlight = "";
        let mut reset = "";
        let mut bold = "";
        let mut normal = "";
        if self.color {
            accent = sev.accent();
            highlight = "\x1b[95m";
            bold = "\x1b[1m";
            normal = "\x1b[22m";
            reset = "\x1b[0m";
        }
        let sev_str = sev.as_str();
        let mut s = self.stream.borrow_mut();
        write!(s, "{accent}{sev_str}").unwrap();
        if let Some(id) = id {
            write!(s, " [{bold}{id}{normal}]").unwrap();
        }
        writeln!(s, ":{reset} {f}").unwrap();
        if self.debug {
            let backtrace = std::backtrace::Backtrace::force_capture();
            writeln!(s, "{highlight}Stack backtrace:{reset}\n{backtrace}").unwrap();
        }
        sev
    }
    pub fn fatal<F: std::fmt::Display>(&self, f: F) {
        let _ = self.log(None, Sev::Fatal, f);
    }
    pub fn error<F: std::fmt::Display>(&self, f: F) {
        let _ = self.log(None, Sev::Error, f);
    }
    pub fn warn<F: std::fmt::Display>(&self, f: F) {
        let _ = self.log(None, Sev::Warning, f);
    }
    pub fn info<F: std::fmt::Display>(&self, f: F) {
        let _ = self.log(None, Sev::Info, f);
    }
}

macro_rules! invalid_usage {
    ($config:expr, $fmt:literal $($args:tt)*) => {
        {
            $config.fatal(format_args!(concat!("invalid usage: ", $fmt) $($args)*));
            $crate::help($config);
        }
    };
}

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or_else(|| "build".into());

    let stream = std::io::stderr();
    let color = stream.is_terminal();

    let mut source_root = Path::new(&program).parent().unwrap_or_else(|| Path::new("")).to_path_buf();
    if source_root.as_path() == "." {
        source_root.clear();
    }

    let mut config = Config {
        program,
        stream: RefCell::new(Box::new(stream)),
        color,
        debug: std::env::var_os("BLUEMETAL_DEBUG").is_some(),
        verbose: false,
        bootstrap: true,
        source_root,
    };

    let mut targets = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_encoded_bytes() {
            b"--no-bootstrap" => {
                config.bootstrap = false;
            },
            b"-v" => {
                config.verbose = true;
            },
            b"-o" => {
                let Some(target) = args.next() else {
                    invalid_usage!(&config, "missing required argument for `-o`");
                    return ExitCode::FAILURE;
                };
                match target.into_string() {
                    Ok(target) => targets.push(target),
                    Err(target) => {
                        invalid_usage!(&config, "invalid target name `{:?}`", target);
                        return ExitCode::FAILURE;
                    },
                };
            },
            _ => {
                invalid_usage!(&config, "unexpected argument {:?}", arg);
                return ExitCode::FAILURE;
            }
        }
    }

    let mut terminal_pool = JobPool::new(&config);

    // TODO: only bootstrap when dirty.
    if config.bootstrap && cfg!(unix) {
        // Rebuild the build system itself first.
        const BUILD_SYSTEM: &Target = PLAN.find("bs").expect("missing required target `build`");
        BUILD_SYSTEM.build(&mut terminal_pool).unwrap();

        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(&config.program).args(std::env::args_os().skip(1)).arg("--no-bootstrap").exec();
        config.error(format_args!("failed to hot-reload: {error}"));
        return ExitCode::FAILURE;
    }

    if targets.is_empty() {
        const DEFAULT: &Target = PLAN.find("default").expect("missing required target `default`");
        todo!("TODO: build all targets in {DEFAULT:#?}");
    } else {
        for name in targets {
            let Some(target) = PLAN.find(&name) else {
                config.error(format_args!("invalid target {name:?} requested"));
                return ExitCode::FAILURE;
            };
            target.build(&mut terminal_pool).unwrap();
        }
    }

    // let db = config.build_root.join("build.db");
    // let db = match db::Db::new(&config) {
    //     Ok(db) => db,
    //     Err(error) => {
    //         eprintln!("failed to access compilation database {db:?}: {error}");
    //         return ExitCode::FAILURE;
    //     }
    // };

    ExitCode::SUCCESS
}

fn help(config: &Config) {
    let program = &config.program;
    let mut s = config.stream.borrow_mut();
    writeln!(s, "Usage: {program}

Options:

    -v
        Verbose. Print operations before performing tasks.

    --dry-run
        Do not build, just print what would have been done.

    -g <group>
        Only build the specified group.

    -o <target>
        Only build the specified target.
",
        program=program.display()
    ).unwrap();
}

pub const PLAN: &'static Plan = &Plan::new([
    Target::new(
        "bs",
        &Rust::new("dev/main.rs")
            .crate_type(tool::rust::CrateType::Bin),
    ),
    Target::new(
        "default",
        &Phony,
    ).depends(&[
        "debug",
    ]),
    Target::new(
        "debug",
        &Command {
            program: "echo",
            args: &[
                "Hello,",
                "World!",
            ],
        },
    ).depends(&[
        "bs",
    ]),
    Target::new(
        "build/example",
        &Rust::new("example.rs")
            .crate_type(tool::rust::CrateType::Bin),
    ),
]);
