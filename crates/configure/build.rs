
fn main() {
    println!("cargo::rerun-if-env-changed=MACHINE");
    let machine = option_env!("MACHINE").unwrap_or("sifive_fu540");

    println!("cargo::rustc-env=CONFIGURE_MACHINE={machine}");
}
