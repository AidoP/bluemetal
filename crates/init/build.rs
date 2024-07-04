fn main() {
    let config = configure::Config::load();
    config.cfg();

    use configure::profile::Machine;
    match config.profile().machine {
        Machine::QemuVirt | Machine::SifiveU540 => {
            config.library("rt", &[
                "src/riscv/init.s",
                "src/riscv/trap.s",
            ]);
        },
    }
}
