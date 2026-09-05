# Glyphst GPUI Kit fork

This is the maintained UI dependency of the sibling `glyphst` workspace.
Upstream is `https://github.com/longbridge/gpui-kit`; the fork is
`https://github.com/glyphst/gpui-kit`.

- Keep `main` a fast-forward-only mirror of `upstream/main`.
- Keep Glyphst patches on `glyphst/main`; merge upstream into that branch.
- Keep application composition, styling workarounds, and keybindings in Glyphst.
  Patch this fork only for private internals or internal event/render lifecycles.
- Keep all GPUI packages on the same pinned Glyphst Zed revision. This fork does
  not use upstream's `gpui-pre` packages. Publish Zed before pinning it here, then
  publish this fork before updating Glyphst's Cargo revision.
- `crates/component` still exports `gpui_component`; `crates/assets` is now
  `gpui-kit-assets`. Preserve the compatibility facade and existing fork tests.

Use the pinned Rust toolchain. Validate changes with:

```sh
cargo fmt --all -- --check
cargo check -p gpui-component -p gpui-kit-assets -p gpui-fps --all-targets --locked
cargo test -p gpui-base -p gpui-component -p gpui-fps --lib --locked
cargo clippy -p gpui-base -p gpui-component -p gpui-kit-assets -p gpui-fps --all-targets --locked -- -D warnings
```

Do not build Nix derivations unless requested. Do not use cua-driver on Niri.
