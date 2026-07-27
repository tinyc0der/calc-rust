//! Loader for the Qalculate XML definition files.
//!
//! Port of `Calculator::loadDefinitions` / `loadGlobalDefinitions` from
//! `Calculator-definitions.cc`, using `roxmltree` (pure Rust) in place of
//! libxml2.
//!
//! What is ported
//! --------------
//! * The `<QALCULATE version="...">` envelope and nested `<category>` tree,
//!   including the `!context!Title` message-context prefix and the
//!   `/` → U+2215 substitution in category names.
//! * `<prefix>`, `<unit type="base|alias|composite">`, `<builtin_unit>`,
//!   `<variable>`, `<unknown>`, `<builtin_variable>`, `<function>` and
//!   `<builtin_function>`.
//! * The C++ retry queue (`unfinished_nodes`): an alias or composite unit that
//!   names a base which has not been read yet is deferred and retried until
//!   the file stops making progress.
//! * `UserFunction::setFormula`'s argument counting, which derives
//!   `minargs`/`maxargs` and the optional-argument defaults from the `\x` /
//!   `\X{default}` / `\v` placeholders in the formula.
//!
//! What is skipped
//! ---------------
//! * TODO(port): `<dataset>` / `<builtin_dataset>` (datasets.xml,
//!   elements.xml, planets.xml) — needs the `DataSet` machinery.
//! * TODO(port): exchange-rate fetching (`loadExchangeRates`) and therefore
//!   the real rates behind the currency units; see
//!   [`Unit::pending_exchange_rate`](crate::defs::Unit::pending_exchange_rate).
//! * TODO(port): local user definitions (`loadLocalDefinitions`), `<activate>`
//!   / `<deactivate>`, duplicate checking, and all of the save/export half of
//!   `Calculator-definitions.cc`.
//! * TODO(port): locale selection. Only the untranslated (no `xml:lang`)
//!   `<names>` / `<title>` / `<description>` are read, which is what the C++
//!   does under `LANG=C`. The shipped `.xml.in` templates carry no
//!   translations anyway — those live in the `.po` catalogues.
//! * TODO(port): the pre-0.9.4 `<name>`/`<singular>`/`<plural>` element form.
//!   Every shipped file declares a version that uses `<names>`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use roxmltree::{Document, Node};

use super::{
    ArgumentDef, ArgumentType, Assumptions, AssumptionSign, AssumptionType, CompositePart,
    FunctionDef, FunctionKind, Prefix, PrefixId, PrefixPreference, Registry, Subfunction, Unit,
    UnitKind, Variable, VariableValue,
};
use crate::ids::{FunctionId, UnitId, VariableId};
use crate::names::NameSet;
use qalc_num::{Number, ParseOptions};

/// Environment variable naming the directory holding the definition files,
/// mirroring the C++ `QALCULATE_DEFINITIONS_DIR`.
pub const DEFINITIONS_DIR_ENV: &str = "QALCULATE_DEFINITIONS_DIR";

/// The files `loadGlobalDefinitions` reads, in the order it reads them.
/// Prefixes must exist before composite units can name them, currencies
/// before units (`units.xml` aliases a few currencies), and variables last
/// because their values may mention units.
///
/// `datasets.xml` sits between functions and variables in the C++ list; it is
/// omitted here (TODO(port): DataSet).
pub const GLOBAL_DEFINITION_FILES: &[&str] = &[
    "prefixes.xml",
    "currencies.xml",
    "units.xml",
    "functions.xml",
    "variables.xml",
];

#[derive(Debug)]
pub enum LoadError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Xml {
        path: PathBuf,
        source: roxmltree::Error,
    },
    /// The root element was not `<QALCULATE>`; the C++ reports
    /// "File not identified as Qalculate! definitions file".
    NotADefinitionsFile { path: PathBuf },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io { path, source } => write!(f, "{}: {}", path.display(), source),
            LoadError::Xml { path, source } => write!(f, "{}: {}", path.display(), source),
            LoadError::NotADefinitionsFile { path } => write!(
                f,
                "{}: file not identified as a Qalculate! definitions file",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// The directory the definition files are read from: `$QALCULATE_DEFINITIONS_DIR`
/// if set, else `None` (the C++ falls back to a compiled-in `PKGDATADIR`,
/// which a library port has no business guessing).
pub fn definitions_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DEFINITIONS_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    for candidate in [
        "../libqalculate/data",
        "../../libqalculate/data",
        "../../../libqalculate/data",
        "../../../../libqalculate/data",
        "../Demo/libqalculate/data",
        "../../Demo/libqalculate/data",
        "../../../Demo/libqalculate/data",
        "../../../../Demo/libqalculate/data",
        "/usr/share/qalculate",
        "/usr/local/share/qalculate",
    ] {
        let p = PathBuf::from(candidate);
        if p.join("prefixes.xml.in").is_file() || p.join("prefixes.xml").is_file() {
            return Some(p);
        }
    }
    None
}

/// Port of `Calculator::loadGlobalDefinitions`.
///
/// Seeds the built-in currency units (`Calculator::addBuiltinUnits`) and then
/// loads each file of [`GLOBAL_DEFINITION_FILES`] in order. For each name both
/// `foo.xml` and the shipped `foo.xml.in` template are tried, so the loader
/// works against an unbuilt source tree.
pub fn load_global_definitions(dir: &Path, reg: &mut Registry) -> Result<(), LoadError> {
    add_builtin_units(reg);
    for name in GLOBAL_DEFINITION_FILES {
        let path = dir.join(name);
        if path.is_file() {
            load_definitions_file(&path, reg)?;
            continue;
        }
        let template = dir.join(format!("{name}.in"));
        if template.is_file() {
            load_definitions_file(&template, reg)?;
        }
        // A missing optional file is not an error, matching the C++, which
        // only warns.
    }
    Ok(())
}

