fn main() {
    let machine = configure::Machine::configure();
    machine.bin_args();
    machine.features();
    machine.build_lib("rt", &[
        machine.kernel_entry(),
        "src/trap.s",
    ]);
}
