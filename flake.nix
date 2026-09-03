{
    description = "Bluemetal";

    inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    outputs = { self, nixpkgs }: let
        system = "x86_64-linux";
        pkgs = nixpkgs.legacyPackages.${system};
    in {
        devShells.${system}.default = pkgs.mkShell.override { stdenv = pkgs.clangStdenv; } {
            buildInputs = with pkgs; [
                ccache
                cmake
                hercules
                ninja
                python3
                python3Packages.python-lsp-server
                ruff
                rustup
            ];

            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
                pkgs.stdenv.cc.cc.lib
                pkgs.zlib
            ];

            hardeningDisable = [ "all" ];
        };
    };
}