/// Port of `Calculator::addBuiltinUnits`: the currency units the C++ creates
/// in code rather than in XML. `EUR` is the base every other currency is an
/// alias of.
pub fn add_builtin_units(reg: &mut Registry) {
    if reg.find_unit_id("EUR").is_some() {
        return;
    }
    let eur = reg.add_unit(Unit {
        id: UnitId(0),
        names: NameSet::from_spec("a-cr:EUR,euro,p:euros"),
        kind: UnitKind::Base,
        category: "Currency".to_string(),
        title: "European Euro".to_string(),
        description: String::new(),
        system: String::new(),
        countries: String::new(),
        hidden: false,
        approximate: false,
        precision: -1,
        use_with_prefixes: None,
        builtin: Some("EUR".to_string()),
        pending_exchange_rate: false,
        active: true,
    });
    let currency_alias = |reg: &mut Registry,
                          builtin: &str,
                          spec: &str,
                          title: &str,
                          base,
                          relation: &str| {
        reg.add_unit(Unit {
            id: UnitId(0),
            names: NameSet::from_spec(spec),
            kind: UnitKind::Alias {
                base,
                relation: relation.to_string(),
                inverse_relation: None,
                exponent: 1,
                mix_priority: 0,
                mix_min: 0,
                uncertainty: None,
                relative_uncertainty: false,
            },
            category: "Currency".to_string(),
            title: title.to_string(),
            description: String::new(),
            system: String::new(),
            countries: String::new(),
            hidden: false,
            approximate: true,
            precision: -2,
            use_with_prefixes: None,
            builtin: Some(builtin.to_string()),
            pending_exchange_rate: true,
            active: true,
        })
    };
    currency_alias(
        reg,
        "BTC",
        "a-cr:BTC,bitcoin,p:bitcoins",
        "Bitcoins",
        eur,
        "55955.6",
    );
    let byn = currency_alias(reg, "BYN", "a-cr:BYN", "Belarusian Ruble", eur, "1/3.3078");
    currency_alias(
        reg,
        "BYR",
        "a-cr:BYR",
        "Belarusian Ruble p. (obsolete)",
        byn,
        "0.0001",
    );
}

/// Load one definitions file into `reg`.
pub fn load_definitions_file(path: &Path, reg: &mut Registry) -> Result<(), LoadError> {
    let text = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    load_definitions_named(&text, path, reg)
}

/// Load definitions from an in-memory XML document (the C++ accepts a string
/// starting with `<` in place of a file name).
pub fn load_definitions_str(xml: &str, reg: &mut Registry) -> Result<(), LoadError> {
    load_definitions_named(xml, Path::new("<memory>"), reg)
}

fn load_definitions_named(xml: &str, path: &Path, reg: &mut Registry) -> Result<(), LoadError> {
    let doc = Document::parse(xml).map_err(|source| LoadError::Xml {
        path: path.to_path_buf(),
        source,
    })?;
    let root = doc.root_element();
    if root.tag_name().name() != "QALCULATE" {
        return Err(LoadError::NotADefinitionsFile {
            path: path.to_path_buf(),
        });
    }
    let mut deferred: Vec<(Node, String)> = Vec::new();
    walk_category(root, "", reg, &mut deferred);

    // `unfinished_nodes` in the C++: retry the items whose dependencies were
    // not loaded yet, until a whole pass makes no progress.
    while !deferred.is_empty() {
        let mut progressed = false;
        let mut still = Vec::new();
        for (node, category) in deferred.drain(..) {
            if load_item(node, &category, reg) {
                progressed = true;
            } else {
                still.push((node, category));
            }
        }
        deferred = still;
        if !progressed {
            break;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Category tree
// ---------------------------------------------------------------------------

/// The division-slash the C++ substitutes for `/` inside a category title, so
/// that the title cannot be mistaken for a path separator.
const SIGN_DIVISION_SLASH: &str = "\u{2215}";

fn walk_category<'a, 'i: 'a>(
    node: Node<'a, 'i>,
    parent_category: &str,
    reg: &mut Registry,
    deferred: &mut Vec<(Node<'a, 'i>, String)>,
) {
    let mut title = String::new();
    for child in node.children().filter(Node::is_element) {
        if child.tag_name().name() == "title" && child.attribute(XML_LANG).is_none() {
            if title.is_empty() {
                title = node_text(child);
            }
        }
    }
    let mut category = parent_category.to_string();
    if !category.is_empty() {
        category.push('/');
    }
    category.push_str(&strip_context(&title.replace('/', SIGN_DIVISION_SLASH)));

    let mut subcategories = Vec::new();
    for child in node.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "title" => {}
            "category" => subcategories.push(child),
            _ => {
                if !load_item(child, &category, reg) {
                    deferred.push((child, category.clone()));
                }
            }
        }
    }
    for sub in subcategories {
        walk_category(sub, &category, reg, deferred);
    }
}

const XML_LANG: (&str, &str) = ("http://www.w3.org/XML/1998/namespace", "lang");

