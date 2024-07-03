fn main() {
    configure::Profile::load()
        .cfg()
        .library("debug", &[
            "src/debug.s",
        ]);
}
