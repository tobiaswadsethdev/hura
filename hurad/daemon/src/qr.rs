//! The pairing string as a QR code in the terminal, for a phone to scan.
//!
//! Two rows of modules to a character cell: `▀` with its foreground the top
//! module and its background the bottom one. **The colours are explicit**,
//! black and bright white, rather than "the terminal's text on the terminal's
//! background": on a dark theme that would draw the code inverted, which the
//! camera on an iPhone does not reliably read.

use qrcode::{Color, EcLevel, QrCode};

/// Modules of light border around the code. The standard asks for four; two
/// scans fine off a screen and keeps the code inside an 80-column terminal.
const QUIET: i32 = 2;

const FG_DARK: &str = "\x1b[30m";
const FG_LIGHT: &str = "\x1b[97m";
const BG_DARK: &str = "\x1b[40m";
const BG_LIGHT: &str = "\x1b[107m";
const RESET: &str = "\x1b[0m";

/// `text` as lines of terminal output, or `None` if it does not fit in a QR
/// code at all.
pub fn render(text: &str) -> Option<String> {
    let code = QrCode::with_error_correction_level(text, EcLevel::M).ok()?;
    let size = code.width() as i32;
    let dark = |x: i32, y: i32| {
        (0..size).contains(&x)
            && (0..size).contains(&y)
            && code[(x as usize, y as usize)] == Color::Dark
    };
    let mut out = String::new();
    for y in (-QUIET..size + QUIET).step_by(2) {
        for x in -QUIET..size + QUIET {
            out.push_str(if dark(x, y) { FG_DARK } else { FG_LIGHT });
            out.push_str(if dark(x, y + 1) { BG_DARK } else { BG_LIGHT });
            out.push('▀');
        }
        out.push_str(RESET);
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rendering read back into modules: each cell's foreground is the
    /// top one and its background the bottom one.
    fn modules(rendered: &str) -> Vec<Vec<bool>> {
        let mut rows = Vec::new();
        for line in rendered.lines() {
            let (mut top, mut bottom) = (Vec::new(), Vec::new());
            for cell in line.split('▀').filter(|c| c.contains("\x1b[")) {
                if cell.trim_start_matches(RESET).is_empty() {
                    continue;
                }
                top.push(cell.contains(FG_DARK));
                bottom.push(cell.contains(BG_DARK));
            }
            if !top.is_empty() {
                rows.push(top);
                rows.push(bottom);
            }
        }
        rows
    }

    /// The real check: what is printed decodes, with an ordinary QR decoder,
    /// to exactly the pairing string.
    #[test]
    fn what_is_printed_decodes_to_the_pairing_string() {
        let pairing = format!(
            "hura://box.tailnet-name.ts.net:17671/{}#{}",
            "8f3c".repeat(16),
            "d8fa".repeat(16)
        );
        let grid = modules(&render(&pairing).unwrap());
        // Scaled up, as a camera sees it, so the decoder has pixels to find
        // the finder patterns in.
        const SCALE: usize = 4;
        let (h, w) = (grid.len(), grid[0].len());
        let mut img = rqrr::PreparedImage::prepare_from_bitmap(w * SCALE, h * SCALE, |x, y| {
            grid[y / SCALE][x / SCALE]
        });
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1, "one code on the screen");
        let (_, decoded) = grids[0].decode().unwrap();
        assert_eq!(decoded, pairing);
    }

    #[test]
    fn the_border_is_light_and_the_colours_are_explicit() {
        let rendered = render("hura://x:1/t#f").unwrap();
        let grid = modules(&rendered);
        assert!(grid[0].iter().all(|dark| !dark), "the top border is light");
        assert!(grid.iter().all(|row| !row[0]), "the left border is light");
        // Every line resets, so nothing after the code is drawn in its colours.
        assert!(rendered.lines().all(|l| l.ends_with(RESET)));
    }

    #[test]
    fn a_pairing_string_fits_an_80_column_terminal() {
        let long = format!(
            "hura://{}:17671/{}#{}",
            "h".repeat(60),
            "a".repeat(64),
            "b".repeat(64)
        );
        let rendered = render(&long).unwrap();
        let width = rendered.lines().next().unwrap().matches('▀').count();
        assert!(width <= 80, "{width} columns");
    }
}
