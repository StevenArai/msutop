use std::{env, fs, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let font = root.join("assets/font6x8.h");
    println!("cargo:rerun-if-changed={}", font.display());
    let source = fs::read_to_string(font).expect("6x8 font source");
    let mut bytes = Vec::new();
    for line in source
        .lines()
        .filter(|line| line.trim_start().starts_with("{0x"))
    {
        let row = line
            .split('}')
            .next()
            .unwrap()
            .trim()
            .trim_start_matches('{');
        let glyph: Vec<_> = row
            .split(',')
            .map(|v| u8::from_str_radix(v.trim().trim_start_matches("0x"), 16).expect("font byte"))
            .collect();
        assert_eq!(glyph.len(), 8, "each glyph has eight rows");
        bytes.extend(glyph);
    }
    assert_eq!(bytes.len(), 95 * 8, "ASCII 0x20..0x7e, exactly 760 bytes");
    assert!(bytes.iter().all(|b| b & 3 == 0), "bit 7 is the left pixel");

    let small_font = root.join("assets/font3x5.inc");
    println!("cargo:rerun-if-changed={}", small_font.display());
    let small_source = fs::read_to_string(small_font).expect("credits_vfd 3x5 font source");
    let glyphs: Vec<_> = small_source
        .lines()
        .filter(|line| line.trim_start().starts_with("0x"))
        .flat_map(|line| line.split(','))
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            u16::from_str_radix(value.trim().trim_start_matches("0x"), 16).expect("3x5 glyph")
        })
        .collect();
    assert_eq!(glyphs.len(), 95, "3x5 ASCII 0x20..0x7e");
    assert!(
        glyphs.iter().all(|g| g & 0x8000 == 0),
        "each glyph is 15 bits"
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("font.rs");
    fs::write(
        output,
        format!(
            "pub const FONT: [u8; 760] = {bytes:?};\npub const FONT_3X5: [u16; 95] = {glyphs:?};\n"
        ),
    )
    .unwrap();

    if env::var_os("CARGO_FEATURE_SDL").is_some()
        && env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
    {
        let lib = root.join(".deps/SDL2-2.32.10/x86_64-w64-mingw32/lib");
        if lib.exists() {
            println!("cargo:rustc-link-search=native={}", lib.display());
        }
    }
}
