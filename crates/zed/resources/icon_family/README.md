# Omega icon family

Pinned OpenAgents Desktop icon inputs and the generated package outputs used by
macOS, Windows, Linux, Flatpak, and Snap.

## Channel badges

**Deferred.** Every release channel currently ships the unbadged OpenAgents
Desktop icon. Distinct channel badges are not generated until OpenAgents
approves them.

## Regenerate

```sh
script/generate-omega-icon-family
```

The script verifies the pinned source digests, rewrites the package PNG/ICO/ICNS
outputs, and refreshes `manifest.json`.

## Package consumers

| Surface | Path |
| --- | --- |
| macOS `cargo-bundle` | `crates/zed/Cargo.toml` `package.metadata.bundle-*` icon lists |
| macOS document type | `crates/zed/resources/Document.icns` via `script/bundle-mac` |
| Linux / FreeBSD | `script/bundle-linux` copies 512 and 1024 PNGs |
| Flatpak | `script/flatpak/bundle-flatpak` installs channel PNG |
| Snap | `script/snap-build` copies `app-icon.png` |
| Windows | `crates/windows_resources` embeds channel ICO files |
| About window | `crates/zed/src/zed.rs` embeds channel PNGs |
