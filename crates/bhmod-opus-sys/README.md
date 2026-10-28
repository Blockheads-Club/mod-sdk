# bhmod-opus-sys

**Licence: GPL-3.0-or-later** for our Rust code and `build.rs`; the vendored libopus in `opus/` is **BSD-3-Clause**
([`opus/COPYING`](opus/COPYING), see [`../../THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md)).

Opus for the voice chat of the PC host and the Android loader (`notes/voice-transport.md` §5): the official libopus,
compiled by `build.rs` with `cc` (no cmake, no bindgen), so it cross-builds with nothing more than the C compiler the
host build already uses (the NDK clang for `armv7-linux-androideabi`).

| Part | What |
| --- | --- |
| `ffi` | hand-written declarations: encoder/decoder create/destroy/encode/decode/ctl, the CTL request numbers |
| `Encoder` | 48 kHz mono, 20 ms frames (`FRAME_SAMPLES` = 960 `i16`); bitrate, VBR, complexity, in-band FEC, packet loss %, DTX, lookahead, reset |
| `Decoder` | `decode`, `decode_lost` (PLC), `decode_fec` (recover the previous lost frame from in-band FEC), reset |

## Build configuration

`FIXED_POINT`, float API kept, no deep PLC / DRED / OSCE (the `dnn/` tree is not vendored), C99 VLAs (`alloca` on
MSVC). SIMD without run-time detection:

| Target arch | Defines |
| --- | --- |
| `arm` (armeabi-v7a) | NEON intrinsics presumed (`-mfpu=neon`, the NDK default since r21) + ARMv5E/v6 inline asm for the fixed-point multiplies |
| `aarch64` | NEON intrinsics presumed |
| `x86_64` | SSE, SSE2 presumed (SSE4.1/AVX2 would need RTCD; not built) |
| others | portable C |

Source lists are read from libopus' own `celt_sources.mk`, `silk_sources.mk` and `opus_sources.mk`.

## Vendored tree

`opus/` is `opus-1.5.2.tar.gz` from <https://downloads.xiph.org/releases/opus/> (same file as the GitHub release),
SHA-256 `65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1` as published in `SHA256SUMS.txt`, reduced
to `celt/ silk/ src/ include/`, `COPYING AUTHORS README NEWS package_version` and the `celt_*`, `silk_*`, `opus_*`
`.mk` files, minus `celt/tests`, `silk/tests`, `meson.build` files and the demo programs (`src/opus_demo.c`,
`src/opus_compare.c`, `src/repacketizer_demo.c`, `celt/opus_custom_demo.c`). No file is modified. To update, repeat
that with the new release, verify its published SHA-256, and update the hash here and in the notices.

`cargo run --release -p bhmod-opus-sys --example opus_bench [bitrate] [complexity]` times encode / decode / PLC.
