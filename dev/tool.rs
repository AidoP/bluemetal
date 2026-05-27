use crate::{
    JobPool,
    Target,
};

pub mod rust;
pub use rust::Rust;

pub trait Tool : std::fmt::Debug {
    fn build(&self, target: &Target, pool: &mut JobPool) -> Result<(), ()>;
}

#[derive(Debug)]
pub struct Phony;
impl Tool for Phony {
    fn build(&self, target: &Target, pool: &mut JobPool) -> Result<(), ()> { Ok(()) }
}

#[derive(Debug)]
pub struct Command {
    pub program: &'static str,
    pub args: &'static [&'static str],
}
impl Tool for Command {
    fn build(&self, target: &Target, pool: &mut JobPool) -> Result<(), ()> {
        use std::process::Command;
        let mut command = Command::new(self.program);
        command.args(self.args);
        pool.run(&mut command);
        Ok(())
    }
}
