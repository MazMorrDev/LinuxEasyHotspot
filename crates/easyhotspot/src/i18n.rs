//! Interface translations.
//!
//! Strings are written in English in the code and wrapped in [`tr`] /
//! [`tr_n`]. Translations live in standard gettext files under `po/`
//! (edit them with Poedit or any text editor); they are embedded into the
//! binary at build time, so nothing extra has to be installed.

use std::collections::HashMap;
use std::sync::OnceLock;

use leh_model::Language;

/// Embedded catalogs: (language code, .po source).
const CATALOGS: &[(&str, &str)] = &[("es", include_str!("../../../po/es.po"))];

struct Catalog {
    singular: HashMap<String, &'static str>,
    plural: HashMap<String, (&'static str, &'static str)>,
}

static ACTIVE: OnceLock<Option<Catalog>> = OnceLock::new();

/// Two-letter code the app will use: the setting, or the system language.
pub fn resolve(lang: Language) -> &'static str {
    match lang {
        Language::English => "en",
        Language::Spanish => "es",
        Language::System => {
            if system_locale().starts_with("es") {
                "es"
            } else {
                "en"
            }
        }
    }
}

/// The desktop's language, e.g. "es_AR" or "es-AR".
fn system_locale() -> String {
    let env = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty());
    // LANGUAGE may be a list ("es_AR:es:en"); the first entry wins.
    if let Some(env) = env {
        return env.split(':').next().unwrap_or("").to_owned();
    }
    // Windows has no locale variables; ask the system.
    #[cfg(windows)]
    if let Some(l) = sys_locale::get_locale() {
        return l;
    }
    String::new()
}

/// Picks the catalog. Call once, before building any UI.
pub fn init(lang: Language) {
    let code = resolve(lang);
    let cat = CATALOGS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, src)| parse(src));
    let _ = ACTIVE.set(cat);
}