/// `!context!Title` — a gettext message context prefix that is not part of
/// the string. `!Title` with no closing `!` is kept verbatim.
fn strip_context(s: &str) -> String {
    if !s.starts_with('!') {
        return s.to_string();
    }
    match s[1..].find('!') {
        None => s.to_string(),
        Some(rel) => s[rel + 2..].to_string(),
    }
}

// ---------------------------------------------------------------------------
// Item dispatch
// ---------------------------------------------------------------------------

/// Returns false when the item could not be loaded yet because it references
/// something not in the registry, asking the caller to retry it later.
fn load_item(node: Node, category: &str, reg: &mut Registry) -> bool {
    match node.tag_name().name() {
        "prefix" => {
            load_prefix(node, reg);
            true
        }
        "unit" => load_unit(node, category, reg),
        "builtin_unit" => {
            load_builtin_unit(node, category, reg);
            true
        }
        "variable" => {
            load_variable(node, category, reg);
            true
        }
        "unknown" => {
            load_unknown_variable(node, category, reg);
            true
        }
        "builtin_variable" => {
            load_builtin_variable(node, category, reg);
            true
        }
        "function" => {
            load_function(node, category, reg);
            true
        }
        "builtin_function" => {
            load_builtin_function(node, category, reg);
            true
        }
        // TODO(port): DataSet support (datasets.xml, elements.xml,
        // planets.xml) and the local-definitions-only activate/deactivate.
        "dataset" | "builtin_dataset" | "activate" | "deactivate" => true,
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Shared metadata
// ---------------------------------------------------------------------------

/// `ITEM_INIT_DTH` / `ITEM_READ_DTH` / `ITEM_SET_DTH` plus `ITEM_READ_NAMES`.
#[derive(Default)]
struct ItemMeta {
    names: NameSet,
    title: String,
    description: String,
    hidden: bool,
    active: bool,
}

fn read_meta(node: Node) -> ItemMeta {
    let mut m = ItemMeta {
        active: node.attribute("active") != Some("false"),
        ..Default::default()
    };
    for child in node.children().filter(Node::is_element) {
        if child.attribute(XML_LANG).is_some() {
            continue;
        }
        match child.tag_name().name() {
            "names" if m.names.is_empty() => {
                m.names = NameSet::from_spec(&node_text(child));
            }
            "title" if m.title.is_empty() => m.title = strip_context(&node_text(child)),
            "description" if m.description.is_empty() => m.description = node_text(child),
            "hidden" => m.hidden = node_text(child) == "true",
            _ => {}
        }
    }
    m
}

fn node_text(node: Node) -> String {
    let mut s = String::new();
    for child in node.children() {
        if let Some(t) = child.text() {
            s.push_str(t);
        }
    }
    s.trim().to_string()
}

fn child_text(node: Node, name: &str) -> Option<String> {
    node.children()
        .filter(Node::is_element)
        .find(|c| c.tag_name().name() == name)
        .map(node_text)
}

fn attr_i32(node: Node, name: &str, default: i32) -> i32 {
    node.attribute(name)
        .and_then(|v| v.trim().parse::<i32>().ok())
        .unwrap_or(default)
}

fn text_i64(node: Node, default: i64) -> i64 {
    let t = node_text(node);
    if t.is_empty() {
        default
    } else {
        t.parse::<i64>().unwrap_or(default)
    }
}

/// `XML_GET_APPROX_FROM_PROP`: `approximate="true"`, or `precise` negated.
fn approx_attr(node: Node) -> bool {
    match node.attribute("approximate") {
        Some(v) => v == "true",
        None => match node.attribute("precise") {
            Some(v) => v != "true",
            None => false,
        },
    }
}

/// `XML_GET_PREC_FROM_PROP`.
fn precision_attr(node: Node) -> i32 {
    attr_i32(node, "precision", -1)
}

/// `<use_with_prefixes max= min= default=>true</use_with_prefixes>`.
fn read_prefix_preference(node: Node) -> PrefixPreference {
    PrefixPreference {
        use_by_default: node_text(node) == "true",
        max: attr_i32(node, "max", i32::MAX),
        min: attr_i32(node, "min", i32::MIN),
        default: attr_i32(node, "default", 0),
    }
}

// ---------------------------------------------------------------------------
// Prefixes
// ---------------------------------------------------------------------------

fn load_prefix(node: Node, reg: &mut Registry) {
    let meta = read_meta(node);
    if meta.names.is_empty() {
        return;
    }
    let kind = node.attribute("type").unwrap_or("");
    let exponent = child_text(node, "exponent")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let value = child_text(node, "value").unwrap_or_default();
    let id = PrefixId(0);
    let prefix = match kind {
        "decimal" => Prefix::decimal(id, meta.names, exponent),
        "binary" => Prefix::binary(id, meta.names, exponent),
        "number" => Prefix::number(id, meta.names, parse_number(&value)),
        // Untyped: a decimal prefix unless a free value is given.
        _ if value.is_empty() => Prefix::decimal(id, meta.names, exponent),
        _ => Prefix::number(id, meta.names, parse_number(&value)),
    };
    reg.add_prefix(prefix);
}

fn parse_number(s: &str) -> Number {
    Number::parse(s, &ParseOptions::default())
}

// ---------------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------------

fn load_unit(node: Node, category: &str, reg: &mut Registry) -> bool {
    match node.attribute("type").unwrap_or("") {
        "base" => {
            load_base_unit(node, category, reg);
            true
        }
        "alias" => load_alias_unit(node, category, reg),
        "composite" => load_composite_unit(node, category, reg),
        _ => true,
    }
}

fn base_unit_fields(node: Node, category: &str, meta: &ItemMeta, kind: UnitKind) -> Unit {
    Unit {
        id: UnitId(0),
        names: meta.names.clone(),
        kind,
        category: category.to_string(),
        title: meta.title.clone(),
        description: meta.description.clone(),
        system: child_text(node, "system").unwrap_or_default(),
        countries: child_text(node, "countries").unwrap_or_default(),
        hidden: meta.hidden,
        approximate: false,
        precision: -1,
        use_with_prefixes: node
            .children()
            .filter(Node::is_element)
            .find(|c| c.tag_name().name() == "use_with_prefixes")
            .map(read_prefix_preference),
        builtin: None,
        pending_exchange_rate: false,
        active: meta.active,
    }
}

fn load_base_unit(node: Node, category: &str, reg: &mut Registry) {
    let meta = read_meta(node);
    if meta.names.is_empty() {
        return;
    }
    let u = base_unit_fields(node, category, &meta, UnitKind::Base);
    reg.add_unit(u);
}

fn load_alias_unit(node: Node, category: &str, reg: &mut Registry) -> bool {
    let meta = read_meta(node);
    if meta.names.is_empty() {
        return true;
    }
    let Some(base_node) = node
        .children()
        .filter(Node::is_element)
        .find(|c| c.tag_name().name() == "base")
    else {
        return true;
    };

    let mut base_name = String::new();
    let mut relation = String::new();
    let mut inverse_relation = None;
    let mut exponent = 1i64;
    let mut mix_priority = 0i32;
    let mut mix_min = 0i32;
    let mut uncertainty = None;
    let mut relative_uncertainty = false;
    let mut approximate = false;
    let mut precision = -1;

    for child in base_node.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "unit" => base_name = node_text(child),
            "relation" => {
                relation = node_text(child);
                approximate = approx_attr(child);
                precision = precision_attr(child);
                if let Some(u) = child.attribute("relative_uncertainty") {
                    uncertainty = Some(u.to_string());
                    relative_uncertainty = true;
                } else if let Some(u) = child.attribute("uncertainty") {
                    uncertainty = Some(u.to_string());
                }
            }
            // The pre-3.x spelling of the same element.
            "inverse_relation" | "reverse_relation" => {
                inverse_relation = Some(node_text(child));
            }
            "exponent" => exponent = text_i64(child, 1),
            "mix" => {
                mix_priority = text_i64(child, 0) as i32;
                mix_min = attr_i32(child, "min", 0);
            }
            _ => {}
        }
    }

    let Some(base) = reg.find_unit_id(&base_name) else {
        // Defer: the base unit is defined later in the file.
        return false;
    };

    let mut u = base_unit_fields(
        node,
        category,
        &meta,
        UnitKind::Alias {
            base,
            relation,
            inverse_relation,
            exponent,
            mix_priority,
            mix_min,
            uncertainty,
            relative_uncertainty,
        },
    );
    u.approximate = approximate;
    u.precision = precision;
    reg.add_unit(u);
    true
}

