//! Expression names — port of `ExpressionName` and the name-flag syntax used
//! throughout the XML definition files (`ExpressionItem.h`).
//!
//! In the data files a `<names>` element is a comma-separated list where each
//! entry may carry a flag prefix ending in `:` — for example
//! `ar:m,meter,p:meters` or `a-cr:USD,au:€`. Flags:
//!
//! | flag | meaning |
//! |------|---------|
//! | `a`  | abbreviation |
//! | `r`  | reference name (never translated) |
//! | `p`  | plural form |
//! | `u`  | unicode form |
//! | `s`  | suffix |
//! | `c`  | case sensitive |
//! | `i`  | avoid input |
//! | `-`  | negates the flags that follow it |

/// One name of an expression item, with its flags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExpressionName {
    pub name: String,
    pub abbreviation: bool,
    pub suffix: bool,
    pub unicode: bool,
    pub plural: bool,
    pub reference: bool,
    pub avoid_input: bool,
    pub case_sensitive: bool,
    pub completion_only: bool,
}

impl ExpressionName {
    /// A plain name with default flags. Single-character names are
    /// case-sensitive by default, matching `ExpressionName::setDefaults`.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let case_sensitive = name.chars().count() == 1;
        ExpressionName {
            name,
            case_sensitive,
            ..Default::default()
        }
    }
}

/// Parse a `<names>` value into its constituent names.
///
/// Entries are comma-separated; a leading `flags:` prefix applies to that
/// entry only. A `-` inside the flags negates the flags after it.
pub fn parse_names(spec: &str) -> Vec<ExpressionName> {
    let mut out = Vec::new();
    for entry in spec.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        out.push(parse_name_entry(entry));
    }
    out
}

fn parse_name_entry(entry: &str) -> ExpressionName {
    // A flag prefix is everything before the first ':' — but only if that
    // prefix consists solely of flag characters, so names containing ':'
    // (rare, but legal) are not mangled.
    let Some(colon) = entry.find(':') else {
        return ExpressionName::new(entry);
    };
    let (flags, rest) = entry.split_at(colon);
    let rest = &rest[1..];
    if flags.is_empty() || !flags.chars().all(|c| matches!(c, 'a' | 'r' | 'p' | 'u' | 's' | 'c' | 'i' | 'b' | '-')) {
        return ExpressionName::new(entry);
    }
    let mut n = ExpressionName::new(rest);
    let mut value = true;
    for c in flags.chars() {
        match c {
            '-' => value = false,
            'a' => n.abbreviation = value,
            'r' => n.reference = value,
            'p' => n.plural = value,
            'u' => n.unicode = value,
            's' => n.suffix = value,
            'c' => n.case_sensitive = value,
            'i' => n.avoid_input = value,
            'b' => n.completion_only = value,
            _ => {}
        }
    }
    n
}

/// The set of names attached to one expression item, with the
/// `preferredName`-family selection rules from `ExpressionItem.cc`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameSet {
    pub names: Vec<ExpressionName>,
}

