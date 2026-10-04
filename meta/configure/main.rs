use std::path::PathBuf;

mod syntax;

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
    let mut project = syntax::Project::new();
    let file = match syntax::File::read(input.clone()) {
        Ok(file) => file,
        Err(error) => return Err(Error::Read(input, error)),
    };
    project.parse(file);

    let decl = syntax::Decl { span: syntax::Span { file: 0, offset: 0, len: 0 }, current: None };
    let decl2 = syntax::Decl { span: syntax::Span { file: 0, offset: 0, len: 0 }, current: None };
    project.declare("apples".into(), decl);
    project.declare("apples".into(), decl2);
    for d in project.diagnostics {
        eprintln!("diag: {}", d.message);
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
    Parse,
    Stderr(std::io::Error),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::MissingArg(name) => write!(f, "missing required argument `{name}`"),
            Self::Read(path, error) => write!(f, "unable to read {path:?}: {error}"),
            Self::Create(path, error) => write!(f, "failed to create {path:?}: {error}"),
            Self::Write(path, error) => write!(f, "failed to write to {path:?}: {error}"),
            Self::Parse => write!(f, "failed to parse build definition"),
            Self::Stderr(error) => write!(f, "failed to write to stderr: {error}"),
        }
    }
}

struct NinjaFile {
    path: std::path::PathBuf,
    file: std::io::BufWriter<std::fs::File>,
}
impl NinjaFile {
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> Result<Self, Error> {
        fn new(path: &std::path::Path) -> Result<NinjaFile, Error> {
            let file = match std::fs::File::create(path) {
                Ok(f) => f,
                Err(error) => return Err(Error::Create(path.into(), error)),
            };
            Ok(NinjaFile {
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
    pub fn sync_all(&mut self) -> Result<(), Error> {
        if let Err(error) = self.file.get_mut().sync_all() {
            return Err(Error::Write(self.path.clone(), error));
        }
        Ok(())
    }
}