fn load_composite_unit(node: Node, category: &str, reg: &mut Registry) -> bool {
    let meta = read_meta(node);
    if meta.names.is_empty() {
        return true;
    }
    let mut parts = Vec::new();
    for child in node.children().filter(Node::is_element) {
        if child.tag_name().name() != "part" {
            continue;
        }
        let mut unit_name = String::new();
        let mut prefix = None;
        let mut exponent = 1i64;
        for c2 in child.children().filter(Node::is_element) {
            match c2.tag_name().name() {
                "unit" => unit_name = node_text(c2),
                "prefix" => {
                    let ptype = c2.attribute("type").unwrap_or("");
                    let value = node_text(c2);
                    prefix = match ptype {
                        "binary" => {
                            let e = value.parse::<i64>().unwrap_or(0);
                            if e == 0 {
                                None
                            } else {
                                match reg.exact_binary_prefix(e) {
                                    Some(p) => Some(p),
                                    // Prefix missing: the whole composite is
                                    // unusable, as in the C++.
                                    None => return true,
                                }
                            }
                        }
                        "number" => {
                            let n = parse_number(&value);
                            if n.is_zero() {
                                None
                            } else {
                                match reg
                                    .prefixes()
                                    .iter()
                                    .find(|p| p.value().equals(&n, false, false))
                                {
                                    Some(p) => Some(p.id),
                                    None => return true,
                                }
                            }
                        }
                        _ => {
                            let e = value.parse::<i64>().unwrap_or(0);
                            if e == 0 {
                                None
                            } else {
                                match reg.exact_decimal_prefix(e) {
                                    Some(p) => Some(p),
                                    None => return true,
                                }
                            }
                        }
                    };
                }
                "exponent" => exponent = text_i64(c2, 1),
                _ => {}
            }
        }
        let Some(unit) = reg.find_unit_id(&unit_name) else {
            // Defer until the referenced unit exists.
            return false;
        };
        parts.push(CompositePart {
            unit,
            prefix,
            exponent,
        });
    }
    if parts.is_empty() {
        return true;
    }
    let u = base_unit_fields(node, category, &meta, UnitKind::Composite { parts });
    reg.add_unit(u);
    true
}

