use super::*;

fn encode(w: u32, h: u32, color: png::ColorType, depth: png::BitDepth, data: &[u8], palette: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(color);
    enc.set_depth(depth);
    if let Some(p) = palette {
        enc.set_palette(p.to_vec());
    }
    enc.write_header().unwrap().write_image_data(data).unwrap();
    out
}

#[test]
fn decodes_every_colour_type() {
    use png::{BitDepth::*, ColorType::*};
    let rgba = encode(2, 1, Rgba, Eight, &[1, 2, 3, 4, 5, 6, 7, 8], None);
    let t = Texture::decode(&rgba).unwrap();
    assert_eq!((t.width, t.height, t.rgba), (2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]));
    let rgb = encode(1, 1, Rgb, Eight, &[9, 8, 7], None);
    assert_eq!(Texture::decode(&rgb).unwrap().rgba, [9, 8, 7, 255]);
    let ga = encode(1, 1, GrayscaleAlpha, Eight, &[10, 20], None);
    assert_eq!(Texture::decode(&ga).unwrap().rgba, [10, 10, 10, 20]);
    let g = encode(2, 1, Grayscale, Eight, &[30, 40], None);
    assert_eq!(Texture::decode(&g).unwrap().rgba, [30, 30, 30, 255, 40, 40, 40, 255]);
    let pal = encode(1, 1, Indexed, Eight, &[1], Some(&[0, 0, 0, 11, 22, 33]));
    assert_eq!(Texture::decode(&pal).unwrap().rgba, [11, 22, 33, 255]);
    let deep = encode(1, 1, Rgb, Sixteen, &[0xAB, 0xCD, 0x12, 0x34, 0xFF, 0x00], None);
    assert_eq!(Texture::decode(&deep).unwrap().rgba, [0xAB, 0x12, 0xFF, 255]);
}

#[test]
fn rejects_bad_files() {
    use png::{BitDepth::Eight, ColorType::Grayscale};
    assert!(Texture::decode(b"not a png").is_err());
    let huge = encode(1025, 1, Grayscale, Eight, &vec![0; 1025], None);
    assert!(Texture::decode(&huge).unwrap_err().contains("outside"));
    let mut cut = encode(4, 4, Grayscale, Eight, &[0; 16], None);
    cut.truncate(cut.len() - 20);
    assert!(Texture::decode(&cut).is_err());
}

#[test]
fn premultiplies() {
    let t = Texture { width: 3, height: 1, rgba: vec![255, 128, 0, 255, 255, 255, 255, 0, 200, 100, 50, 128] };
    let p = t.premultiplied();
    assert_eq!(p.rgba, [255, 128, 0, 255, 0, 0, 0, 0, 100, 50, 25, 128]);
    assert_eq!((p.width, p.height), (3, 1));
}

#[test]
fn reads_from_a_package() {
    use bhmod_manifest::package::DirSource;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let png = encode(1, 1, png::ColorType::Rgba, png::BitDepth::Eight, &[1, 2, 3, 4], None);
    std::fs::write(dir.path().join("assets/a.png"), png).unwrap();
    std::fs::write(dir.path().join("assets/bad.png"), b"nope").unwrap();
    let mut src = DirSource::open(dir.path()).unwrap();
    assert_eq!(Texture::read(&mut src, "assets/a.png").unwrap().rgba, [1, 2, 3, 4]);
    assert!(Texture::read(&mut src, "assets/bad.png").unwrap_err().starts_with("assets/bad.png: "));
    assert!(Texture::read(&mut src, "assets/none.png").unwrap_err().starts_with("assets/none.png: "));
}

#[test]
fn gpu_form_pads_to_powers_of_two() {
    // Two opaque pixels (red, 0x80 grey) and one half-transparent white in a 3x2 picture.
    let t = Texture {
        width: 3,
        height: 2,
        rgba: [255, 0, 0, 255, 128, 128, 128, 255, 255, 255, 255, 128, 0, 0, 0, 0, 0, 0, 255, 255, 16, 32, 48, 255]
            .to_vec(),
    };
    let g = t.for_gpu();
    assert_eq!((g.wide, g.high, g.content_wide, g.content_high), (4, 2, 3, 2));
    assert_eq!(g.pixels.len(), 4 * 2 * 2);
    let px = |x: usize, y: usize| u16::from_le_bytes([g.pixels[(y * 4 + x) * 2], g.pixels[(y * 4 + x) * 2 + 1]]);
    assert_eq!(px(0, 0), 0xF00F);
    assert_eq!(px(1, 0), 0x888F);
    // Half-transparent white, premultiplied: 128 -> 0x8, alpha 128 -> 0x8.
    assert_eq!(px(2, 0), 0x8888);
    // The padding is transparent.
    assert_eq!((px(3, 0), px(3, 1)), (0, 0));
    assert_eq!(px(0, 1), 0x0000);
    assert_eq!(px(1, 1), 0x00FF);
    assert_eq!(px(2, 1), 0x123F);
    // Already a power of two: no padding.
    let one = Texture { width: 1, height: 1, rgba: vec![255, 0, 0, 128] };
    assert_eq!(one.for_gpu().pixels, 0x8008u16.to_le_bytes());
}
