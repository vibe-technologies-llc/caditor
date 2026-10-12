# caditor
Parametric CAD software

## Installing

Each release has an archive for 64-bit Linux; extract it and run `./install.sh`. Its
`INSTALL.md` (`packaging/INSTALL.md`) has the details and the system requirements.

For 64-bit Windows, each release has an installer, `caditor-<version>-windows-x86_64.msi`, which
installs for your account without administrator rights; `packaging/windows/INSTALL.md` has the
details.

## Building

```sh
cargo run --release -p caditor
```

## Third-party assets

caditor bundles the Inter typeface, licensed under the SIL Open Font License 1.1
(`crates/caditor/assets/fonts/Inter-LICENSE.txt`), and the Phosphor icons (MIT,
`crates/caditor/assets/fonts/Phosphor-LICENSE.txt`) through `egui-phosphor`. egui adds its default
fonts (Hack, Ubuntu Light, Noto Emoji and emoji-icon-font), and the Rust libraries it is built from
are mostly MIT, Apache-2.0, BSD, ISC, Zlib or Unicode licensed. Every release carries their
licences in `THIRD-PARTY-NOTICES.txt` beside `LICENSE`; `cargo run --release -p caditor-notices --
OUTPUT` writes it for a checkout.