/// `<builtin_unit name="USD">` decorates a unit the C++ created in code.
///
/// When the named unit is already registered we merge the XML metadata into
/// it. Otherwise — which is every currency but `EUR`, `BTC`, `BYN` and `BYR`,
/// since the rest are created by `loadExchangeRates` — a placeholder currency
/// alias is registered so that name resolution works. Its relation is *not* a
/// real exchange rate; see [`Unit::pending_exchange_rate`].
fn load_builtin_unit(node: Node, category: &str, reg: &mut Registry) {
    let name = node.attribute("name").unwrap_or("").to_string();
    let meta = read_meta(node);
    let countries = child_text(node, "countries").unwrap_or_default();
    let system = child_text(node, "system").unwrap_or_default();
    let prefixes = node
        .children()
        .filter(Node::is_element)
        .find(|c| c.tag_name().name() == "use_with_prefixes")
        .map(read_prefix_preference);

    if let Some(id) = reg.find_unit_id(&name) {
        let existing_names = reg.unit(id).names.clone();
        let u = reg.unit_mut(id);
        u.category = category.to_string();
        if !meta.title.is_empty() {
            u.title = meta.title;
        }
        if !meta.description.is_empty() {
            u.description = meta.description;
        }
        if !countries.is_empty() {
            u.countries = countries;
        }
        if !system.is_empty() {
            u.system = system;
        }
        if prefixes.is_some() {
            u.use_with_prefixes = prefixes;
        }
        u.hidden = meta.hidden;
        u.builtin = Some(name);
        if !meta.names.is_empty() {
            // `ITEM_SET_BUILTIN_NAMES`: the XML names win, but any builtin
            // name the file does not restate must survive so that code
            // holding the old spelling can still find the unit.
            let mut names = meta.names;
            for n in existing_names.all() {
                if !names.matches(&n.name) {
                    names.names.push(n.clone());
                }
            }
            reg.set_unit_names(id, names);
        }
        return;
    }

    if meta.names.is_empty() {
        return;
    }
    // TODO(port): the exchange rate comes from loadExchangeRates.
    let kind = match reg.find_unit_id("EUR") {
        Some(eur) => UnitKind::Alias {
            base: eur,
            relation: "1".to_string(),
            inverse_relation: None,
            exponent: 1,
            mix_priority: 0,
            mix_min: 0,
            uncertainty: None,
            relative_uncertainty: false,
        },
        None => UnitKind::Base,
    };
    let mut u = base_unit_fields(node, category, &meta, kind);
    u.countries = countries;
    u.builtin = Some(name);
    u.pending_exchange_rate = true;
    u.approximate = true;
    reg.add_unit(u);
}

// ---------------------------------------------------------------------------
// Variables
// ---------------------------------------------------------------------------

fn load_variable(node: Node, category: &str, reg: &mut Registry) {
    let meta = read_meta(node);
    if meta.names.is_empty() {
        return;
    }
    let mut expression = String::new();
    let mut unit = None;
    let mut uncertainty = None;
    let mut relative_uncertainty = false;
    let mut approximate = false;
    let mut precision = -1;
    for child in node.children().filter(Node::is_element) {
        if child.tag_name().name() != "value" {
            continue;
        }
        expression = node_text(child);
        unit = child.attribute("unit").map(str::to_string);
        if let Some(u) = child.attribute("relative_uncertainty") {
            uncertainty = Some(u.to_string());
            relative_uncertainty = true;
        } else if let Some(u) = child.attribute("uncertainty") {
            uncertainty = Some(u.to_string());
        }
        precision = precision_attr(child);
        approximate = approx_attr(child);
        break;
    }
    reg.add_variable(Variable {
        id: VariableId(0),
        names: meta.names,
        // Kept unevaluated: `KnownVariable` with `b_expression` set. The C++
        // parses it the first time the variable is used.
        value: VariableValue::Expression {
            expression,
            unit,
            uncertainty,
            relative_uncertainty,
        },
        category: category.to_string(),
        title: meta.title,
        description: meta.description,
        hidden: meta.hidden,
        approximate,
        precision,
        active: meta.active,
    });
}

fn load_unknown_variable(node: Node, category: &str, reg: &mut Registry) {
    let meta = read_meta(node);
    if meta.names.is_empty() {
        return;
    }
    let mut a = Assumptions::new();
    for child in node.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "type" => {
                a.atype = match node_text(child).as_str() {
                    "integer" => AssumptionType::Integer,
                    "boolean" => AssumptionType::Boolean,
                    "rational" => AssumptionType::Rational,
                    "real" => AssumptionType::Real,
                    "complex" => AssumptionType::Complex,
                    "number" => AssumptionType::Number,
                    "non-matrix" => AssumptionType::NonMatrix,
                    "none" => AssumptionType::None,
                    _ => a.atype,
                };
            }
            "sign" => {
                a.sign = match node_text(child).as_str() {
                    "non-zero" => AssumptionSign::NonZero,
                    "non-positive" => AssumptionSign::NonPositive,
                    "negative" => AssumptionSign::Negative,
                    "non-negative" => AssumptionSign::NonNegative,
                    "positive" => AssumptionSign::Positive,
                    "unknown" => AssumptionSign::Unknown,
                    _ => a.sign,
                };
            }
            _ => {}
        }
    }
    reg.add_variable(Variable {
        id: VariableId(0),
        names: meta.names,
        value: VariableValue::Unknown(a),
        category: category.to_string(),
        title: meta.title,
        description: meta.description,
        hidden: meta.hidden,
        approximate: false,
        precision: -1,
        active: meta.active,
    });
}