impl NameSet {
    pub fn from_spec(spec: &str) -> Self {
        NameSet {
            names: parse_names(spec),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// `findName`: does any name match `name`, honouring case sensitivity?
    pub fn matches(&self, name: &str) -> bool {
        self.names.iter().any(|n| {
            if n.case_sensitive {
                n.name == name
            } else {
                n.name.eq_ignore_ascii_case(name)
            }
        })
    }

    /// `preferredDisplayName`: prefer an abbreviation when `abbreviate` is
    /// set, a unicode form when allowed, and never a plural or input-avoided
    /// name.
    pub fn preferred_display_name(&self, abbreviate: bool, use_unicode: bool) -> Option<&str> {
        let candidates: Vec<&ExpressionName> = self
            .names
            .iter()
            .filter(|n| !n.plural && !n.suffix && (use_unicode || !n.unicode))
            .collect();
        if candidates.is_empty() {
            return self.names.first().map(|n| n.name.as_str());
        }
        let pick = if abbreviate {
            candidates
                .iter()
                .find(|n| n.abbreviation)
                .or_else(|| candidates.first())
        } else {
            candidates
                .iter()
                .find(|n| !n.abbreviation)
                .or_else(|| candidates.first())
        };
        pick.map(|n| n.name.as_str())
    }

    /// `preferredInputName`: the name to use when re-parsing output.
    pub fn preferred_input_name(&self, abbreviate: bool) -> Option<&str> {
        let candidates: Vec<&ExpressionName> = self
            .names
            .iter()
            .filter(|n| !n.avoid_input && !n.plural && !n.unicode)
            .collect();
        let pool = if candidates.is_empty() {
            self.names.iter().collect::<Vec<_>>()
        } else {
            candidates
        };
        let pick = if abbreviate {
            pool.iter().find(|n| n.abbreviation).or_else(|| pool.first())
        } else {
            pool.iter().find(|n| !n.abbreviation).or_else(|| pool.first())
        };
        pick.map(|n| n.name.as_str())
    }

    /// `referenceName`: the untranslated identity of the item.
    pub fn reference_name(&self) -> Option<&str> {
        self.names
            .iter()
            .find(|n| n.reference)
            .or_else(|| self.names.first())
            .map(|n| n.name.as_str())
    }

    /// Every name, for registry indexing.
    pub fn all(&self) -> impl Iterator<Item = &ExpressionName> {
        self.names.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names() {
        let n = parse_names("meter,metre");
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].name, "meter");
        assert!(!n[0].abbreviation);
    }

    #[test]
    fn meter_name_spec() {
        // From units.xml: `ar:m,meter,p:meters,metre,p:metres`
        let n = parse_names("ar:m,meter,p:meters,metre,p:metres");
        assert_eq!(n.len(), 5);
        assert_eq!(n[0].name, "m");
        assert!(n[0].abbreviation && n[0].reference);
        assert!(n[0].case_sensitive, "single-char names are case sensitive");
        assert_eq!(n[1].name, "meter");
        assert!(!n[1].abbreviation);
        assert_eq!(n[2].name, "meters");
        assert!(n[2].plural);
    }

    #[test]
    fn negating_flags() {
        // From currencies.xml: `a-cr:USD,au:€`
        let n = parse_names("a-cr:USD,au:€");
        assert_eq!(n[0].name, "USD");
        assert!(n[0].abbreviation, "a precedes the -");
        assert!(!n[0].case_sensitive, "c is negated");
        assert!(!n[0].reference, "r is negated");
        assert_eq!(n[1].name, "€");
        assert!(n[1].abbreviation && n[1].unicode);
    }

    #[test]
    fn names_without_flag_prefix_survive() {
        // A colon that is not a flag prefix must not be treated as one.
        let n = parse_names("some:thing");
        assert_eq!(n[0].name, "some:thing");
    }

    #[test]
    fn display_name_selection() {
        let s = NameSet::from_spec("ar:m,meter,p:meters");
        assert_eq!(s.preferred_display_name(true, false), Some("m"));
        assert_eq!(s.preferred_display_name(false, false), Some("meter"));
    }

    #[test]
    fn input_name_avoids_plural_and_unicode() {
        let s = NameSet::from_spec("a-cr:USD,au:€");
        assert_eq!(s.preferred_input_name(true), Some("USD"));
    }

    #[test]
    fn matching_respects_case_sensitivity() {
        let s = NameSet::from_spec("ar:m,meter");
        assert!(s.matches("m"));
        assert!(!s.matches("M"), "single-char name is case sensitive");
        assert!(s.matches("Meter"), "multi-char names are not");
    }

    #[test]
    fn reference_name_is_the_flagged_one() {
        let s = NameSet::from_spec("a:km_c,r:kilometer");
        assert_eq!(s.reference_name(), Some("kilometer"));
    }
}
