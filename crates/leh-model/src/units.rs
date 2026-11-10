//! Typed speeds and data amounts with selectable units.
//!
//! Speeds are stored in kbit/s and data caps in MB. The UI shows a number
//! plus a unit chosen from [`RATE_UNITS`] or [`DATA_UNITS`]; a unit typed in
//! the text ("2,5 MB/s") overrides the selected one. Empty means no limit.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unit {
    pub label: &'static str,
    /// How many base units (kbit/s or MB) one of this unit is.
    pub factor: f64,
}

/// Speed units; base is kbit/s. Bytes are 8 bits, prefixes are decimal.
pub const RATE_UNITS: [Unit; 5] = [
    Unit {
        label: "kbps",
        factor: 1.0,
    },
    Unit {
        label: "Mbps",
        factor: 1_000.0,
    },
    Unit {
        label: "Gbps",
        factor: 1_000_000.0,
    },
    Unit {
        label: "KB/s",
        factor: 8.0,
    },
    Unit {
        label: "MB/s",
        factor: 8_000.0,
    },
];
/// Index of Mbps, the unit for a bare number.
pub const RATE_DEFAULT: usize = 1;

/// Data units; base is MB (decimal).
pub const DATA_UNITS: [Unit; 3] = [
    Unit {
        label: "MB",
        factor: 1.0,
    },
    Unit {
        label: "GB",
        factor: 1_000.0,
    },
    Unit {
        label: "TB",
        factor: 1_000_000.0,
    },
];
pub const DATA_DEFAULT: usize = 0;

/// Lowest and highest speed the traffic shaper accepts, in kbit/s.
pub const MIN_KBPS: u32 = 8;
pub const MAX_KBPS: u32 = 10_000_000;
/// Highest data cap, in MB (10 TB).
pub const MAX_MB: u32 = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rate,
    Data,
}