/// `<builtin_variable name="pi">` decorates a `DynamicVariable` defined in
/// C++. Nothing implements those yet, so a placeholder carrying the builtin's
/// name is registered — enough for the parser to resolve `pi`.
fn load_builtin_variable(node: Node, category: &str, reg: &mut Registry) {
    let name = node.attribute("name").unwrap_or("").to_string();
    let meta = read_meta(node);
    if let Some(id) = reg.find_variable_id(&name) {
        let v = reg.variable_mut(id);
        v.category = category.to_string();
        if !meta.title.is_empty() {
            v.title = meta.title;
        }
        return;
    }
    if meta.names.is_empty() {
        return;
    }
    reg.add_variable(Variable {
        id: VariableId(0),
        names: meta.names,
        value: VariableValue::Builtin { builtin_name: name },
        category: category.to_string(),
        title: meta.title,
        description: meta.description,
        hidden: meta.hidden,
        approximate: false,
        precision: -1,
        active: meta.active,
    });
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

fn read_arguments(node: Node, user_defined: bool) -> Vec<ArgumentDef> {
    let mut args: BTreeMap<usize, ArgumentDef> = BTreeMap::new();
    for child in node.children().filter(Node::is_element) {
        if child.tag_name().name() != "argument" {
            continue;
        }
        let index = child
            .attribute("index")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1);
        let atype = ArgumentType::from_xml(child.attribute("type").unwrap_or(""));
        let mut a = ArgumentDef::new(index, atype);
        // The plain `Argument` created for an unknown/missing type handles
        // vectors by default in the global definitions.
        if atype == ArgumentType::Free && !user_defined {
            a.handle_vector = true;
        }
        for c2 in child.children().filter(Node::is_element) {
            if c2.attribute(XML_LANG).is_some() {
                continue;
            }
            match c2.tag_name().name() {
                "title" if a.name.is_empty() => a.name = strip_context(&node_text(c2)),
                "min" => {
                    a.min = Some(node_text(c2));
                    a.include_equals_min = c2.attribute("include_equals") != Some("false");
                }
                "max" => {
                    a.max = Some(node_text(c2));
                    a.include_equals_max = c2.attribute("include_equals") != Some("false");
                }
                "complex_allowed" => a.complex_allowed = node_text(c2) != "false",
                "condition" => a.condition = Some(node_text(c2)),
                "matrix_allowed" => a.matrix_allowed = node_text(c2) == "true",
                "zero_forbidden" => a.zero_forbidden = node_text(c2) == "true",
                "test" => a.tests = node_text(c2) != "false",
                "handle_vector" => a.handle_vector = node_text(c2) != "false",
                "alert" => a.alerts = node_text(c2) != "false",
                _ => {}
            }
        }
        args.insert(index, a);
    }
    args.into_values().collect()
}

fn load_function(node: Node, category: &str, reg: &mut Registry) {
    let meta = read_meta(node);
    if meta.names.is_empty() {
        return;
    }
    let mut raw_expression = String::new();
    let mut approximate = false;
    let mut precision = -1;
    let mut condition = None;
    let mut example = None;
    let mut subs_raw: Vec<Subfunction> = Vec::new();
    for child in node.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "expression" => {
                raw_expression = node_text(child);
                precision = precision_attr(child);
                approximate = approx_attr(child);
            }
            "condition" => condition = Some(node_text(child)),
            "example" => example = Some(node_text(child)),
            "subfunction" => subs_raw.push(Subfunction {
                expression: node_text(child),
                precalculate: child.attribute("precalculate") != Some("false"),
            }),
            _ => {}
        }
    }

    let sub_texts: Vec<String> = subs_raw.iter().map(|s| s.expression.clone()).collect();
    let arity = user_function_arity(&raw_expression, &sub_texts);
    let subfunctions: Vec<Subfunction> = subs_raw
        .iter()
        .zip(arity.subfunctions.iter())
        .map(|(s, rewritten)| Subfunction {
            expression: rewritten.clone(),
            precalculate: s.precalculate,
        })
        .collect();

    reg.add_function(FunctionDef {
        id: FunctionId(0),
        names: meta.names,
        kind: FunctionKind::User {
            expression: arity.formula,
            raw_expression,
            subfunctions,
            default_values: arity.default_values,
        },
        min_args: arity.min_args,
        max_args: arity.max_args,
        arguments: read_arguments(node, true),
        category: category.to_string(),
        title: meta.title,
        description: meta.description,
        condition,
        example,
        hidden: meta.hidden,
        approximate,
        precision,
        active: meta.active,
    });
}

/// `<builtin_function name="sin">` decorates a C++ `MathFunction` subclass.
///
/// TODO(port): the classes are not ported, so a metadata-only placeholder is
/// registered and the argument counts are taken from the `<argument>`
/// elements the file documents (see [`FunctionKind::Builtin`]).
fn load_builtin_function(node: Node, category: &str, reg: &mut Registry) {
    let name = node.attribute("name").unwrap_or("").to_string();
    let meta = read_meta(node);
    let arguments = read_arguments(node, false);
    let declared = arguments.iter().map(|a| a.index).max().unwrap_or(0) as i32;

    if let Some(id) = reg.find_function_id(&name) {
        let f = reg.function_mut(id);
        f.category = category.to_string();
        if !meta.title.is_empty() {
            f.title = meta.title;
        }
        if !meta.description.is_empty() {
            f.description = meta.description;
        }
        return;
    }
    if meta.names.is_empty() {
        return;
    }
    reg.add_function(FunctionDef {
        id: FunctionId(0),
        names: meta.names,
        kind: FunctionKind::Builtin { builtin_name: name },
        min_args: declared,
        max_args: declared,
        arguments,
        category: category.to_string(),
        title: meta.title,
        description: meta.description,
        condition: child_text(node, "condition"),
        example: child_text(node, "example"),
        hidden: meta.hidden,
        approximate: false,
        precision: -1,
        active: meta.active,
    });
}