/// Marks a string for translation without translating it (for constants);
/// pass the value through [`tr`] where it is shown.
pub const fn n_(s: &'static str) -> &'static str {
    s
}

/// Translates a UI string (English in, current language out).
pub fn tr(s: &'static str) -> &'static str {
    ACTIVE
        .get()
        .and_then(Option::as_ref)
        .and_then(|c| c.singular.get(s).copied())
        .unwrap_or(s)
}

/// Translates a string with a count; `{n}` is replaced by `n`.
pub fn tr_n(singular: &'static str, plural: &'static str, n: usize) -> String {
    let (one, many) = ACTIVE
        .get()
        .and_then(Option::as_ref)
        .and_then(|c| c.plural.get(singular).copied())
        .unwrap_or((singular, plural));
    // English and Spanish share the rule: 1 is singular, everything else plural.
    let s = if n == 1 { one } else { many };
    s.replace("{n}", &n.to_string())
}

/// Translates a message built at run time (e.g. by `leh-model`). A number
/// in it is matched as `{n}`, so "… (reason 9)." finds "… (reason {n}).".
pub fn tr_dyn(s: &str) -> String {
    let Some(cat) = ACTIVE.get().and_then(Option::as_ref) else {
        return s.to_owned();
    };
    if let Some(t) = cat.singular.get(s) {
        return (*t).to_owned();
    }
    if let Some(start) = s.find(|c: char| c.is_ascii_digit()) {
        let end = s[start..]
            .find(|c: char| !c.is_ascii_digit())
            .map_or(s.len(), |e| start + e);
        let key = format!("{}{{n}}{}", &s[..start], &s[end..]);
        if let Some(t) = cat.singular.get(&key) {
            return t.replace("{n}", &s[start..end]);
        }
    }
    s.to_owned()
}

/// Translates and fills `{name}`-style placeholders.
pub fn trf(s: &'static str, args: &[(&str, &str)]) -> String {
    let mut out = tr(s).to_owned();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

fn unquote(line: &str) -> Option<String> {
    let inner = line.trim().strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

#[derive(Default)]
struct Entry {
    id: String,
    plural_id: Option<String>,
    strs: Vec<String>,
    fuzzy: bool,
}

/// Minimal .po reader: msgid / msgid_plural / msgstr / msgstr[n],
/// continuation lines, `#, fuzzy` (skipped) and comments.
fn parse(src: &'static str) -> Catalog {
    let mut cat = Catalog {
        singular: HashMap::new(),
        plural: HashMap::new(),
    };
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let mut entries = Vec::new();
    let mut cur = Entry::default();
    // Which field continuation lines append to: 0 id, 1 plural id, 2+ msgstr[n-2].
    let mut field = 0usize;
    let mut fuzzy_next = false;
    for line in src.lines() {
        let l = line.trim();
        if l.starts_with("#,") {
            fuzzy_next |= l.contains("fuzzy");
            continue;
        }
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix("msgid_plural ") {
            cur.plural_id = unquote(rest);
            field = 1;
        } else if let Some(rest) = l.strip_prefix("msgid ") {
            entries.push(std::mem::take(&mut cur));
            cur.id = unquote(rest).unwrap_or_default();
            cur.fuzzy = std::mem::take(&mut fuzzy_next);
            field = 0;
        } else if let Some(rest) = l.strip_prefix("msgstr[") {
            let (idx, val) = rest.split_once(']').unwrap_or(("0", rest));
            let idx: usize = idx.parse().unwrap_or(0);
            if cur.strs.len() <= idx {
                cur.strs.resize(idx + 1, String::new());
            }
            cur.strs[idx] = unquote(val).unwrap_or_default();
            field = 2 + idx;
        } else if let Some(rest) = l.strip_prefix("msgstr ") {
            cur.strs = vec![unquote(rest).unwrap_or_default()];
            field = 2;
        } else if let Some(more) = unquote(l) {
            match field {
                0 => cur.id.push_str(&more),
                1 => {
                    if let Some(p) = cur.plural_id.as_mut() {
                        p.push_str(&more);
                    }
                }
                n => {
                    if let Some(s) = cur.strs.get_mut(n - 2) {
                        s.push_str(&more);
                    }
                }
            }
        }
    }
    entries.push(cur);
    for e in entries {
        // The header has an empty msgid; untranslated entries fall back to English.
        if e.id.is_empty() || e.fuzzy || e.strs.iter().all(String::is_empty) {
            continue;
        }
        if e.plural_id.is_some() {
            let one = e.strs.first().cloned().unwrap_or_default();
            let many = e.strs.get(1).cloned().unwrap_or_else(|| one.clone());
            cat.plural.insert(e.id, (leak(one), leak(many)));
        } else {
            cat.singular.insert(e.id, leak(e.strs[0].clone()));
        }
    }
    cat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_po() {
        let c = parse(
            r#"
msgid ""
msgstr "Content-Type: text/plain; charset=UTF-8\n"

#: app.rs
msgid "Hotspot is off"
msgstr "El punto de acceso está apagado"

msgid ""
"Long "
"text"
msgstr ""
"Texto "
"largo"

#, fuzzy
msgid "Guess"
msgstr "Adivinanza"

msgid "Untranslated"
msgstr ""

msgid "{n} device"
msgid_plural "{n} devices"
msgstr[0] "{n} dispositivo"
msgstr[1] "{n} dispositivos"

msgid "Say \"hi\""
msgstr "Di \"hola\""
"#,
        );
        assert_eq!(
            c.singular["Hotspot is off"],
            "El punto de acceso está apagado"
        );
        assert_eq!(c.singular["Long text"], "Texto largo");
        assert!(!c.singular.contains_key("Guess"));
        assert!(!c.singular.contains_key("Untranslated"));
        assert!(!c.singular.contains_key(""));
        assert_eq!(
            c.plural["{n} device"],
            ("{n} dispositivo", "{n} dispositivos")
        );
        assert_eq!(c.singular["Say \"hi\""], "Di \"hola\"");
    }

    #[test]
    fn bundled_catalog_is_consistent() {
        for (code, src) in CATALOGS {
            let c = parse(src);
            assert!(c.singular.len() > 50, "{code}: too few strings");
            // Placeholders must survive translation.
            for (id, t) in &c.singular {
                for ph in [
                    "{n}",
                    "{name}",
                    "{version}",
                    "{iface}",
                    "{source}",
                    "{rate}",
                ] {
                    assert_eq!(id.contains(ph), t.contains(ph), "{code}: {ph} in {id:?}");
                }
            }
        }
    }
}
