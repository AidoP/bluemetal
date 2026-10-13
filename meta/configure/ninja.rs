use super::Error;

pub struct NinjaFile {
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
    pub fn rule(&mut self) -> Result<(), Error> {
        use std::io::Write;
        if let Err(error) = write!(self.file, "rule TODO
    command = TODO
") {
            return Err(Error::Write(self.path.clone(), error));
        }
        Ok(())
    }
    pub fn build(&mut self, rule: &str, outputs: &[&str]) -> Result<(), Error> {
        use std::io::Write;

        if let Err(error) = write!(self.file, "build TODO: {rule} TODO
    command = TODO
") {
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
