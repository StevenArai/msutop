use crate::Rgb;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    Default,
    Vivid,
}

impl ThemeKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "default" => Some(Self::Default),
            "vivid" => Some(Self::Vivid),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Vivid => "vivid",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Gradient(pub Rgb, pub Rgb, pub Rgb);

impl Gradient {
    pub const fn hex(start: u32, mid: u32, end: u32) -> Self {
        Self(Rgb::hex(start), Rgb::hex(mid), Rgb::hex(end))
    }
    pub fn at(self, amount: f32) -> Rgb {
        let t = amount.clamp(0.0, 1.0);
        if t <= 0.5 {
            self.0.mix(self.1, t * 2.0)
        } else {
            self.1.mix(self.2, t * 2.0 - 1.0)
        }
    }
}

/// Palette verified against btop's built-in Default_theme at fd1d2920e5442045f89b8991cca4c3c503ff48f6.
#[derive(Clone, Copy)]
pub struct Theme {
    pub background: Rgb,
    pub text: Rgb,
    pub title: Rgb,
    pub muted: Rgb,
    pub grid: Rgb,
    pub plot_grid: Rgb,
    pub network_activity: Rgb,
    pub memory_track: Rgb,
    pub swap_track: Rgb,
    pub cpu_box: Rgb,
    pub mem_box: Rgb,
    pub net_box: Rgb,
    pub proc_box: Rgb,
    pub cpu: Gradient,
    pub temperature: Gradient,
    pub used: Gradient,
    pub cached: Gradient,
    pub free: Gradient,
    pub available: Gradient,
    pub download: Gradient,
    pub upload: Gradient,
    pub process: Gradient,
}

impl Theme {
    pub fn new(kind: ThemeKind) -> Self {
        Self {
            background: Rgb::hex(0x000000),
            text: Rgb::hex(0xcccccc),
            title: Rgb::hex(0xeeeeee),
            muted: Rgb::hex(0x606060),
            grid: Rgb::hex(0x202020),
            plot_grid: Rgb::hex(0x505050),
            network_activity: Rgb::hex(0xffa64d),
            memory_track: Rgb::hex(0x4a584f),
            swap_track: Rgb::hex(0x4b514d),
            cpu_box: Rgb::hex(0x556d59),
            mem_box: Rgb::hex(0x6c6c4b),
            net_box: Rgb::hex(0x5c588d),
            proc_box: Rgb::hex(0x805252),
            cpu: match kind {
                ThemeKind::Default => Gradient::hex(0x77ca9b, 0xcbc06c, 0xdc4c4c),
                ThemeKind::Vivid => Gradient::hex(0x50f095, 0xf2e266, 0xfa1e1e),
            },
            temperature: Gradient::hex(0x4897d4, 0x5474e8, 0xff40b6),
            used: Gradient::hex(0x592b26, 0xd9626d, 0xff4769),
            cached: Gradient::hex(0x163350, 0x74e6fc, 0x26c5ff),
            free: Gradient::hex(0x384f21, 0xb5e685, 0xdcff85),
            available: Gradient::hex(0x4e3f0e, 0xffd77a, 0xffb814),
            download: Gradient::hex(0x291f75, 0x4f43a3, 0xb0a9de),
            upload: Gradient::hex(0x620665, 0x7d4180, 0xdcafde),
            process: Gradient::hex(0x80d0a3, 0xdcd179, 0xd45454),
        }
    }
}
