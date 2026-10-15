use std::{io::IsTerminal, path::{Path, PathBuf}};

mod check;
mod database;
mod generate;
mod ninja;
mod project;
mod rust_analyzer;
mod target_spec;
mod test_runner;

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
    if input == "--check" {
        let Some(label) = args.next() else {
            return Err(Error::MissingArg("label"));
        };
        check::check(&label.to_string_lossy());
        return Ok(());
    }
    if input == "--test" {
        let names: Vec<_> = args.map(|name| name.to_string_lossy().into_owned()).collect();
        return test_runner::test(&names);
    }
    let input = PathBuf::from(input);

    let sysroot = rustc_print("sysroot")?;
    let host = rustc_print("host-tuple")?;
    let targets = rustc_print("target-list")?.lines().map(String::from).collect();

    let mut project = project::Project::new(input.parent().map_or_default(|path| path.into()), targets);
    let file = match project::File::read(input.clone()) {
        Ok(file) => file,
        Err(error) => return Err(Error::Read(input, error)),
    };
    let builtins = [
        (
            project::Property::new("sysroot".into(), project::Type::String)
                .info("the sysroot of the Rust toolchain"),
            sysroot.clone(),
        ),
        (
            project::Property::new("target".into(), project::Type::String)
                .info("the target triple for crates without a `target` attribute"),
            host.clone(),
        ),
        (
            project::Property::new("host-target".into(), project::Type::String)
                .info("the target triple of the machine running the build")
                .read_only(),
            host.clone(),
        ),
    ];
    for (property, value) in builtins {
        let name = property.name().to_string();
        let id = project.declare(name.clone(), Some(property));
        project.set(name, project::Value::new(id, project::Data::Str(value)));
    }
    project.parse(file);
    // the configure tool builds itself, and runs its tests, when its source is part of the project
    let self_hosted = Path::new(CONFIGURE_SOURCE).exists();
    if self_hosted {
        // the root build file, as the crates are not declared by one
        let span = project::Span { file: 0, offset: 0, len: 0 };
        let implicit = |name: &str, path: &str, kind, output: Option<&str>| project::rust::Crate {
            name: name.into(),
            name_span: span,
            path: path.into(),
            path_span: span,
            target: host.clone(),
            target_span: span,
            kind,
            output: output.map(PathBuf::from),
            builtin: false,
            implicit: true,
            default: false,
            test: true,
            uses: Vec::new(),
        };
        let configure = implicit("configure", CONFIGURE_SOURCE, project::rust::Kind::Bin, Some(CONFIGURE));
        project.add_implicit_crate(configure, Vec::new());
        // the integration tests run the configure tool
        let tests = implicit("test_dev_configure", "test/dev/configure/main.rs", project::rust::Kind::Test, None);
        project.add_implicit_crate(tests, vec![CONFIGURE.into()]);
    }
    project.check();

    report_diagnostics(&project)?;
    generate::write(project, &input, &sysroot, self_hosted)
}

/// The configure tool, which is built by `build.ninja` from its source when that is part of the project.
const CONFIGURE: &str = "target/configure";
const CONFIGURE_SOURCE: &str = "dev/configure/main.rs";

/// Write the diagnostics of a project to stderr, failing if there are errors.
///
/// Warnings alone do not fail, but are followed by a warning which counts them.
fn report_diagnostics(project: &project::Project) -> Result<(), Error> {
    if project.diagnostics.is_empty() {
        return Ok(());
    }
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
    TestBuild,
    TargetSpec(String, &'static str),
    UnknownTest(String),
    TestsFailed(usize),
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
            Self::TestBuild => write!(f, "failed to build the tests"),
            Self::TargetSpec(name, error) => write!(f, "failed to generate the specification of target {name:?}: {error}"),
            Self::UnknownTest(name) => write!(f, "no test named {name:?}"),
            Self::TestsFailed(1) => write!(f, "1 test failed"),
            Self::TestsFailed(count) => write!(f, "{count} tests failed"),
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
