//! PNG textures of mods, decoded to RGBA for the clients' game hooks.

use std::io::Cursor;

use bhmod_manifest::package::{Source, MAX_TEXTURE_EDGE};

/// Largest PNG file read from a package.
pub const MAX_PNG_BYTES: u64 = 8 * 1024 * 1024;

/// A decoded texture: `width * height` pixels of RGBA, 8 bits per channel, straight (not premultiplied) alpha,
/// first row on top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl Texture {
    /// Decodes PNG bytes (any colour type and depth, palettes included).
    pub fn decode(bytes: &[u8]) -> Result<Texture, String> {
        let mut dec = png::Decoder::new(Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        let (w, h) = (reader.info().width, reader.info().height);
        if w == 0 || h == 0 || w > MAX_TEXTURE_EDGE || h > MAX_TEXTURE_EDGE {
            return Err(format!("{w}x{h} pixels is outside 1..={MAX_TEXTURE_EDGE}"));
        }
        let mut buf = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let raw = &buf[..frame.buffer_size()];
        let rgba = match frame.color_type {
            png::ColorType::Rgba => raw.to_vec(),
            png::ColorType::Rgb => raw.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            png::ColorType::GrayscaleAlpha => raw.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            png::ColorType::Grayscale => raw.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
            png::ColorType::Indexed => return Err("palette was not expanded".to_string()),
        };
        Ok(Texture { width: w, height: h, rgba })
    }

    /// Reads and decodes `path` from a package.
    pub fn read(src: &mut dyn Source, path: &str) -> Result<Texture, String> {
        let bytes = src.read(path, MAX_PNG_BYTES).map_err(|e| format!("{path}: {e}"))?;
        Texture::decode(&bytes).map_err(|e| format!("{path}: {e}"))
    }

    /// The same pixels with colours multiplied by alpha (what the game's own loader hands the GPU).
    pub fn premultiplied(&self) -> Texture {
        let mut rgba = self.rgba.clone();
        for p in rgba.chunks_exact_mut(4) {
            let a = u32::from(p[3]);
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * a + 127) / 255) as u8;
            }
        }
        Texture { width: self.width, height: self.height, rgba }
    }
}

#[cfg(test)]
mod tests;
