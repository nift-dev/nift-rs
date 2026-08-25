//! Nift JSON plumbing.
//!
//! The JSON parser and schema validator live in the native Rust `jsonic` crate
//! (Jsonic++ contract); this module re-exports them under Nift's historical
//! names. `entity` (HTML entity escaping for rendered template output) is a
//! Nift/template concern and stays here.

pub use jsonic::{parse_json, validate_schema};

/// Entity escaping (reference `entity()`): map a short marker to its named
/// entity, or `None` when unknown.
pub fn entity(value: &str) -> Option<&'static str> {
    const ENTITIES: &[(&str, &str)] = &[
        ("`", "&grave;"),
        ("~", "&tilde;"),
        ("!", "&excl;"),
        ("@", "&commat;"),
        ("#", "&num;"),
        ("$", "&dollar;"),
        ("%", "&percnt;"),
        ("^", "&Hat;"),
        ("&", "&amp;"),
        ("*", "&ast;"),
        ("?", "&quest;"),
        ("<", "&lt;"),
        (">", "&gt;"),
        ("(", "&lpar;"),
        (")", "&rpar;"),
        ("[", "&lbrack;"),
        ("]", "&rbrack;"),
        ("{", "&lbrace;"),
        ("}", "&rbrace;"),
        ("-", "&minus;"),
        ("_", "&lowbar;"),
        ("=", "&equals;"),
        ("+", "&plus;"),
        ("|", "&vert;"),
        ("\\", "&bsol;"),
        ("/", "&sol;"),
        (";", "&semi;"),
        (":", "&colon;"),
        ("'", "&apos;"),
        ("\"", "&quot;"),
        (",", "&comma;"),
        (".", "&period;"),
        ("£", "&pound;"),
        ("¥", "&yen;"),
        ("€", "&euro;"),
        ("section", "&sect;"),
        ("+-", "&pm;"),
        ("-+", "&mp;"),
        ("!=", "&ne;"),
        ("<=", "&leq;"),
        (">=", "&geq;"),
        ("->", "&rarr;"),
        ("<-", "&larr;"),
        ("<->", "&harr;"),
        ("==>", "&rArr;"),
        ("<==", "&lArr;"),
        ("<==>", "&hArr;"),
        ("<=!=>", "&nhArr;"),
        ("...", "&hellip;"),
    ];
    for (key, encoded) in ENTITIES {
        if *key == value {
            return Some(encoded);
        }
    }
    None
}
