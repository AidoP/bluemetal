use crate::Config;

pub struct JobPool<'a> {
    pub config: &'a Config,
}
impl<'a> JobPool<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self {
            config,
        }
    }
    pub fn run(&self, command: &mut std::process::Command) {
        if self.config.verbose {
            self.config.info(format_args!("running {command:?}"));
        }
        let status = command.status().unwrap();
        if !status.success() {
            self.config.error(format_args!("command failed with {status}"));
        }
    }
}
