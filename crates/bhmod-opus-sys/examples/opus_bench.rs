// SPDX-License-Identifier: GPL-3.0-or-later
//! Times 20 ms voice frames: `cargo run --release -p bhmod-opus-sys --example opus_bench [bitrate] [complexity]`.

use bhmod_opus_sys::{Application, Decoder, Encoder, FRAME_SAMPLES, MAX_PACKET};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let bitrate: i32 = args.next().map_or(24_000, |a| a.parse().unwrap());
    let complexity: u8 = args.next().map_or(5, |a| a.parse().unwrap());
    let frames = 3_000; // 60 s of audio
                        // A swept tone with an envelope, closer to speech than a steady sine.
    let pcm: Vec<i16> = (0..frames * FRAME_SAMPLES)
        .map(|i| {
            let t = i as f64 / 48_000.0;
            let f = 150.0 + 100.0 * (t * 3.0).sin();
            ((t * f * std::f64::consts::TAU).sin() * (t * 4.0).sin().abs() * 12_000.0) as i16
        })
        .collect();
    let mut enc = Encoder::new(Application::Voip).unwrap();
    enc.set_bitrate(bitrate).unwrap();
    enc.set_complexity(complexity).unwrap();
    enc.set_inband_fec(true).unwrap();
    enc.set_packet_loss_perc(10).unwrap();
    let mut dec = Decoder::new().unwrap();
    let mut packets = Vec::with_capacity(frames);
    let t = Instant::now();
    for f in pcm.chunks(FRAME_SAMPLES) {
        let mut out = [0u8; MAX_PACKET];
        let n = enc.encode(f, &mut out).unwrap();
        packets.push(out[..n].to_vec());
    }
    let enc_t = t.elapsed();
    let mut buf = [0i16; FRAME_SAMPLES];
    let t = Instant::now();
    for p in &packets {
        dec.decode(p, &mut buf).unwrap();
    }
    let dec_t = t.elapsed();
    let t = Instant::now();
    for _ in 0..frames {
        dec.decode_lost(&mut buf).unwrap();
    }
    let plc_t = t.elapsed();
    let per = |d: std::time::Duration| d.as_secs_f64() * 1e6 / frames as f64;
    let bytes: usize = packets.iter().map(Vec::len).sum();
    println!("{} | {bitrate} bps, complexity {complexity}, {frames} frames", bhmod_opus_sys::version());
    println!(
        "encode {:.1} us/frame, decode {:.1} us/frame, plc {:.1} us/frame, avg packet {:.1} B",
        per(enc_t),
        per(dec_t),
        per(plc_t),
        bytes as f64 / frames as f64
    );
}
