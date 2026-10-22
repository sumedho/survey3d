//! Helvetica metrics (from Adobe's AFM files) and WinAnsi encoding.

/// Height of the capitals as a fraction of the font size.
pub const CAP_HEIGHT: f64 = 0.718;
/// Depth of the descenders below the baseline as a fraction of the font size.
pub const DESCENT: f64 = 0.207;

/// Advance widths (thousandths of an em) of the printable ASCII characters 32–126.
#[rustfmt::skip]
const REGULAR: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

#[rustfmt::skip]
const BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// The WinAnsi byte for a character (primes become ASCII quotes; anything
/// without a glyph becomes `?`).
pub fn win_ansi(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '′' => b'\'',
        '″' => b'"',
        '€' => 0x80,
        '…' => 0x85,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '\u{a0}'..='\u{ff}' => c as u8,
        _ => b'?',
    }
}

fn byte_width(b: u8, bold: bool) -> u16 {
    match b {
        32..=126 => {
            let table = if bold { &BOLD } else { &REGULAR };
            table[usize::from(b - 32)]
        }
        0xA0 => 278,
        0xB0 => 400,
        0xB1 | 0xD7 => 584,
        0xB2 | 0xB3 | 0xB9 => 333,
        0xB7 => 278,
        0x91 | 0x92 => {
            if bold {
                278
            } else {
                222
            }
        }
        0x93 | 0x94 => {
            if bold {
                500
            } else {
                333
            }
        }
        0x95 => 350,
        0x85 | 0x97 => 1000,
        _ => 556,
    }
}

/// Width of `text` in the units of `size` (millimetres for a size in millimetres).
pub fn text_width(text: &str, size: f64, bold: bool) -> f64 {
    let units: u32 = text
        .chars()
        .map(|c| u32::from(byte_width(win_ansi(c), bold)))
        .sum();
    f64::from(units) / 1000.0 * size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_and_encoding() {
        assert_eq!(win_ansi('°'), 0xB0);
        assert_eq!(win_ansi('″'), b'"');
        assert_eq!(win_ansi('σ'), b'?');
        assert!((text_width("1°", 10.0, false) - 9.56).abs() < 1e-9);
        assert!(text_width("Wm", 1.0, true) > text_width("Wm", 1.0, false));
    }
}
