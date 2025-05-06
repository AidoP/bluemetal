-- Rust Analyzer Configuration for Neovim
--
-- Configures Rust Analyzer to use the options from a profile.


-- Rust Analyzer
lspconfig.rust_analyzer.setup({
    capabilities = capabilities,
    on_attach = on_attach,
    settings = {
        ["rust-analyzer"] = {
            cargo = {
                allTargets = false,
                target = "/home/aidop/projects/bluemetal/configure/build/target/riscv64gc-unknown-bluemetal-elf.json",
                extraEnv = {
                    ["BLUEMETAL_PROFILE"] = "/home/aidop/projects/bluemetal/profile/sifive-fu540.toml",
                },
            },
            check = {
                target = "/home/aidop/projects/bluemetal/configure/build/target/riscv64gc-unknown-bluemetal-elf.json",
            },
        },
    },
})
