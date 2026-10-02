# caditor
Parametric CAD software

## Installing

Each release has an archive for 64-bit Linux; extract it and run `./install.sh`. Its
`INSTALL.md` has the details and the system requirements.

## Building

```sh
cargo run --release -p caditor
```

## Third-party assets

caditor bundles the Inter typeface, licensed under the SIL Open Font License 1.1
(`crates/caditor/assets/fonts/Inter-LICENSE.txt`), and the Phosphor icons through
`egui-phosphor` (MIT or Apache-2.0).
