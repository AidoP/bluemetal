use std::{io::IsTerminal, path::PathBuf};

mod ninja;
mod project;

use ninja::NinjaFile;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or("configure".into());
    match run(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}: {error}", program.display());
            std::process::ExitCode::FAILURE
        },
    }
}

fn run(mut args: std::env::ArgsOs) -> Result<(), Error> {
    let Some(input) = args.next() else {
        return Err(Error::MissingArg("input"));
    };
    let input = PathBuf::from(input);

    let sysroot = rustc_print("sysroot")?;
    let host = rustc_print("host-tuple")?;

    let mut project = project::Project::new(input.parent().map_or_default(|path| path.into()));
    let file = match project::File::read(input.clone()) {
        Ok(file) => file,
        Err(error) => return Err(Error::Read(input, error)),
    };
    let id = project.declare("sysroot".into(), Some(project::Property::new("sysroot".into(), project::Type::String)));
    project.set("sysroot".into(), project::Value::new(id, project::Data::Str(sysroot)));
    // the target triple crates are compiled for, unless they have a `target` attribute
    let id = project.declare("target".into(), Some(project::Property::new("target".into(), project::Type::String)));
    project.set("target".into(), project::Value::new(id, project::Data::Str(host.clone())));
    // the triple of the machine running the build, for crates such as tools which must run during it
    let id = project.declare("host-target".into(), Some(project::Property::new("host-target".into(), project::Type::String)));
    project.set("host-target".into(), project::Value::new(id, project::Data::Str(host)));
    project.parse(file);
    project.check();

    if !project.diagnostics.is_empty() {
        let mut errors = 0;
        let mut warnings = 0;
        let mut stderr = std::io::stderr().lock();
        let pretty = stderr.is_terminal();
        for diagnostic in &project.diagnostics {
            match diagnostic.severity {
                project::Severity::Error => errors += 1,
                project::Severity::Warning => warnings += 1,
                _ => (),
            }
            if let Err(error) = project.write_diagnostic(&mut stderr, diagnostic, pretty) {
                return Err(Error::Stderr(error));
            }
        }
        let result = Error::Diagnostics { errors, warnings };
        if errors > 0 {
            return Err(result);
        } else if warnings > 0 {
            let diagnostic = project::Diagnostic {
                span: None,
                message: project::Message::User(result.to_string()),
                severity: project::Severity::Warning,
            };
            if let Err(error) = project.write_diagnostic(&mut stderr, &diagnostic, pretty) {
                return Err(Error::Stderr(error));
            }
        }
    }

    let depfile_path = std::path::Path::new("build.ninja.d");
    let mut depfile = match std::fs::File::create(depfile_path) {
        Ok(file) => file,
        Err(error) => return Err(Error::Create(depfile_path.into(), error)),
    };
    if let Err(error) = write_depfile(&mut depfile, &project) {
        return Err(Error::Write(depfile_path.into(), error));
    }

    let mut build_ninja = NinjaFile::new("build.ninja")?;
    build_ninja.subninja("bootstrap.ninja")?;

    build_ninja.variable("edition", "2024")?;
    build_ninja.rule("rustc", &[
        ("command", "rustc --color=always --out-dir=target/tmp --emit=link=$out --emit=dep-info=$depfile --crate-type=$crate-type --crate-name=$crate-name --edition=$edition $flags $in"),
        ("description", "RUSTC $out"),
        ("deps", "gcc"),
        ("depfile", "$out.d"),
    ])?;

    let mut crates: Vec<_> = project.crates.into_values().collect();
    crates.sort_unstable_by(|lhs, rhs| (&lhs.name, &lhs.target).cmp(&(&rhs.name, &rhs.target)));
    for c in crates {
        let output = PathBuf::from(format!("target/{}/lib{}.rlib", c.target, c.name));
        let flags = format!("--target={}", c.target);
        build_ninja.build(&ninja::Build {
            rule: "rustc",
            outputs: &[&output],
            inputs: &[&c.path],
            variables: &[("crate-name", &c.name), ("crate-type", "lib"), ("flags", &flags)],
            ..Default::default()
        })?;
    }

    build_ninja.sync_all()?;

    Ok(())
}

fn write_depfile(w: &mut dyn std::io::Write, project: &project::Project) -> Result<(), std::io::Error> {
    write!(w, "build.ninja:")?;
    for file in &project.files {
        write!(w, " {}", file.path.display())?;
    }
    writeln!(w)?;
    Ok(())
}

enum Error {
    MissingArg(&'static str),
    Read(std::path::PathBuf, std::io::Error),
    Create(std::path::PathBuf, std::io::Error),
    Write(std::path::PathBuf, std::io::Error),
    Diagnostics {
        errors: usize,
        warnings: usize,
    },
    Stderr(std::io::Error),
    InvalidOutput(std::process::Command),
    Exec(std::process::Command, std::io::Error),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::MissingArg(name) => write!(f, "missing required argument `{name}`"),
            Self::Read(path, error) => write!(f, "unable to read {path:?}: {error}"),
            Self::Create(path, error) => write!(f, "failed to create {path:?}: {error}"),
            Self::Write(path, error) => write!(f, "failed to write to {path:?}: {error}"),
            Self::Diagnostics { errors: 0, warnings: 1 } => write!(f, "configuration raised 1 warning"),
            Self::Diagnostics { errors: 1, warnings: 0 } => write!(f, "configuration failed with 1 error"),
            Self::Diagnostics { errors: 0, warnings } => write!(f, "configuration raised {warnings} warnings"),
            Self::Diagnostics { errors, warnings: 0 } => write!(f, "configuration failed with {errors} errors"),
            Self::Diagnostics { errors, warnings } => {
                let error_suffix = if *errors > 1 { "s" } else { "" };
                let warning_suffix = if *warnings > 1 { "s" } else { "" };
                write!(f, "configuration failed with {errors} error{error_suffix}, {warnings} warning{warning_suffix}")
            },
            Self::Stderr(error) => write!(f, "failed to write to stderr: {error}"),
            Self::InvalidOutput(cmd) => write!(f, "invalid output from command {cmd:?}"),
            Self::Exec(cmd, error) => write!(f, "failed to execute {cmd:?}: {error}"),
        }
    }
}

/// Returns the trimmed output of `rustc --print=<request>`.
fn rustc_print(request: &str) -> Result<String, Error> {
    use std::process::Command;
    let mut cmd = Command::new("rustc");
    cmd.arg(format!("--print={request}"));
    let output = match cmd.output() {
        Err(error) => {
            return Err(Error::Exec(cmd, error));
        },
        Ok(output) => output,
    };
    if !output.status.success() {
        return Err(Error::InvalidOutput(cmd));
    }
    match String::from_utf8(output.stdout) {
        Ok(value) if !value.trim().is_empty() => Ok(value.trim().into()),
        _ => Err(Error::InvalidOutput(cmd)),
    }
}
