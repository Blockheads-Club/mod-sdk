# Third-party notices

Code from the projects below is included in this repository. Their licences are compatible with the licences of the
parts that use them (see [LICENSING.md](LICENSING.md)); their notices are reproduced or referenced here.

## libopus 1.5.2 (BSD-3-Clause)

`crates/bhmod-opus-sys/opus/` is an unmodified subset of the official libopus 1.5.2 release
(`opus-1.5.2.tar.gz`, SHA-256 `65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1`), built into the
voice chat of the Blockheads Club hosts. Its full copyright notice and licence are in
[`crates/bhmod-opus-sys/opus/COPYING`](crates/bhmod-opus-sys/opus/COPYING), which must accompany binary
distributions:

> Copyright 2001-2023 Xiph.Org, Skype Limited, Octasic, Jean-Marc Valin, Timothy B. Terriberry, CSIRO, Gregory
> Maxwell, Mark Borgerding, Erik de Castro Lopo, Mozilla, Amazon
>
> Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
> conditions in `COPYING` are met (retain the copyright notice, the list of conditions and the disclaimer; do not use
> the names of the copyright holders to endorse derived products without permission).

Opus is subject to royalty-free patent licences; see <https://opus-codec.org/license/>.

## Rust crates

Dependencies (wasmtime, mlua with its vendored Luau, jni, serde, zip, cc, ...) are used under their own licences;
see `Cargo.lock` and each crate's metadata.
