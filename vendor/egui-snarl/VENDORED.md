# Vendored egui-snarl 0.12.0

Source: crates.io `egui-snarl` 0.12.0 (MIT OR Apache-2.0), sources unchanged.
Only `Cargo.toml` differs: `egui` is depended on with
`default-features = false` (the upstream manifest enables egui's
`default_fonts`, which pulls OFL / UFL fonts that `deny.toml` does not allow),
and the example and dev-dependencies were dropped.

Drop this folder and the `[patch.crates-io]` entry of the root `Cargo.toml`
once upstream makes the egui features optional.

`egui-scale` 0.6.1 (MIT OR Apache-2.0, a dependency of egui-snarl) is vendored for
the same reason: sources unchanged, only `egui` became `default-features = false`.
