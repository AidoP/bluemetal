use std::{io::IsTerminal, path::PathBuf};

mod project;

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
    let mut project = project::Project::new(input.parent().map_or_default(|path| path.into()));
    let file = match project::File::read(input.clone()) {
        Ok(file) => file,
        Err(error) => return Err(Error::Read(input, error)),
    };
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

    let mut build_ninja = NinjaFile::new("build.ninja")?;
    build_ninja.subninja("bootstrap.ninja")?;
    build_ninja.sync_all()?;
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
        }
    }
}

struct NinjaFile {
    path: std::path::PathBuf,
    tmp_path: std::path::PathBuf,
    file: std::io::BufWriter<std::fs::File>,
}
impl NinjaFile {
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> Result<Self, Error> {
        fn new(path: &std::path::Path) -> Result<NinjaFile, Error> {
            let tmp_path = path.with_added_extension("tmp");
            let file = match std::fs::File::create(&tmp_path) {
                Ok(f) => f,
                Err(error) => return Err(Error::Create(path.into(), error)),
            };
            Ok(NinjaFile {
                tmp_path,
                path: path.into(),
                file: std::io::BufWriter::new(file),
            })
        }
        new(path.as_ref())
    }
    pub fn subninja(&mut self, name: &str) -> Result<(), Error> {
        use std::io::Write;
        if let Err(error) = writeln!(self.file, "subninja {name}") {
            return Err(Error::Write(self.path.clone(), error));
        }
        Ok(())
    }
    /// Finalize writing the ninja file.
    ///
    /// Flushes writes and renames the temporary file to the true file.
    pub fn sync_all(&mut self) -> Result<(), Error> {
        if let Err(error) = self.file.get_mut().sync_all() {
            return Err(Error::Write(self.path.clone(), error));
        }
        if let Err(error) = std::fs::rename(&self.tmp_path, &self.path) {
            // last-ditch clean up attempt
            let _ = std::fs::remove_file(&self.tmp_path);
            return Err(Error::Write(self.path.clone(), error));
        }
        Ok(())
    }
}
