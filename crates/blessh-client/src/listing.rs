//! Rows of `blessh scan` and of the interactive list: one server each, as
//! a person reads them or as a script parses them (docs/client.md, "Scan
//! output").

use std::io::{IsTerminal, Write};

use clap::ValueEnum;

/// Output format of `blessh scan`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Columns for reading.
    Human,
    /// Tab-separated: name, RSSI, device.
    Tsv,
    /// One JSON object per line.
    Json,
}

/// A server in range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Advertised name, escaped (`escape`); empty if none is known.
    pub name: String,
    /// Signal strength in dBm.
    pub rssi: i16,
    /// Bluetooth address (Linux) or CoreBluetooth identifier (macOS).
    pub id: String,
}

impl Row {
    pub fn new(name: &str, rssi: i16, id: &str) -> Self {
        Self {
            name: escape(name),
            rssi,
            id: id.to_owned(),
        }
    }
}

/// `name` with control characters and backslashes as `\xHH`: anyone in
/// range can advertise a name, and it must not reach the terminal raw.
pub fn escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_control() || c == '\\' {
            out.push_str(&format!("\\x{:02x}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// Width of the name column: the longest valid server name.
pub const NAME_WIDTH: usize = blessh_core::name::MAX_LEN;

/// How rows look on a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// ANSI colors.
    pub color: bool,
    /// Signal bars, and whether they may use Unicode.
    pub bars: Option<Glyphs>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    Unicode,
    Ascii,
}

impl Style {
    /// No bars, no colors: output that is not a terminal.
    pub const PLAIN: Style = Style {
        color: false,
        bars: None,
    };

    /// The style for standard output: bars and colors on a terminal
    /// (colors unless NO_COLOR is set or TERM is dumb), Unicode bars when
    /// the locale is UTF-8.
    pub fn stdout() -> Style {
        if !std::io::stdout().is_terminal() {
            return Style::PLAIN;
        }
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let dumb = env("TERM").as_deref() == Some("dumb");
        let locale = env("LC_ALL")
            .or_else(|| env("LC_CTYPE"))
            .or_else(|| env("LANG"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let utf8 = locale.contains("utf-8") || locale.contains("utf8");
        Style {
            color: !dumb && env("NO_COLOR").is_none(),
            bars: Some(if utf8 { Glyphs::Unicode } else { Glyphs::Ascii }),
        }
    }

    /// The arrow before the server chosen in the interactive list.
    pub fn arrow(&self) -> &'static str {
        match self.bars {
            Some(Glyphs::Unicode) => "→",
            _ => "->",
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    /// Dimmed text, for the header and the status lines.
    pub fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }
}

/// Signal level from 1 to 4, as Wi-Fi shows it.
pub fn level(rssi: i16) -> usize {
    match rssi {
        r if r >= -60 => 4,
        r if r >= -70 => 3,
        r if r >= -80 => 2,
        _ => 1,
    }
}

fn bars(rssi: i16, glyphs: Glyphs, style: &Style) -> String {
    let n = level(rssi);
    let (on, off): (Vec<&str>, &str) = match glyphs {
        Glyphs::Unicode => (vec!["▂", "▄", "▆", "█"], " "),
        Glyphs::Ascii => (vec!["#"; 4], "."),
    };
    let lit: String = on[..n].concat();
    let color = match n {
        4 | 3 => "32",
        2 => "33",
        _ => "31",
    };
    style.paint(color, &lit) + &off.repeat(4 - n)
}

/// A row for reading: `key` is the interactive list's key column,
/// `id_width` cuts the device identifier, `note` follows it.
pub fn human(
    row: &Row,
    style: &Style,
    key: Option<&str>,
    id_width: Option<usize>,
    note: Option<&str>,
) -> String {
    let mut line = String::new();
    if let Some(k) = key {
        line.push_str(&format!("{k:>2}  "));
    }
    let name = if row.name.is_empty() { "?" } else { &row.name };
    let pad = NAME_WIDTH.saturating_sub(name.chars().count());
    line.push_str(name);
    line.push_str(&" ".repeat(pad));
    line.push_str("  ");
    if let Some(g) = style.bars {
        line.push_str(&bars(row.rssi, g, style));
        line.push(' ');
    }
    line.push_str(&format!("{:>4} dBm  ", row.rssi));
    let id: String = match id_width {
        Some(w) => row.id.chars().take(w).collect(),
        None => row.id.clone(),
    };
    line.push_str(&style.dim(&id));
    if let Some(n) = note {
        line.push_str("  ");
        line.push_str(&style.paint("33", n));
    }
    line
}

pub fn tsv(row: &Row) -> String {
    format!("{}\t{}\t{}", row.name, row.rssi, row.id)
}

/// Parses a line of `tsv`; fields after the third are ignored.
pub fn parse_tsv(line: &str) -> Option<Row> {
    let mut f = line.split('\t');
    let name = f.next()?.to_owned();
    let rssi = f.next()?.parse().ok()?;
    let id = f.next()?.to_owned();
    (!id.is_empty()).then_some(Row { name, rssi, id })
}

pub fn json(row: &Row) -> String {
    // After `escape`, a name holds no control characters; its value in JSON
    // is the escaped name, as in the other formats.
    let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{{\"name\":\"{}\",\"rssi\":{},\"id\":\"{}\"}}",
        q(&row.name),
        row.rssi,
        q(&row.id)
    )
}

/// Prints the rows of `blessh scan`, each as soon as it is known; a row
/// whose name an earlier one had says so.
pub struct Printer {
    format: Format,
    style: Style,
    names: Vec<String>,
}

impl Printer {
    pub fn new(format: Format) -> Self {
        Self {
            format,
            style: Style::stdout(),
            names: Vec::new(),
        }
    }

    pub fn row(&mut self, row: &Row) {
        let line = match self.format {
            Format::Tsv => tsv(row),
            Format::Json => json(row),
            Format::Human => {
                let dup = !row.name.is_empty()
                    && self
                        .names
                        .iter()
                        .any(|n| blessh_core::name::same(n, &row.name));
                human(
                    row,
                    &self.style,
                    None,
                    None,
                    dup.then_some("duplicate name"),
                )
            }
        };
        self.names.push(row.name.clone());
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_control_characters() {
        assert_eq!(escape("my-pi"), "my-pi");
        assert_eq!(escape("a\x1b[2Jb"), "a\\x1b[2Jb");
        assert_eq!(escape("t\tn\n"), "t\\x09n\\x0a");
        assert_eq!(escape("back\\slash"), "back\\x5cslash");
        assert_eq!(escape("c1\u{85}"), "c1\\x85");
        assert_eq!(escape("ünï"), "ünï");
    }

    #[test]
    fn levels() {
        assert_eq!(level(-40), 4);
        assert_eq!(level(-60), 4);
        assert_eq!(level(-61), 3);
        assert_eq!(level(-70), 3);
        assert_eq!(level(-80), 2);
        assert_eq!(level(-81), 1);
        assert_eq!(level(-127), 1);
    }

    #[test]
    fn human_rows() {
        let r = Row::new("my-pi", -55, "AA:BB:CC:00:00:01");
        assert_eq!(
            human(&r, &Style::PLAIN, None, None, None),
            format!("my-pi{}   -55 dBm  AA:BB:CC:00:00:01", " ".repeat(24))
        );
        let s = Style {
            color: false,
            bars: Some(Glyphs::Ascii),
        };
        let r = Row::new("x", -75, "0123456789abcdef");
        assert_eq!(
            human(&r, &s, Some("a"), Some(8), Some("same name as 1")),
            format!(
                " a  x{}  ##..  -75 dBm  01234567  same name as 1",
                " ".repeat(28)
            )
        );
        let s = Style {
            color: false,
            bars: Some(Glyphs::Unicode),
        };
        let r = Row::new("", -50, "id");
        assert!(human(&r, &s, None, None, None).starts_with("?"));
        assert!(human(&r, &s, None, None, None).contains("▂▄▆█  -50 dBm"));
    }

    #[test]
    fn colors_only_when_asked() {
        let r = Row::new("pi", -90, "id");
        let s = Style {
            color: true,
            bars: Some(Glyphs::Unicode),
        };
        let line = human(&r, &s, None, None, None);
        assert!(line.contains("\x1b[31m▂\x1b[0m   "));
        assert!(!human(&r, &Style::PLAIN, None, None, None).contains('\x1b'));
    }

    #[test]
    fn machine_formats() {
        let r = Row::new("my\"pi\x07", -61, "aa:bb");
        assert_eq!(tsv(&r), "my\"pi\\x07\t-61\taa:bb");
        assert_eq!(parse_tsv(&tsv(&r)), Some(r.clone()));
        assert_eq!(
            json(&r),
            "{\"name\":\"my\\\"pi\\\\x07\",\"rssi\":-61,\"id\":\"aa:bb\"}"
        );
        assert_eq!(
            parse_tsv("pi\t-50\tid\tlater field"),
            Some(Row::new("pi", -50, "id"))
        );
        assert_eq!(parse_tsv("\t-50\tid"), Some(Row::new("", -50, "id")));
        assert_eq!(parse_tsv("pi\tloud\tid"), None);
        assert_eq!(parse_tsv("pi\t-50"), None);
    }
}
