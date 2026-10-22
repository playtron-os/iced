# Kora crates

crates.io rejects git dependencies, so a library built on this fork, such as [ic3d](https://github.com/playtron-os/ic3d), can only be published if the fork is on crates.io too. The upstream names belong to upstream, so this fork and the forks it pins are published as `-kora` crates: `iced-kora`, `iced-core-kora`, `wgpu-kora`, `winit-kora`, `cosmic-text-kora`, `cryoglyph-kora`, `softbuffer-kora`, and so on. Only the package names change. Code keeps its crate names:

```toml
[dependencies]
iced = { package = "iced-kora", version = "0.15" }
```

## Publishing

1. In `stage.py`, bump the version of each fork whose pinned commit changed. `FORKS` lists them in publishing order. winit is pinned by branch, so it changes whenever that branch moves.
2. Run the "Publish Kora crates" workflow. It fetches the forks at the commits `Cargo.toml` pins, stages them in renamed form and publishes them. Versions crates.io already has are skipped, so a failed run can be started again.

To try it locally, stage with `--local`, then build against the staged copies without publishing anything:

```bash
pip install tomlkit
python3 kora/stage.py . /tmp/kora --local
cd /tmp/kora/iced && cargo check -p iced-kora --all-features
```

`publish.py /tmp/kora --dry-run` packages and verifies the crates without uploading. Stage without `--local` first, because `cargo publish` ignores local patches. Crates whose dependencies are not on crates.io yet fail to verify.
