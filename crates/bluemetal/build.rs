fn main() {
    let machine = configure::Machine::configure();
    machine.bin_args();
    machine.features();
}