// ---------------------------------------------------------------------------
// UserFunction::setFormula — argument counting
// ---------------------------------------------------------------------------

/// Result of scanning a user-function formula for argument placeholders.
pub struct FormulaArity {
    /// Required argument count (`MathFunction::minargs`).
    pub min_args: i32,
    /// Maximum argument count; negative means unlimited.
    pub max_args: i32,
    /// Defaults for the optional arguments, in order.
    pub default_values: Vec<String>,
    /// The formula with `\X{default}` rewritten to `\x`.
    pub formula: String,
    /// The subfunction bodies with the same rewrite applied.
    pub subfunctions: Vec<String>,
}

/// Port of the placeholder scan in `UserFunction::setFormula`.
///
/// `\x`, `\y`, `\z`, `\a` … `\u` are the 24 positional arguments; the
/// uppercase spelling `\X` marks the argument optional and `\X{expr}` gives it
/// a default. `\v` stands for "all remaining arguments" and makes the argument
/// count unlimited. A backslash before the marker escapes it.
pub fn user_function_arity(formula: &str, subfunctions: &[String]) -> FormulaArity {
    let mut f = formula.to_string();
    let mut subs: Vec<String> = subfunctions.to_vec();
    let mut default_values: Vec<String> = Vec::new();

    if f.is_empty() && subs.is_empty() {
        return FormulaArity {
            min_args: 0,
            max_args: 0,
            default_values,
            formula: f,
            subfunctions: subs,
        };
    }

    let mut argc: i32 = 0;
    let mut max_argc: i32 = 0;
    let mut optionals = false;
    let mut last_def_i: i32 = -1;
    let mut i: i32 = 0;

    while i < 26 {
        let lower = placeholder_char(i, false);
        let upper = placeholder_char(i, true);
        let svar = format!("\\{lower}");
        let svar_o = format!("\\{upper}");
        let mut found = false;

        if i < 24 {
            scan_optionals(
                &mut f,
                &svar,
                &svar_o,
                i,
                &mut last_def_i,
                &mut default_values,
                &mut optionals,
                &mut found,
            );
        }
        if !found {
            found = contains_unescaped(&f, &svar);
        }
        for sub in subs.iter_mut() {
            if i < 24 {
                scan_optionals(
                    sub,
                    &svar,
                    &svar_o,
                    i,
                    &mut last_def_i,
                    &mut default_values,
                    &mut optionals,
                    &mut found,
                );
            }
            if !found {
                found = contains_unescaped(sub, &svar);
            }
        }

        if !found {
            if i < 24 && !optionals {
                // Nothing so far: jump straight to the `\v` catch-all.
                i = 24;
                continue;
            }
            break;
        }
        if i >= 24 {
            max_argc = -1;
        } else {
            max_argc += 1;
            if !optionals {
                argc += 1;
            }
        }
        i += 1;
    }

    if argc > 24 {
        argc = 24;
    }
    if max_argc > 24 {
        max_argc = 24;
    }
    if max_argc < 0 || argc < 0 {
        max_argc = -1;
        if argc < 0 {
            argc = 0;
        }
    } else if max_argc < argc {
        max_argc = argc;
    }
    if max_argc > 0 && (default_values.len() as i32) < max_argc - argc {
        default_values.resize((max_argc - argc) as usize, "0".to_string());
    }

    FormulaArity {
        min_args: argc,
        max_args: max_argc,
        default_values,
        formula: f,
        subfunctions: subs,
    }
}

/// `\x`, `\y`, `\z`, then `\a`..`\w`; index 24 is `\v`, the catch-all.
fn placeholder_char(i: i32, upper: bool) -> char {
    let base = if upper { b'X' } else { b'x' };
    let top = if upper { b'Z' } else { b'z' };
    let alpha = if upper { b'A' } else { b'a' };
    let c = if base + i as u8 > top {
        alpha + i as u8 - 3
    } else {
        base + i as u8
    };
    c as char
}

fn find_from(s: &str, pat: &str, start: usize) -> Option<usize> {
    if start > s.len() {
        return None;
    }
    s[start..].find(pat).map(|r| r + start)
}

