{
    pkgs ? import <nixpkgs> { },
}: let
    overrides = (builtins.fromTOML (builtins.readFile ../rust-toolchain.toml));
in pkgs.mkShell {
    nativeBuildInputs = with pkgs; [
        # (pkgs.callPackage ./package.nix { })
    ];

    buildInputs = with pkgs; [
        llvmPackages.bintools
        ninja
        qemu
        rustup
    ];

    env = {
        RUSTC_VERSION = overrides.toolchain.channel;
    };
}
