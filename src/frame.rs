use std::io::{self, Write};

pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 80;
pub const PIXELS: usize = WIDTH * HEIGHT;
pub const RGB565_BYTES: usize = PIXELS * 2;
include!(concat!(env!("OUT_DIR"), "/font.rs"));

// Preserve the user's row-major, bit-14-first credits_vfd font for all small values.
fn value_glyph(ch: char) -> ([u8; 5], i32) {
    let ascii = if (' '..='~').contains(&ch) {
        ch as usize
    } else {
        b'?' as usize
    };
    let glyph = FONT_3X5[ascii - 32];
    let rows = std::array::from_fn(|row| ((glyph >> (12 - row * 3)) & 7) as u8);
    let width = match ch {
        '.' => 1,
        ' ' => 2,
        _ => 3,
    };
    (rows, width)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(value: u32) -> Self {
        Self((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    pub fn mix(self, other: Self, amount: f32) -> Self {
        let t = amount.clamp(0.0, 1.0);
        let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Self(
            channel(self.0, other.0),
            channel(self.1, other.1),
            channel(self.2, other.2),
        )
    }

    pub fn scale(self, amount: f32) -> Self {
        Self(
            (self.0 as f32 * amount) as u8,
            (self.1 as f32 * amount) as u8,
            (self.2 as f32 * amount) as u8,
        )
    }

    pub const fn to_565(self) -> u16 {
        ((self.0 as u16 >> 3) << 11) | ((self.1 as u16 >> 2) << 5) | (self.2 as u16 >> 3)
    }

    pub const fn from_565(value: u16) -> Self {
        let r = ((value >> 11) & 31) as u8;
        let g = ((value >> 5) & 63) as u8;
        let b = (value & 31) as u8;
        Self(
            (r << 3) | (r >> 2),
            (g << 2) | (g >> 4),
            (b << 3) | (b >> 2),
        )
    }
}

#[derive(Clone)]
pub struct Frame {
    pub pixels: Vec<Rgb>,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            pixels: vec![Rgb::default(); PIXELS],
        }
    }
}

impl Frame {
    pub fn clear(&mut self, color: Rgb) {
        self.pixels.fill(color);
    }

    pub fn pixel(&mut self, x: i32, y: i32, color: Rgb) {
        if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            self.pixels[y as usize * WIDTH + x as usize] = color;
        }
    }

    /// Blend coverage into one real display pixel; this never filters the whole frame.
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: Rgb, coverage: f32) {
        if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            let pixel = &mut self.pixels[y as usize * WIDTH + x as usize];
            *pixel = pixel.mix(color, coverage);
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Rgb) {
        let left = x.clamp(0, WIDTH as i32) as usize;
        let right = (x + width.max(0)).clamp(0, WIDTH as i32) as usize;
        let top = y.clamp(0, HEIGHT as i32) as usize;
        let bottom = (y + height.max(0)).clamp(0, HEIGHT as i32) as usize;
        for row in top..bottom {
            self.pixels[row * WIDTH + left..row * WIDTH + right].fill(color);
        }
    }

    pub fn line(&mut self, mut x: i32, mut y: i32, end_x: i32, end_y: i32, color: Rgb) {
        let dx = (end_x - x).abs();
        let sx = if x < end_x { 1 } else { -1 };
        let dy = -(end_y - y).abs();
        let sy = if y < end_y { 1 } else { -1 };
        let mut error = dx + dy;
        loop {
            self.pixel(x, y, color);
            if x == end_x && y == end_y {
                break;
            }
            let twice = 2 * error;
            if twice >= dy {
                error += dy;
                x += sx;
            }
            if twice <= dx {
                error += dx;
                y += sy;
            }
        }
    }

    /// Fixed 6x8 glyph cells for process names and values, including digits.
    pub fn text(&mut self, mut x: i32, y: i32, value: &str, color: Rgb) {
        for ch in value.chars() {
            let ascii = if (' '..='~').contains(&ch) {
                ch as usize
            } else {
                b'?' as usize
            };
            let glyph = &FONT[(ascii - 32) * 8..(ascii - 32) * 8 + 8];
            for (row, bits) in glyph.iter().enumerate() {
                for column in 0..6 {
                    if bits & (0x80 >> column) != 0 {
                        self.pixel(x + column, y + row as i32, color);
                    }
                }
            }
            x += 6;
        }
    }

    pub fn text_right(&mut self, right: i32, y: i32, value: &str, color: Rgb) {
        self.text(right - Self::text_width(value), y, value, color);
    }

    pub fn text_width(value: &str) -> i32 {
        value.chars().count() as i32 * 6
    }

    /// Measured ink-cell width, including one blank pixel between each value glyph.
    pub fn value_width(value: &str) -> i32 {
        (value.chars().map(|ch| value_glyph(ch).1 + 1).sum::<i32>() - 1).max(0)
    }

    pub fn value(&mut self, mut x: i32, y: i32, value: &str, color: Rgb) {
        for ch in value.chars() {
            let (rows, width) = value_glyph(ch);
            for (row, bits) in rows.iter().enumerate() {
                for column in 0..width {
                    if bits & (0b100 >> column) != 0 {
                        self.pixel(x + column, y + row as i32, color);
                    }
                }
            }
            x += width + 1;
        }
    }

    pub fn value_right(&mut self, right: i32, y: i32, value: &str, color: Rgb) {
        self.value(right - Self::value_width(value), y, value, color);
    }

    /// Three pixel wide, five pixel high digits. Pitch 3 keeps "100" inside a small ring.
    pub fn digits_3x5(&mut self, mut x: i32, y: i32, value: &str, pitch: i32, color: Rgb) {
        for ch in value.bytes() {
            let glyph = match ch {
                b'0'..=b'9' | b'-' => value_glyph(ch as char).0,
                _ => [0; 5],
            };
            for (row, bits) in glyph.iter().enumerate() {
                for column in 0..3 {
                    if bits & (0b100 >> column) != 0 {
                        self.pixel(x + column, y + row as i32, color);
                    }
                }
            }
            x += pitch;
        }
    }

    pub fn rgb24(&self, destination: &mut [u8]) {
        assert_eq!(destination.len(), PIXELS * 3);
        for (pixel, output) in self.pixels.iter().zip(destination.as_chunks_mut::<3>().0) {
            output.copy_from_slice(&[pixel.0, pixel.1, pixel.2]);
        }
    }

    /// Row-major, RGB565, big endian. These are pixels only, not a serial protocol.
    pub fn rgb565_be(&self, destination: &mut [u8]) {
        assert_eq!(destination.len(), RGB565_BYTES);
        for (pixel, output) in self.pixels.iter().zip(destination.as_chunks_mut::<2>().0) {
            output.copy_from_slice(&pixel.to_565().to_be_bytes());
        }
    }

    pub fn quantize_565(&mut self) {
        for pixel in &mut self.pixels {
            *pixel = Rgb::from_565(pixel.to_565());
        }
    }

    pub fn write_bmp(&self, writer: &mut impl Write, scale: usize) -> io::Result<()> {
        if !(1..=16).contains(&scale) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "BMP scale must be 1..16",
            ));
        }
        let width = WIDTH * scale;
        let height = HEIGHT * scale;
        let row_bytes = (width * 3 + 3) & !3;
        let size = 54 + row_bytes * height;
        let mut header = [0u8; 54];
        header[0..2].copy_from_slice(b"BM");
        header[2..6].copy_from_slice(&(size as u32).to_le_bytes());
        header[10..14].copy_from_slice(&54u32.to_le_bytes());
        header[14..18].copy_from_slice(&40u32.to_le_bytes());
        header[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        header[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        header[26..28].copy_from_slice(&1u16.to_le_bytes());
        header[28..30].copy_from_slice(&24u16.to_le_bytes());
        writer.write_all(&header)?;
        let mut row = vec![0; row_bytes];
        for y in (0..height).rev() {
            for x in 0..width {
                let pixel = self.pixels[(y / scale) * WIDTH + x / scale];
                row[x * 3..x * 3 + 3].copy_from_slice(&[pixel.2, pixel.1, pixel.0]);
            }
            writer.write_all(&row)?;
        }
        Ok(())
    }
}

/// File and device transports share this boundary without changing the renderer.
pub trait FrameSink {
    fn present(&mut self, frame: &Frame) -> io::Result<()>;
}

pub struct Rgb565Writer<W: Write> {
    writer: W,
    scratch: Vec<u8>,
}

impl<W: Write> Rgb565Writer<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            scratch: vec![0; RGB565_BYTES],
        }
    }
}

impl<W: Write> FrameSink for Rgb565Writer<W> {
    fn present(&mut self, frame: &Frame) -> io::Result<()> {
        frame.rgb565_be(&mut self.scratch);
        self.writer.write_all(&self.scratch)
    }
}
