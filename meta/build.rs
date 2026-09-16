/// The global build configuration.
struct Build {
    crates: 
}

#[derive(Default)]
enum Edition {
    #[default]
    Edition2024,
}
enum CrateType {
    #[default]
    Lib,
    Bin,
}
struct Crate {
    name: 
    crate_type: CrateType,
}

/// Writer for a ninja build file.
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
