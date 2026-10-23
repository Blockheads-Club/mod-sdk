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
        // EXPAND turned palettes into RGB(A) and low bit depths into 8 bits, so what is left is 1 to 4 samples.
        let rgba = match frame.color_type.samples() {
            4 => raw.to_vec(),
            3 => raw.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            2 => raw.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            _ => raw.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
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

/// The `pixelFormat` of [`Gpu`]: the game's `RGBA4444` (what its own loader uses for every texture).
pub const PIXEL_FORMAT: u32 = 1;

/// A texture as the game's `CPTexture2D` wants it: edges rounded up to powers of two, colours premultiplied by
/// alpha, 4 bits per channel, plus the size of the picture inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    /// Padded width (a power of two).
    pub wide: u32,
    /// Padded height (a power of two).
    pub high: u32,
    /// The picture's width.
    pub content_wide: u32,
    /// The picture's height.
    pub content_high: u32,
    /// `wide * high * 2` bytes: one little-endian `u16` per pixel, `r << 12 | g << 8 | b << 4 | a` (premultiplied
    /// RGBA4444); the picture is in the top-left corner, the padding is transparent.
    pub pixels: Vec<u8>,
}

impl Texture {
    /// The pixels in the form `CPTexture2D initWithData:pixelFormat:pixelsWide:pixelsHigh:contentSize:mipmap:`
    /// takes them with [`PIXEL_FORMAT`].
    pub fn for_gpu(&self) -> Gpu {
        let (wide, high) = (self.width.next_power_of_two(), self.height.next_power_of_two());
        let src = self.premultiplied();
        let mut pixels = vec![0; wide as usize * high as usize * 2];
        for y in 0..self.height as usize {
            for x in 0..self.width as usize {
                let p = &src.rgba[(y * self.width as usize + x) * 4..][..4];
                let v = u16::from(p[0] >> 4) << 12
                    | u16::from(p[1] >> 4) << 8
                    | u16::from(p[2] >> 4) << 4
                    | u16::from(p[3] >> 4);
                let to = (y * wide as usize + x) * 2;
                pixels[to..to + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
        Gpu { wide, high, content_wide: self.width, content_high: self.height, pixels }
    }
}

#[cfg(test)]
mod tests;