/// Rewrites every `\X` / `\X{default}` occurrence to `\x`, recording the
/// default value for the argument at `i`.
#[allow(clippy::too_many_arguments)]
fn scan_optionals(
    s: &mut String,
    svar: &str,
    svar_o: &str,
    i: i32,
    last_def_i: &mut i32,
    default_values: &mut Vec<String>,
    optionals: &mut bool,
    found: &mut bool,
) {
    let mut i4 = 0usize;
    while let Some(i2) = find_from(s, svar_o, i4) {
        i4 = i2 + 2;
        if i2 > 0 && s.as_bytes()[i2 - 1] == b'\\' {
            continue;
        }
        // `\X{default}`
        let mut span = 2usize;
        let mut default: Option<String> = None;
        if s.len() > i2 + 2 && s.as_bytes()[i2 + 2] == b'{' {
            if let Some(close) = find_from(s, "}", i2 + 2) {
                default = Some(s[i2 + 3..close].to_string());
                span = close + 1 - i2;
            }
        }
        // Fill in defaults for any arguments skipped over.
        while *last_def_i >= 0 && *last_def_i + 1 < i {
            default_values.push("0".to_string());
            *last_def_i += 1;
        }
        if *last_def_i != i {
            default_values.push(default.clone().unwrap_or_else(|| "0".to_string()));
        } else if let Some(d) = default.clone() {
            if let Some(last) = default_values.last_mut() {
                *last = d;
            }
        }
        *last_def_i = i;
        s.replace_range(i2..i2 + span, svar);

        // Any remaining occurrences lose their default and become `\x`.
        let mut j = i2;
        while let Some(k) = find_from(s, svar_o, j + 1) {
            if k > 0 && s.as_bytes()[k - 1] == b'\\' {
                j = k + 1;
                continue;
            }
            let mut span = 2usize;
            if k + 4 < s.len() && s.as_bytes()[k + 2] == b'{' {
                if let Some(close) = find_from(s, "}", k + 3) {
                    span = close + 1 - k;
                }
            }
            s.replace_range(k..k + span, svar);
            j = k;
        }
        *optionals = true;
        *found = true;
    }
}

/// Is `svar` present unescaped (not preceded by a second backslash)?
fn contains_unescaped(s: &str, svar: &str) -> bool {
    let mut at = 0usize;
    while let Some(i) = find_from(s, svar, at) {
        if i > 0 && s.as_bytes()[i - 1] == b'\\' {
            at = i + 2;
        } else {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arity_of_a_two_argument_formula() {
        let a = user_function_arity("\\x + \\y", &[]);
        assert_eq!((a.min_args, a.max_args), (2, 2));
    }

    #[test]
    fn optional_argument_with_default() {
        // `\y` optional, defaulting to 10.
        let a = user_function_arity("log(\\x, \\Y{10})", &[]);
        assert_eq!((a.min_args, a.max_args), (1, 2));
        assert_eq!(a.default_values, vec!["10".to_string()]);
        assert_eq!(a.formula, "log(\\x, \\y)");
    }

    #[test]
    fn catch_all_placeholder_is_unlimited() {
        let a = user_function_arity("sum(\\v)", &[]);
        assert_eq!(a.max_args, -1);
    }

    #[test]
    fn no_placeholders_means_no_arguments() {
        let a = user_function_arity("42", &[]);
        assert_eq!((a.min_args, a.max_args), (0, 0));
    }

    #[test]
    fn placeholder_alphabet() {
        assert_eq!(placeholder_char(0, false), 'x');
        assert_eq!(placeholder_char(2, false), 'z');
        assert_eq!(placeholder_char(3, false), 'a');
        assert_eq!(placeholder_char(24, false), 'v');
        assert_eq!(placeholder_char(0, true), 'X');
        assert_eq!(placeholder_char(3, true), 'A');
    }

    #[test]
    fn category_context_prefix_is_stripped() {
        assert_eq!(strip_context("!units!Length"), "Length");
        assert_eq!(strip_context("Length"), "Length");
        assert_eq!(strip_context("!Length"), "!Length");
    }

    #[test]
    fn minimal_document_round_trip() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<QALCULATE version="5.7.0">
  <category>
    <title>!units!Length</title>
    <prefix type="decimal"><names>ar:k,r:kilo</names><exponent>3</exponent></prefix>
    <unit type="base">
      <system>SI</system>
      <title>Meter</title>
      <names>ar:m,meter,p:meters</names>
    </unit>
    <unit type="composite">
      <names translatable="no">r:km_c</names>
      <part><unit>m</unit><prefix>3</prefix><exponent>1</exponent></part>
    </unit>
    <unit type="alias">
      <names>ar:in,inch,p:inches</names>
      <base><unit>m</unit><relation>0.0254</relation><exponent>1</exponent></base>
    </unit>
  </category>
</QALCULATE>"#;
        let mut reg = Registry::new();
        load_definitions_str(xml, &mut reg).unwrap();
        assert_eq!(reg.prefixes().len(), 1);
        assert_eq!(reg.units().len(), 3);
        let m = reg.find_unit("meter").unwrap();
        assert!(m.is_base());
        assert_eq!(m.category, "Length");
        assert_eq!(m.system, "SI");
        let km = reg.find_unit("km_c").unwrap();
        match &km.kind {
            UnitKind::Composite { parts } => {
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].unit, m.id);
                assert_eq!(reg.prefix(parts[0].prefix.unwrap()).exponent, 3);
            }
            other => panic!("expected composite, got {other:?}"),
        }
        assert_eq!(reg.find_unit("in").unwrap().relation(), Some("0.0254"));
    }

    #[test]
    fn forward_reference_is_deferred_and_retried() {
        // The alias is declared before its base unit.
        let xml = r#"<QALCULATE version="5.7.0">
  <category>
    <title>Length</title>
    <unit type="alias">
      <names>r:foo</names>
      <base><unit>m</unit><relation>2</relation></base>
    </unit>
    <unit type="base"><names>ar:m,meter</names></unit>
  </category>
</QALCULATE>"#;
        let mut reg = Registry::new();
        load_definitions_str(xml, &mut reg).unwrap();
        let foo = reg.find_unit("foo").expect("deferred alias was retried");
        assert_eq!(foo.base_unit(), Some(reg.find_unit_id("m").unwrap()));
    }

    #[test]
    fn wrong_root_element_is_rejected() {
        let mut reg = Registry::new();
        assert!(load_definitions_str("<other/>", &mut reg).is_err());
    }
}
