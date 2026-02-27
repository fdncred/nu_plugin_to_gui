use crate::color_utils::{ansi_16_fg, xterm_256_to_rgb};
use gpui::{rgb, Rgba};

#[derive(Clone, Debug)]
pub(crate) struct AnsiSegment {
    pub text: String,
    pub fg: Option<Rgba>,
    pub bold: bool,
}

pub(crate) fn parse_ansi_segments(input: &str) -> Option<Vec<AnsiSegment>> {
    if !input.contains("\u{1b}[") {
        return None;
    }

    let mut segments = Vec::new();
    let mut buf = String::new();
    let mut current_fg: Option<Rgba> = None;
    let mut current_bold = false;

    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if !buf.is_empty() {
                segments.push(AnsiSegment {
                    text: std::mem::take(&mut buf),
                    fg: current_fg,
                    bold: current_bold,
                });
            }

            i += 2;
            let start = i;
            while i < bytes.len() && bytes[i] != b'm' {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }

            let codes = &input[start..i];
            let mut nums: Vec<u16> = if codes.is_empty() {
                vec![0]
            } else {
                codes
                    .split(';')
                    .filter_map(|s| s.parse::<u16>().ok())
                    .collect()
            };
            if nums.is_empty() {
                nums.push(0);
            }

            let mut idx = 0usize;
            while idx < nums.len() {
                let code = nums[idx];
                match code {
                    0 => {
                        current_fg = None;
                        current_bold = false;
                    }
                    1 => current_bold = true,
                    22 => current_bold = false,
                    39 => current_fg = None,
                    30..=37 | 90..=97 => {
                        current_fg = ansi_16_fg(code as u8);
                    }
                    38 => {
                        if idx + 2 < nums.len() && nums[idx + 1] == 5 {
                            current_fg = Some(xterm_256_to_rgb(nums[idx + 2] as u8));
                            idx += 2;
                        } else if idx + 4 < nums.len() && nums[idx + 1] == 2 {
                            let r = nums[idx + 2] as u32;
                            let g = nums[idx + 3] as u32;
                            let b = nums[idx + 4] as u32;
                            current_fg = Some(rgb((r << 16) | (g << 8) | b));
                            idx += 4;
                        }
                    }
                    _ => {}
                }
                idx += 1;
            }

            i += 1;
            continue;
        }

        if let Some(ch) = input[i..].chars().next() {
            buf.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }

    if !buf.is_empty() {
        segments.push(AnsiSegment {
            text: buf,
            fg: current_fg,
            bold: current_bold,
        });
    }

    Some(segments)
}
