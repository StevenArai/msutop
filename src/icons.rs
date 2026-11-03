use crate::{Frame, Rgb};

/// Crisp pixel symbols sized for their fixed resource-header slots.
#[derive(Clone, Copy, Debug)]
pub enum Icon {
    Memory,
    Swap,
    Ethernet,
    Plug,
    TriangleDown,
    TriangleUp,
}

impl Icon {
    pub fn draw(self, frame: &mut Frame, x: i32, y: i32, color: Rgb) {
        let (width, rows): (i32, &[u16]) = match self {
            Self::Memory => (
                9,
                &[
                    0b111111111,
                    0b101101101,
                    0b101101101,
                    0b111111111,
                    0b010101010,
                ],
            ),
            Self::Swap => (
                9,
                &[
                    0b001000100,
                    0b011100100,
                    0b101010101,
                    0b001001110,
                    0b001000100,
                ],
            ),
            Self::Ethernet => (
                9,
                &[
                    0b111111111,
                    0b101010101,
                    0b100000001,
                    0b100000001,
                    0b110000011,
                    0b010000010,
                    0b011111110,
                ],
            ),
            Self::Plug => (
                9,
                &[
                    0b000111000,
                    0b001111111,
                    0b111111000,
                    0b001111111,
                    0b000111000,
                ],
            ),
            Self::TriangleDown => (5, &[0b11111, 0b01110, 0b00100]),
            Self::TriangleUp => (5, &[0b00100, 0b01110, 0b11111]),
        };
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..width {
                if bits & (1 << (width - 1 - column)) != 0 {
                    frame.pixel(x + column, y + row as i32, color);
                }
            }
        }
    }
}
