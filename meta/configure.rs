mod syntax;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let program = std::env::args_os().next().unwrap_or("configure".into());
            eprintln!("{}: {error}", program.display());
            std::process::ExitCode::FAILURE
        },
    }
}

fn run() -> Result<(), Error> {
    let mut build_ninja = NinjaFile::new("build.ninja")?;
    build_ninja.subninja("bootstrap.ninja")?;
    build_ninja.sync_all()?;
    Ok(())
}

enum Error {
    Create(std::path::PathBuf, std::io::Error),
    Write(std::path::PathBuf, std::io::Error),
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::Create(path, error) => write!(f, "failed to create {path:?}: {error}"),
            Self::Write(path, error) => write!(f, "failed to write to {path:?}: {error}"),
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

struct ProjectJson {
    path: std::path::PathBuf,
    file: std::io::BufWriter<std::fs::File>,
}
