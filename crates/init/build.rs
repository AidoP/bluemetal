fn main() {
    let profile = configure::Profile::load();
    profile.cfg()
        .libinit();
}
