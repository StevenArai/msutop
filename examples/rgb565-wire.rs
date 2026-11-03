//! Export two deterministic wire frames for comparison with msu_host.rgb565.
use msutop::{rgb565::Encoder, Frame, Rgb, RGB565_BYTES};
use std::io::Write;

fn main() -> std::io::Result<()> {
    let path = std::env::args().nth(1).expect("output wire path");
    let mut output = std::fs::File::create(path)?;
    let mut frame = Frame::default();
    let mut encoder = Encoder::default();
    for number in [1_u16, 2] {
        for (index, pixel) in frame.pixels.iter_mut().enumerate() {
            let hi = (index * 2 + number as usize) as u8;
            let lo = (index * 2 + 1 + number as usize) as u8;
            *pixel = Rgb::from_565(u16::from_be_bytes([hi, lo]));
        }
        assert_eq!(frame.pixels.len() * 2, RGB565_BYTES);
        output.write_all(encoder.encode(&frame, 7, number))?;
    }
    Ok(())
}