impl Kind {
    pub fn units(self) -> &'static [Unit] {
        match self {
            Self::Rate => &RATE_UNITS,
            Self::Data => &DATA_UNITS,
        }
    }

    fn range(self) -> (u32, u32) {
        match self {
            Self::Rate => (MIN_KBPS, MAX_KBPS),
            Self::Data => (1, MAX_MB),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitError {
    /// Not a number, optionally followed by a known unit.
    Invalid,
    TooSmall,
    TooLarge,
}

/// Words that mean "no limit" (English and Spanish).
fn is_unlimited(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "" | "-"
            | "∞"
            | "off"
            | "none"
            | "unlimited"
            | "no limit"
            | "no cap"
            | "ilimitado"
            | "sin límite"
            | "sin limite"
    )
}

/// Finds the unit a typed suffix names ("Mbps", "mb/s", "KB/s", "g").
/// Rates: an uppercase `B` means bytes, anything else bits ("mbps", "Mbit").
fn match_unit(kind: Kind, suffix: &str) -> Option<usize> {
    let s: String = suffix.chars().filter(|c| !c.is_whitespace()).collect();
    match kind {
        Kind::Data => match s
            .to_lowercase()
            .trim_end_matches("ib")
            .trim_end_matches('b')
        {
            "m" => Some(0),
            "g" => Some(1),
            "t" => Some(2),
            _ => None,
        },
        Kind::Rate => {
            let bytes = s.contains('B') && !s.to_lowercase().contains("bit");
            let l = s.to_lowercase();
            let l = l
                .strip_suffix("/s")
                .or_else(|| l.strip_suffix("ps"))
                .unwrap_or(&l);
            let prefix = l
                .trim_end_matches("its")
                .trim_end_matches("it")
                .trim_end_matches('b');
            match (prefix, bytes) {
                ("k", false) => Some(0),
                ("m", false) => Some(1),
                ("g", false) => Some(2),
                ("k", true) => Some(3),
                ("m", true) => Some(4),
                _ => None,
            }
        }
    }
}

/// Parses `text` with `unit` (an index into `kind.units()`) unless the text
/// names its own unit. Returns the value in base units and the unit used;
/// `None` means no limit.
pub fn parse(kind: Kind, text: &str, unit: usize) -> Result<Option<(u32, usize)>, UnitError> {
    let t = text.trim();
    if is_unlimited(t) {
        return Ok(None);
    }
    let end = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(t.len());
    let (num, suffix) = t.split_at(end);
    if num.is_empty() || num.matches(['.', ',']).count() > 1 {
        return Err(UnitError::Invalid);
    }
    let n: f64 = num
        .replace(',', ".")
        .parse()
        .map_err(|_| UnitError::Invalid)?;
    let unit = if suffix.trim().is_empty() {
        unit.min(kind.units().len() - 1)
    } else {
        match_unit(kind, suffix).ok_or(UnitError::Invalid)?
    };
    let v = (n * kind.units()[unit].factor).round();
    if v <= 0.0 {
        return Ok(None);
    }
    let (min, max) = kind.range();
    if v < f64::from(min) {
        return Err(UnitError::TooSmall);
    }
    if v > f64::from(max) {
        return Err(UnitError::TooLarge);
    }
    Ok(Some((v as u32, unit)))
}

/// "2.50" → "2.5", "3.00" → "3".
fn trim_num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The number to show for `value` (base units) in `unit`, if it is exact
/// there with at most two decimals and at least 0.01.
fn show_in(kind: Kind, value: u32, unit: usize) -> Option<String> {
    let f = kind.units().get(unit)?.factor;
    let shown = f64::from(value) / f;
    let exact = ((shown * 100.0).round() / 100.0 * f).round() == f64::from(value);
    (shown >= 0.01 && exact).then(|| trim_num(shown))
}

/// Text and unit for showing `value`. Keeps `preferred` when the value is
/// exact in it, otherwise picks the largest bit/decimal unit that fits.
pub fn display(kind: Kind, value: u32, preferred: usize) -> (String, usize) {
    if let Some(s) = show_in(kind, value, preferred)
        && f64::from(value) / kind.units()[preferred].factor >= 1.0
    {
        return (s, preferred);
    }
    // Largest first; for speeds only the bit units (bytes are opt-in).
    for u in [2, 1, 0] {
        if f64::from(value) >= kind.units()[u].factor
            && let Some(s) = show_in(kind, value, u)
        {
            return (s, u);
        }
    }
    (value.to_string(), 0)
}

/// Human text with unit, for subtitles: "2.5 Mbps", "1.5 GB".
pub fn format(kind: Kind, value: u32) -> String {
    let (n, u) = display(kind, value, usize::MAX);
    format!("{n} {}", kind.units()[u].label)
}

pub fn format_rate(kbps: u32) -> String {
    format(Kind::Rate, kbps)
}

pub fn format_data(mb: u32) -> String {
    format(Kind::Data, mb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Kind::{Data, Rate};

    #[test]
    fn rates_use_the_selected_unit() {
        assert_eq!(parse(Rate, "5", 1), Ok(Some((5_000, 1))));
        assert_eq!(parse(Rate, "2,5", 1), Ok(Some((2_500, 1))));
        assert_eq!(parse(Rate, "800", 0), Ok(Some((800, 0))));
        assert_eq!(parse(Rate, "1", 4), Ok(Some((8_000, 4))));
        assert_eq!(parse(Rate, "100", 3), Ok(Some((800, 3))));
    }

    #[test]
    fn typed_units_win() {
        assert_eq!(parse(Rate, "2.5 Mbps", 0), Ok(Some((2_500, 1))));
        assert_eq!(parse(Rate, "800k", 1), Ok(Some((800, 0))));
        assert_eq!(parse(Rate, "1 Gbit/s", 1), Ok(Some((1_000_000, 2))));
        assert_eq!(parse(Rate, "2 MB/s", 1), Ok(Some((16_000, 4))));
        assert_eq!(parse(Rate, "2 mb/s", 0), Ok(Some((2_000, 1))));
        assert_eq!(parse(Rate, "50 KB/s", 1), Ok(Some((400, 3))));
        assert_eq!(parse(Data, "1,5 GB", 0), Ok(Some((1_500, 1))));
        assert_eq!(parse(Data, "2g", 0), Ok(Some((2_000, 1))));
    }

    #[test]
    fn empty_and_errors() {
        for t in ["", " ", "0", "Unlimited", "ilimitado", "sin límite"] {
            assert_eq!(parse(Rate, t, 1), Ok(None), "{t:?}");
        }
        assert_eq!(parse(Rate, "5", 0), Err(UnitError::TooSmall));
        assert_eq!(parse(Rate, "50", 2), Err(UnitError::TooLarge));
        assert_eq!(parse(Rate, "fast", 1), Err(UnitError::Invalid));
        assert_eq!(parse(Rate, "5 MB/h", 1), Err(UnitError::Invalid));
        assert_eq!(parse(Rate, "1.2.3", 1), Err(UnitError::Invalid));
        assert_eq!(parse(Data, "lots", 0), Err(UnitError::Invalid));
    }

    #[test]
    fn display_keeps_or_picks_units() {
        assert_eq!(display(Rate, 16_000, 4), ("2".into(), 4));
        assert_eq!(display(Rate, 2_500, 1), ("2.5".into(), 1));
        assert_eq!(display(Rate, 2_500, 0), ("2500".into(), 0));
        assert_eq!(display(Rate, 1_234, 4), ("1234".into(), 0));
        assert_eq!(display(Rate, 1_000_000, 1), ("1000".into(), 1));
        assert_eq!(display(Rate, 1_000_000, usize::MAX), ("1".into(), 2));
        assert_eq!(display(Data, 1_500, 0), ("1500".into(), 0));
        assert_eq!(format_rate(2_500), "2.5 Mbps");
        assert_eq!(format_rate(800), "800 kbps");
        assert_eq!(format_data(1_500), "1.5 GB");
    }

    #[test]
    fn display_round_trips() {
        for kind in [Rate, Data] {
            for v in [8, 256, 800, 1_000, 1_234, 2_500, 12_340, 16_000, 1_500_000] {
                for pref in 0..kind.units().len() {
                    let (text, u) = display(kind, v, pref);
                    assert_eq!(
                        parse(kind, &text, u),
                        Ok(Some((v, u))),
                        "{kind:?} {v} {pref}"
                    );
                }
            }
        }
    }
}
