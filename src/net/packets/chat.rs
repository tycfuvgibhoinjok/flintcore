use crate::util::bytes::{Reader, Writer};
use crate::util::Result;

pub const MAX_CHAT_LENGTH: usize = 119;

// Characters allowed in Beta 1.7.3 chat, mirroring Poseidon's
// FontAllowedCharacters.allowedCharacters (loaded from font.txt).
// Anything outside this set causes a kick with "Illegal characters in chat"
// (NetServerHandler.java:811-815).
pub const ALLOWED_CHARS: &str =
    " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_'abcdefghijklmnopqrstuvwxyz{|}~⌂ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜø£Ø×ƒáíóúñÑªº¿®¬½¼¡«»";

/// Java `FontAllowedCharacters.allowedCharacters`: the characters of the
/// file `font.txt` of the reference. The colour character `§` is not in it.
pub fn is_allowed_char(c: char) -> bool {
    ALLOWED_CHARS.contains(c)
}

// Character widths per index (index = position in ALLOWED_CHARS + 32),
// mirroring TextWrapper.characterWidths from Poseidon.
const CHARACTER_WIDTHS: [u8; 256] = [
    1, 9, 9, 8, 8, 8, 8, 7, 9, 8, 9, 9, 8, 9, 9, 9, 8, 8, 8, 8, 9, 9, 8, 9, 8, 8, 8, 8, 8, 9, 9, 9,
    4, 2, 5, 6, 6, 6, 6, 3, 5, 5, 5, 6, 2, 6, 2, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 2, 2, 5, 6, 5, 6,
    7, 6, 6, 6, 6, 6, 6, 6, 6, 4, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 4, 6, 4, 6, 6,
    3, 6, 6, 6, 6, 6, 5, 6, 6, 2, 6, 5, 3, 6, 6, 6, 6, 6, 6, 6, 4, 6, 6, 6, 6, 6, 6, 5, 2, 5, 7, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 4, 6, 3, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 4, 6,
    6, 3, 6, 6, 6, 6, 6, 6, 6, 7, 6, 6, 6, 2, 6, 6, 8, 9, 9, 6, 6, 6, 8, 8, 6, 8, 8, 8, 8, 8, 6, 6,
    9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 6, 9, 9, 9, 5, 9, 9,
    8, 7, 7, 8, 7, 8, 8, 8, 7, 8, 8, 7, 9, 9, 6, 7, 7, 7, 7, 7, 9, 6, 7, 8, 7, 6, 6, 9, 7, 6, 7, 1,
];

pub const COLOR_CHAR: char = '\u{00A7}';
pub const CHAT_STRING_LENGTH: usize = 119;
pub const CHAT_WINDOW_WIDTH: i32 = 320;

fn char_width(c: char) -> Option<i32> {
    let pos = ALLOWED_CHARS.chars().position(|x| x == c)?;
    let index = pos + 32;
    if index >= CHARACTER_WIDTHS.len() {
        return None;
    }
    Some(CHARACTER_WIDTHS[index] as i32)
}

/// Wraps chat text into lines, mirroring Poseidon's TextWrapper.wrapText.
/// Each line is at most CHAT_STRING_LENGTH characters and CHAT_WINDOW_WIDTH pixels wide.
/// Color codes (§ + color char) are preserved and re-applied at line starts.
pub fn wrap_text(text: &str) -> Vec<String> {
    let mut out = String::new();
    let mut color_char = 'f';
    let mut line_width: i32 = 0;
    let mut line_length: usize = 0;
    let mut char_iter = text.chars().peekable();

    while let Some(ch) = char_iter.next() {
        // Get the color
        if ch == COLOR_CHAR {
            if let Some(next) = char_iter.next() {
                // We might need a linebreak ... so ugly ;(
                if line_length + 2 > CHAT_STRING_LENGTH {
                    out.push('\n');
                    line_length = 0;
                    if color_char != 'f' && color_char != 'F' {
                        out.push(COLOR_CHAR);
                        out.push(color_char);
                        line_length += 2;
                    }
                }
                color_char = next;
                out.push(COLOR_CHAR);
                out.push(next);
                line_length += 2;
            }
            continue;
        }

        // Figure out if it's allowed
        let width = match char_width(ch) {
            Some(w) => w,
            None => {
                // Invalid character .. skip it.
                continue;
            }
        };

        // See if we need a linebreak
        if line_length + 1 > CHAT_STRING_LENGTH || line_width + width >= CHAT_WINDOW_WIDTH {
            out.push('\n');
            line_length = 0;

            // Re-apply the last color if it isn't the default
            if color_char != 'f' && color_char != 'F' {
                out.push(COLOR_CHAR);
                out.push(color_char);
                line_length += 2;
            }
            line_width = width;
        } else {
            line_width += width;
        }
        out.push(ch);
        line_length += 1;
    }

    // Return it split
    out.split('\n').map(|s| s.to_string()).collect()
}

/// Translates alternate color codes (e.g. '&c' -> '§c').
pub fn translate_alternate_color_codes(alt_color_char: char, text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut out = String::with_capacity(text.len());
    while let Some(c) = chars.next() {
        if c == alt_color_char {
            if let Some(&next) = chars.peek() {
                let lower = next.to_ascii_lowercase();
                if lower.is_ascii_digit()
                    || ('a'..='f').contains(&lower)
                    || ('k'..='o').contains(&lower)
                    || lower == 'r'
                {
                    out.push(COLOR_CHAR);
                    out.push(lower);
                    chars.next();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    pub message: String,
}

impl Chat {
    pub fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            message: reader.read_string(MAX_CHAT_LENGTH)?,
        })
    }

    pub fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.write_string(&self.message, MAX_CHAT_LENGTH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_text_basic() {
        let lines = wrap_text("hello world");
        assert_eq!(lines, vec!["hello world"]);
    }

    #[test]
    fn wrap_text_color_code_preserved() {
        let lines = wrap_text("§ctest");
        assert_eq!(lines, vec!["§ctest"]);
    }

    #[test]
    fn translate_color_codes() {
        assert_eq!(
            translate_alternate_color_codes('&', "&aGreen &cRed &eYellow"),
            "§aGreen §cRed §eYellow"
        );
    }

    #[test]
    fn is_allowed_char_space() {
        assert!(is_allowed_char(' '));
    }

    #[test]
    fn is_allowed_char_emoji_not_allowed() {
        assert!(!is_allowed_char('\u{1F600}'));
    }

    #[test]
    fn is_allowed_char_backtick_not_allowed() {
        assert!(!is_allowed_char('`'));
    }

    #[test]
    fn is_allowed_char_section_is_not_allowed() {
        // The colour character is not in the file `font.txt` of the
        // reference.
        assert!(!is_allowed_char('§'));
    }
}
