// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

/// `frames` 20 ms frames of a 440 Hz sine at about -10 dBFS.
fn sine(frames: usize) -> Vec<i16> {
    (0..frames * FRAME_SAMPLES)
        .map(|i| ((i as f64 * 2.0 * std::f64::consts::PI * 440.0 / 48_000.0).sin() * 10_000.0) as i16)
        .collect()
}

fn energy(x: &[i16]) -> f64 {
    x.iter().map(|&s| f64::from(s) * f64::from(s)).sum()
}

fn encode_all(enc: &mut Encoder, pcm: &[i16]) -> Vec<Vec<u8>> {
    pcm.chunks(FRAME_SAMPLES)
        .map(|f| {
            let mut out = [0u8; MAX_PACKET];
            let n = enc.encode(f, &mut out).unwrap();
            out[..n].to_vec()
        })
        .collect()
}

#[test]
fn version_is_fixed_point_1_5() {
    assert!(version().starts_with("libopus 1.5"), "{}", version());
    assert!(version().contains("fixed"), "{}", version());
}

#[test]
fn sine_roundtrip() {
    let input = sine(50);
    let mut enc = Encoder::new(Application::Voip).unwrap();
    enc.set_bitrate(32_000).unwrap();
    let mut dec = Decoder::new().unwrap();
    let mut output = Vec::new();
    for p in encode_all(&mut enc, &input) {
        let mut buf = [0i16; MAX_FRAME_SAMPLES];
        let n = dec.decode(&p, &mut buf).unwrap();
        assert_eq!(n, FRAME_SAMPLES);
        output.extend_from_slice(&buf[..n]);
    }
    // Output lags the input by the lookahead; compare a stretch after the codec settled.
    let delay = enc.lookahead();
    assert!(delay > 0 && delay < FRAME_SAMPLES, "{delay}");
    let a = &input[4800..input.len() - FRAME_SAMPLES];
    let b = &output[4800 + delay..input.len() - FRAME_SAMPLES + delay];
    let dot: f64 = a.iter().zip(b).map(|(&x, &y)| f64::from(x) * f64::from(y)).sum();
    let corr = dot / (energy(a) * energy(b)).sqrt();
    assert!(corr > 0.9, "correlation {corr}");
}

#[test]
fn plc_and_fec() {
    let input = sine(30);
    let mut enc = Encoder::new(Application::Voip).unwrap();
    enc.set_bitrate(24_000).unwrap();
    enc.set_inband_fec(true).unwrap();
    enc.set_packet_loss_perc(20).unwrap();
    assert!(enc.inband_fec());
    assert_eq!(enc.packet_loss_perc(), 20);
    let packets = encode_all(&mut enc, &input);
    let mut dec = Decoder::new().unwrap();
    let mut buf = [0i16; FRAME_SAMPLES];
    for p in &packets[..20] {
        dec.decode(p, &mut buf).unwrap();
    }
    // Packet 20 lost: conceal it, which keeps the tone going.
    assert_eq!(dec.decode_lost(&mut buf).unwrap(), FRAME_SAMPLES);
    assert!(energy(&buf) > 0.0);
    dec.decode(&packets[21], &mut buf).unwrap();
    // Packet 22 lost: recover it from 23's FEC, then decode 23.
    assert_eq!(dec.decode_fec(&packets[23], &mut buf).unwrap(), FRAME_SAMPLES);
    assert!(energy(&buf) > 0.0);
    assert_eq!(dec.decode(&packets[23], &mut buf).unwrap(), FRAME_SAMPLES);
    // PLC / FEC need room for a whole frame.
    let mut small = [0i16; 10];
    assert_eq!(dec.decode_lost(&mut small), Err(Error(ffi::OPUS_BUFFER_TOO_SMALL)));
    dec.reset();
}

#[test]
fn cbr_bitrate_sets_packet_size() {
    let input = sine(10);
    let mut enc = Encoder::new(Application::Voip).unwrap();
    enc.set_vbr(false).unwrap();
    assert!(!enc.vbr());
    for bps in [8_000, 24_000, 64_000] {
        enc.set_bitrate(bps).unwrap();
        assert_eq!(enc.bitrate(), bps);
        enc.reset();
        let sizes: Vec<usize> = encode_all(&mut enc, &input).iter().map(Vec::len).collect();
        assert!(sizes.iter().all(|&n| n == bps as usize / 400), "{bps}: {sizes:?}");
    }
}

#[test]
fn dtx_silence_is_tiny() {
    let mut enc = Encoder::new(Application::Voip).unwrap();
    enc.set_dtx(true).unwrap();
    assert!(enc.dtx());
    let silence = vec![0i16; 30 * FRAME_SAMPLES];
    let sizes: Vec<usize> = encode_all(&mut enc, &silence).iter().map(Vec::len).collect();
    assert!(sizes[20..].iter().all(|&n| n <= 2), "{sizes:?}");
    assert!(enc.in_dtx());
}

#[test]
fn settings_and_errors() {
    for app in [Application::Voip, Application::Audio, Application::LowDelay] {
        let mut enc = Encoder::new(app).unwrap();
        enc.set_complexity(5).unwrap();
        assert_eq!(enc.complexity(), 5);
        let err = enc.set_complexity(11).unwrap_err();
        assert_eq!(err.code(), ffi::OPUS_BAD_ARG);
        assert!(err.to_string().contains("invalid argument"), "{err}");
        let _: &dyn std::error::Error = &err;
        assert_eq!(enc.set_bitrate(0), Err(Error(ffi::OPUS_BAD_ARG)));
        // Wrong frame length, then no output buffer.
        let mut out = [0u8; MAX_PACKET];
        assert_eq!(enc.encode(&[0; 480], &mut out), Err(Error(ffi::OPUS_BAD_ARG)));
        assert_eq!(enc.encode(&sine(1), &mut []), Err(Error(ffi::OPUS_BAD_ARG)));
    }
    assert_eq!(Encoder::create(12_345, 1, ffi::OPUS_APPLICATION_VOIP).unwrap_err().code(), ffi::OPUS_BAD_ARG);
    assert_eq!(Decoder::create(12_345, 1).unwrap_err().code(), ffi::OPUS_BAD_ARG);
    let mut dec = Decoder::new().unwrap();
    let mut buf = [0i16; FRAME_SAMPLES];
    assert_eq!(dec.decode(&[0xff, 0xff, 0xff], &mut buf).unwrap_err().code(), ffi::OPUS_INVALID_PACKET);
}

#[test]
fn send() {
    fn is_send<T: Send>() {}
    is_send::<Encoder>();
    is_send::<Decoder>();
}
