//! Units — the port of `Unit.cc`, `Calculator-convert.cc` and the unit half of
//! `MathStructure-convert.cc`.
//!
//! Three jobs live here:
//!
//! 1. **Name resolution.** [`resolve_name`] turns `m`, `km`, `dm3`, `Ω` into a
//!    [`MathStructure::Unit`] (optionally prefixed, optionally raised to a
//!    power), consulting the shipped XML definitions through
//!    [`crate::defs::Registry`].
//! 2. **Base-unit expansion.** Every unit is reduced, once, to a *base form*:
//!    a numeric factor plus a signature over the ten base units
//!    (`m g s K mol Np bit rad A cd`). Alias relations are unevaluated
//!    expression strings in the registry (`ft` is `"3"` over `hand`, `oC` is
//!    `"\x + 273.15"` over `K`), so they are parsed and evaluated here.
//! 3. **Conversion.** [`convert_to`] implements `expr to unit`,
//!    [`convert_to_base_units`] implements `to base`, and
//!    [`convert_to_optimal`] is the reduced port of
//!    `Calculator::convertToOptimalUnit`, the automatic post-conversion that
//!    turns `50 ohm * 2 A` into `100 V`.
//!
//! # Deliberate simplifications (TODO(port))
//!
//! * `Calculator::getOptimalUnit` is a ~300-line point-scoring search that can
//!   split a signature across several named units. [`convert_to_optimal`]
//!   only looks for a *single* named SI unit whose signature matches exactly,
//!   falling back to the base-unit form; that covers `V`, `N`, `Pa`, `W`, `J`
//!   and friends but not, say, `kg*m^2/(A*s^3)` split as `V*A/A`.
//! * Automatic output prefixes (`20 miles` -> `32.18688 km`) are not selected;
//!   only the explicit `?`/`b?` request form is implemented.
//! * A single identifier is never split across several unit names, so `kWh`
//!   does not parse as `kW*h` the way `Calculator::parse` makes it
//!   (`kW h` and `kW*h` do work).
//! * Currencies have no exchange rates (`Registry::pending_exchange_rate`), so
//!   currency conversion is not attempted.
//! * Non-linear relations (temperature) convert scalars through the relation
//!   and `<inverse_relation>` strings, but are not synchronised inside larger
//!   expressions the way `eo.sync_nonlinear_unit_relations` does.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::defs::{
    load_global_definitions, PrefixId, PrefixKind, Registry, UnitKind,
};
use crate::ids::UnitId;
use crate::options::EvaluationOptions;
use crate::structure::MathStructure;
use qalc_num::{Number, ParseOptions};

/// Which prefix family an explicit `to ?unit` / `to b?unit` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixMode {
    /// No automatic prefix.
    None,
    /// `?unit` — decimal prefixes.
    Decimal,
    /// `b?unit` — binary prefixes.
    Binary,
}

// ----------------------------------------------------------------------
// The store
// ----------------------------------------------------------------------

/// The base-unit expansion of one unit.
///
/// `1 unit = factor * PROD(base_i ^ e_i)` where the `base_i` come from
/// [`BaseForm::sig`]. The `display` list is the same expansion but keeps the
/// prefixes the composite definitions carry (`m_kg_p_sqs` spells its mass part
/// as *kilo*grams), which is what `to base` prints.
#[derive(Debug, Clone)]
pub struct BaseForm {
    /// Exponent of each base unit.
    pub sig: BTreeMap<UnitId, i32>,
    /// `1 unit = factor * PROD(base^e)`.
    pub factor: Number,
    /// `1 unit = display_coeff * PROD((prefix base)^e)`.
    pub display_coeff: Number,
    pub display: Vec<(UnitId, Option<PrefixId>, i32)>,
    /// The relation to the base unit is not a plain scale factor
    /// (`\x + 273.15`); `factor` is meaningless then.
    pub nonlinear: bool,
}

impl BaseForm {
    fn unit_itself(id: UnitId) -> BaseForm {
        let mut sig = BTreeMap::new();
        sig.insert(id, 1);
        BaseForm {
            sig,
            factor: Number::from_i64(1),
            display_coeff: Number::from_i64(1),
            display: vec![(id, None, 1)],
            nonlinear: false,
        }
    }
}

/// Everything the unit code needs, built once from the definition files.
pub struct UnitStore {
    reg: Registry,
    forms: Vec<Option<BaseForm>>,
    /// `(name, id, case_sensitive)` sorted by descending name length so the
    /// longest prefix wins (`kilo` before `k`).
    prefix_names: Vec<(String, PrefixId, bool)>,
}

fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = crate::defs::definitions_dir() {
        if dir.is_dir() {
            return Some(dir);
        }
    }
    for candidate in [
        "/root/Project/libqalculate/data",
        "../libqalculate/data",
        "../../libqalculate/data",
        "../../../libqalculate/data",
    ] {
        let p = PathBuf::from(candidate);
        if p.join("units.xml").is_file() || p.join("units.xml.in").is_file() {
            return Some(p);
        }
    }
    None
}

/// The process-wide unit store.
///
/// The C++ keeps the definitions in the `CALCULATOR` singleton; a `OnceLock`
/// is the direct equivalent and keeps `Session::resolve` (which only has
/// `&self`) able to reach it. Returns `None` when no definition directory is
/// present, in which case every name stays symbolic and the port behaves as it
/// did before units existed.
/// The store *if it is already built*, without triggering a load.
///
/// Building the store parses unit relation strings, which re-enters the
/// parser; anything the parser consults must use this rather than
/// [`store`], or `get_or_init` deadlocks on itself.
pub fn store_if_ready() -> Option<&'static UnitStore> {
    store_cell().get().and_then(|o| o.as_ref())
}

fn store_cell() -> &'static OnceLock<Option<UnitStore>> {
    static STORE: OnceLock<Option<UnitStore>> = OnceLock::new();
    &STORE
}

pub fn store() -> Option<&'static UnitStore> {
    store_cell()
        .get_or_init(|| {
            let dir = data_dir()?;
            let mut reg = Registry::new();
            load_global_definitions(&dir, &mut reg).ok()?;
            Some(UnitStore::build(reg))
        })
        .as_ref()
}

impl UnitStore {
    fn build(reg: Registry) -> UnitStore {
        let mut prefix_names: Vec<(String, PrefixId, bool)> = Vec::new();
        for p in reg.prefixes() {
            for n in p.names.all() {
                if n.avoid_input || n.completion_only {
                    continue;
                }
                prefix_names.push((n.name.clone(), p.id, n.case_sensitive));
            }
        }
        prefix_names.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()));
        let n_units = reg.units().len();
        let mut store = UnitStore {
            reg,
            forms: vec![None; n_units],
            prefix_names,
        };
        for i in 0..n_units {
            let id = UnitId(i as u32);
            let form = store.compute_base_form(id, 0);
            store.forms[i] = form;
        }
        store
    }

    pub fn registry(&self) -> &Registry {
        &self.reg
    }

    /// The cached base-unit expansion of `id`, if it could be computed.
    pub fn base_form(&self, id: UnitId) -> Option<&BaseForm> {
        self.forms.get(id.0 as usize).and_then(|f| f.as_ref())
    }

    /// `Unit::convertToBaseUnit` seen structurally: reduce one unit to base
    /// units, recursing through alias relations and composite parts.
    fn compute_base_form(&self, id: UnitId, depth: u32) -> Option<BaseForm> {
        if depth > 32 {
            return None;
        }
        if let Some(Some(f)) = self.forms.get(id.0 as usize) {
            return Some(f.clone());
        }
        let u = self.reg.unit(id);
        match &u.kind {
            UnitKind::Base => Some(BaseForm::unit_itself(id)),
            UnitKind::Alias {
                base,
                relation,
                exponent,
                ..
            } => {
                let inner = self.compute_base_form(*base, depth + 1)?;
                let exp = i32::try_from(*exponent).ok()?;
                let nonlinear = relation.contains("\\x") || inner.nonlinear;
                // A relation without `\x` is a plain scale factor
                // (`AliasUnit::convertToBaseUnit` multiplies by it).
                let rel = if nonlinear {
                    Number::from_i64(1)
                } else {
                    eval_number(relation)?
                };
                let mut sig = BTreeMap::new();
                for (k, e) in &inner.sig {
                    let v = e * exp;
                    if v != 0 {
                        sig.insert(*k, v);
                    }
                }
                let mut factor = pow_number(&inner.factor, exp)?;
                if !factor.multiply(&rel) {
                    return None;
                }
                let mut display_coeff = pow_number(&inner.display_coeff, exp)?;
                if !display_coeff.multiply(&rel) {
                    return None;
                }
                let display = inner
                    .display
                    .iter()
                    .map(|(u, p, e)| (*u, *p, e * exp))
                    .collect();
                Some(BaseForm {
                    sig,
                    factor,
                    display_coeff,
                    display,
                    nonlinear,
                })
            }
            UnitKind::Composite { parts } => {
                let mut sig: BTreeMap<UnitId, i32> = BTreeMap::new();
                let mut factor = Number::from_i64(1);
                let mut display_coeff = Number::from_i64(1);
                let mut display: Vec<(UnitId, Option<PrefixId>, i32)> = Vec::new();
                let mut nonlinear = false;
                for part in parts {
                    let part_exp = i32::try_from(part.exponent).ok()?;
                    let inner = self.compute_base_form(part.unit, depth + 1)?;
                    nonlinear |= inner.nonlinear;
                    for (k, e) in &inner.sig {
                        let slot = sig.entry(*k).or_insert(0);
                        *slot += e * part_exp;
                    }
                    let mut contrib = pow_number(&inner.factor, part_exp)?;
                    if let Some(pid) = part.prefix {
                        let pv = self.reg.prefix(pid).value_for(part.exponent);
                        if !contrib.multiply(&pv) {
                            return None;
                        }
                    }
                    if !factor.multiply(&contrib) {
                        return None;
                    }
                    let dc = pow_number(&inner.display_coeff, part_exp)?;
                    if !display_coeff.multiply(&dc) {
                        return None;
                    }
                    // The prefix of a composite part is attached to the base
                    // unit it expands to when that expansion is a single unit
                    // (`m_kg_p_sqs` -> `m kg s^-2`); otherwise it is folded
                    // into the coefficient.
                    if inner.display.len() == 1 && inner.display[0].1.is_none() && part.prefix.is_some() {
                        let (bu, _, be) = inner.display[0];
                        display.push((bu, part.prefix, be * part_exp));
                    } else {
                        if let Some(pid) = part.prefix {
                            let pv = self.reg.prefix(pid).value_for(part.exponent);
                            if !display_coeff.multiply(&pv) {
                                return None;
                            }
                        }
                        for (bu, bp, be) in &inner.display {
                            display.push((*bu, *bp, be * part_exp));
                        }
                    }
                }
                sig.retain(|_, e| *e != 0);
                merge_display(&mut display);
                Some(BaseForm {
                    sig,
                    factor,
                    display_coeff,
                    display,
                    nonlinear,
                })
            }
        }
    }

    /// The unit's display name (`preferredDisplayName`) with its prefix.
    pub fn unit_name(&self, id: UnitId, prefix: Option<PrefixId>, abbreviate: bool) -> String {
        let u = self.reg.unit(id);
        let name = u
            .names
            .preferred_display_name(abbreviate, false)
            .unwrap_or("?");
        match prefix {
            Some(pid) => {
                let p = self.reg.prefix(pid);
                let pn = p.names.preferred_display_name(abbreviate, false).unwrap_or("");
                format!("{pn}{name}")
            }
            None => name.to_string(),
        }
    }

    /// The reference name, used as the sort key between unit factors.
    pub fn reference_name(&self, id: UnitId) -> &str {
        self.reg.unit(id).reference_name()
    }

    // ------------------------------------------------------------------
    // Name resolution
    // ------------------------------------------------------------------

    /// Resolve one identifier to a unit expression.
    ///
    /// Order matters: a real unit name always wins over a prefix split, so
    /// `min` stays "minute" instead of becoming milli-inch.
    pub fn resolve_name(&self, name: &str) -> Option<MathStructure> {
        if name.is_empty() {
            return None;
        }
        if let Some(id) = self.lookup_unit(name) {
            return Some(MathStructure::unit(id));
        }
        // `dm3` = `dm^3`: a unit name may carry its exponent as trailing
        // digits (Calculator-parse.cc gives unit names this treatment before
        // falling back to an unknown).
        let digits: String = name
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() && digits.len() <= 2 && digits.len() < name.len() {
            let head = &name[..name.len() - digits.len()];
            let exp: i64 = digits.chars().rev().collect::<String>().parse().ok()?;
            if exp >= 2 {
                if let Some(mut base) = self.resolve_plain(head) {
                    base.raise(MathStructure::from(exp));
                    return Some(base);
                }
            }
        }
        self.resolve_plain(name)
    }

    /// A unit name, optionally with a prefix, but without the trailing-digit
    /// exponent shorthand.
    ///
    /// A bare SI prefix is never a name on its own: the reference gives a
    /// standalone `k` no value at all, and `11k` = 11000 comes from the
    /// number parser's magnitude suffix instead (see
    /// [`crate::parser`]'s `magnitude_suffix`), not from this table.
    fn resolve_plain(&self, name: &str) -> Option<MathStructure> {
        if let Some(id) = self.lookup_unit(name) {
            return Some(MathStructure::unit(id));
        }
        for (pname, pid, case_sensitive) in &self.prefix_names {
            let rest = if *case_sensitive {
                name.strip_prefix(pname.as_str())
            } else {
                strip_prefix_ignore_case(name, pname)
            };
            let Some(rest) = rest else { continue };
            if rest.is_empty() {
                continue;
            }
            if let Some(id) = self.lookup_unit(rest) {
                if self.accepts_prefix(id) {
                    return Some(MathStructure::Unit {
                        id,
                        prefix: Some(*pid),
                    });
                }
            }
        }
        None
    }

    fn lookup_unit(&self, name: &str) -> Option<UnitId> {
        let id = self
            .reg
            .find_unit_id_case_sensitive(name)
            .or_else(|| self.reg.find_unit_id(name))?;
        let u = self.reg.unit(id);
        // `<hidden>` (`ExpressionItem::isHidden`) only keeps an item out of
        // menus and listings: the C++ tests it in `qalc.cc`'s `-u`/`--list`
        // output, in `defs2doc`, and when picking a composite unit to display
        // a result in (`Calculator-convert.cc`), never in name lookup
        // (`Calculator::getUnit`/`getActiveUnit`). So `kph`, `cc`, `mHg`,
        // `cumec`, ... must still resolve here; only `<active>false</active>`
        // takes a name out of circulation.
        if !u.active {
            return None;
        }
        Some(id)
    }

    fn accepts_prefix(&self, id: UnitId) -> bool {
        let u = self.reg.unit(id);
        if matches!(u.kind, UnitKind::Composite { .. }) {
            return false;
        }
        match &u.use_with_prefixes {
            // `<use_with_prefixes>false</use_with_prefixes>` (degrees Celsius,
            // radians, ...) really means "never prefixed".
            Some(p) if !p.use_by_default && p.max == 0 && p.min == 0 && p.default == 0 => false,
            _ => true,
        }
    }
}

fn strip_prefix_ignore_case<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    // `get` rather than `split_at`: the boundary may fall inside a multi-byte
    // character (unit and prefix names are full Unicode).
    let head = name.get(..prefix.len())?;
    let rest = name.get(prefix.len()..)?;
    head.eq_ignore_ascii_case(prefix).then_some(rest)
}

fn merge_display(display: &mut Vec<(UnitId, Option<PrefixId>, i32)>) {
    let mut out: Vec<(UnitId, Option<PrefixId>, i32)> = Vec::new();
    for (u, p, e) in display.drain(..) {
        if let Some(slot) = out.iter_mut().find(|(u2, p2, _)| *u2 == u && *p2 == p) {
            slot.2 += e;
        } else {
            out.push((u, p, e));
        }
    }
    out.retain(|(_, _, e)| *e != 0);
    *display = out;
}

fn pow_number(n: &Number, exp: i32) -> Option<Number> {
    if exp == 1 {
        return Some(n.clone());
    }
    let mut r = n.clone();
    if !r.raise(&Number::from_i64(exp as i64), true) {
        return None;
    }
    Some(r)
}

/// Parse and evaluate a definition-file expression string to a number.
///
/// Alias relations are stored unevaluated (`AliasUnit::svalue`); this is the
/// deferred evaluation the C++ does on first use. Returns `None` when the
/// expression needs machinery this port does not have yet (`log2(e)`), which
/// simply leaves the unit unusable.
pub fn eval_number(expr: &str) -> Option<Number> {
    let mut m = crate::parser::parse(expr, &ParseOptions::default()).ok()?;
    crate::eval::evaluate(&mut m);
    match m {
        MathStructure::Number(n) => Some(n),
        _ => None,
    }
}

/// Evaluate a relation string with `\x` bound to `value`.
fn eval_relation_with(expr: &str, value: &Number) -> Option<Number> {
    const MARKER: &str = "qalc_relation_x";
    let src = expr.replace("\\x", MARKER);
    let mut m = crate::parser::parse(&src, &ParseOptions::default()).ok()?;
    substitute_symbol(&mut m, MARKER, value);
    crate::eval::evaluate(&mut m);
    match m {
        MathStructure::Number(n) => Some(n),
        _ => None,
    }
}

fn substitute_symbol(m: &mut MathStructure, name: &str, value: &Number) {
    if matches!(m, MathStructure::Symbolic(s) if s == name) {
        *m = MathStructure::Number(value.clone());
        return;
    }
    for i in 0..m.size() {
        if let Some(c) = m.get_mut(i) {
            substitute_symbol(c, name, value);
        }
    }
}

// ----------------------------------------------------------------------
// Quantities
// ----------------------------------------------------------------------

/// A structure reduced to "a number times a product of base-unit powers".
#[derive(Debug, Clone)]
pub struct Quantity {
    pub coeff: Number,
    pub sig: BTreeMap<UnitId, i32>,
}

/// Reduce `m` to a [`Quantity`], or `None` when it is not a pure quantity
/// (contains symbols, functions, sums, ...).
pub fn quantity_of(store: &UnitStore, m: &MathStructure) -> Option<Quantity> {
    let mut q = Quantity {
        coeff: Number::from_i64(1),
        sig: BTreeMap::new(),
    };
    accumulate(store, m, 1, &mut q)?;
    q.sig.retain(|_, e| *e != 0);
    Some(q)
}

fn accumulate(store: &UnitStore, m: &MathStructure, exp: i32, q: &mut Quantity) -> Option<()> {
    match m {
        MathStructure::Number(n) => {
            let v = pow_number(n, exp)?;
            q.coeff.multiply(&v).then_some(())
        }
        MathStructure::Unit { id, prefix } => {
            let form = store.base_form(*id)?;
            if form.nonlinear {
                return None;
            }
            let mut f = pow_number(&form.factor, exp)?;
            if let Some(pid) = prefix {
                let pv = store.reg.prefix(*pid).value_for(exp as i64);
                if !f.multiply(&pv) {
                    return None;
                }
            }
            if !q.coeff.multiply(&f) {
                return None;
            }
            for (k, e) in &form.sig {
                *q.sig.entry(*k).or_insert(0) += e * exp;
            }
            Some(())
        }
        MathStructure::Multiplication(v) => {
            for f in v {
                accumulate(store, f, exp, q)?;
            }
            Some(())
        }
        MathStructure::Power { base, exponent } => {
            let e = exponent.number()?.to_i64()?;
            let e = i32::try_from(e).ok()?;
            accumulate(store, base, exp * e, q)
        }
        _ => None,
    }
}

/// Does the tree mention a unit anywhere?
pub fn contains_unit(m: &MathStructure) -> bool {
    if matches!(m, MathStructure::Unit { .. }) {
        return true;
    }
    (0..m.size()).any(|i| m.get(i).is_some_and(contains_unit))
}

/// `unit` or `unit^n` — the C++ `isUnit_exp()`.
pub fn is_unit_exp(m: &MathStructure) -> bool {
    match m {
        MathStructure::Unit { .. } => true,
        MathStructure::Power { base, .. } => matches!(**base, MathStructure::Unit { .. }),
        _ => false,
    }
}

/// The (unit, prefix, exponent) of a `unit`/`unit^n` factor.
pub fn unit_exp_parts(m: &MathStructure) -> Option<(UnitId, Option<PrefixId>, i32)> {
    match m {
        MathStructure::Unit { id, prefix } => Some((*id, *prefix, 1)),
        MathStructure::Power { base, exponent } => {
            let MathStructure::Unit { id, prefix } = **base else {
                return None;
            };
            let e = exponent.number()?.to_i64()?;
            Some((id, prefix, i32::try_from(e).ok()?))
        }
        _ => None,
    }
}

/// The mixed-unit rule of `Calculator::parse` (Calculator-parse.cc:6161): a
/// run of quantity pairs in *decreasing* units is a sum, not a product —
/// `5m 2cm` is 5 m + 2 cm, `5ft 2in` is 5 ft + 2 in, `10h 31min` is
/// 10 h + 31 min. Returns the addition terms, or `None` when the run is an
/// ordinary product.
///
/// Two shapes qualify, and the reference keeps them deliberately narrow so an
/// ordinary product like `5 m 2 s` stays a product:
///
/// * metric — the leading unit is base `m` (or unprefixed `L`) and every
///   later pair repeats that unit with a strictly smaller decimal prefix, no
///   smaller than milli;
/// * customary — the leading unit is an unprefixed alias that opted into
///   mixing (`<mix>` in units.xml), and each later unit is the one it is
///   defined against, walking up the alias chain.
pub fn mixed_unit_sum(factors: &[MathStructure]) -> Option<Vec<MathStructure>> {
    use crate::defs::{PrefixKind, UnitKind};
    let store = store_if_ready()?;
    if factors.len() < 4 || factors.len() % 2 != 0 || is_unit_exp(&factors[0]) {
        return None;
    }
    let MathStructure::Unit { id: first_id, prefix: first_prefix } = factors[1] else {
        return None;
    };
    // Every odd position must be a bare unit and every even position a
    // non-unit; that check is shared by both shapes.
    for i in (3..factors.len()).step_by(2) {
        if is_unit_exp(&factors[i - 1]) || !matches!(factors[i], MathStructure::Unit { .. }) {
            return None;
        }
    }
    let decimal_exponent = |p: Option<PrefixId>| -> Option<i64> {
        let p = p?;
        let pr = store.registry().prefix(p);
        (pr.kind == PrefixKind::Decimal).then_some(pr.exponent)
    };

    let ref_name = store.reference_name(first_id);
    let is_metric_head = matches!(store.reg.unit(first_id).kind, UnitKind::Base)
        && (ref_name == "m" || (first_prefix.is_none() && ref_name == "L"));
    let mut ok = false;
    if is_metric_head
        && first_prefix
            .map(|_| matches!(decimal_exponent(first_prefix), Some(e) if e <= 3 && e > -3))
            .unwrap_or(true)
    {
        ok = true;
        let mut p1 = first_prefix;
        for i in (3..factors.len()).step_by(2) {
            let MathStructure::Unit { id, prefix: p2 } = factors[i] else {
                ok = false;
                break;
            };
            if id != first_id {
                ok = false;
                break;
            }
            ok = match (decimal_exponent(p1), decimal_exponent(p2), p1, p2) {
                (Some(e1), Some(e2), _, _) => e1 > e2 && e2 >= -3,
                (None, Some(e2), None, _) => e2 < 0 && e2 >= -3,
                (Some(e1), None, _, None) => e1 > 1,
                _ => false,
            };
            if !ok {
                break;
            }
            p1 = p2;
        }
    } else if first_prefix.is_none() && mix_priority(store, first_id) > 0 {
        ok = true;
        let mut u1 = first_id;
        let last = factors.len() - 1;
        for i in (3..factors.len()).step_by(2) {
            let MathStructure::Unit { id, prefix } = factors[i] else {
                ok = false;
                break;
            };
            if prefix.is_some() || (i != last && mix_priority(store, id) <= 0) {
                ok = false;
                break;
            }
            // Walk up the alias chain to the named unit; every step has to be
            // a mixable alias of its own.
            while alias_base(store, u1) != Some(id) {
                match alias_base(store, u1) {
                    Some(next) if mix_priority(store, next) > 0 => u1 = next,
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                break;
            }
            u1 = id;
        }
    }
    if !ok {
        return None;
    }
    Some(
        factors
            .chunks(2)
            .map(|pair| MathStructure::Multiplication(pair.to_vec()))
            .collect(),
    )
}

// ----------------------------------------------------------------------
// Conversion
// ----------------------------------------------------------------------

/// `MathStructure::convertToBaseUnits` — replace every unit by its base-unit
/// expansion (keeping the prefixes the composite definitions carry, which is
/// why `to base` prints `kg`, not `1000 g`).
pub fn convert_to_base_units(store: &UnitStore, m: &mut MathStructure) {
    if let MathStructure::Unit { id, prefix } = m {
        let (id, prefix) = (*id, *prefix);
        if let Some(form) = store.base_form(id) {
            if form.display.len() == 1 && form.display[0].0 == id && form.display[0].2 == 1 {
                // already a base unit
                return;
            }
            let mut factors: Vec<MathStructure> = Vec::new();
            let mut coeff = form.display_coeff.clone();
            if let Some(pid) = prefix {
                let _ = coeff.multiply(store.reg.prefix(pid).value());
            }
            if !coeff.is_one() {
                factors.push(MathStructure::Number(coeff));
            }
            for (bu, bp, be) in &form.display {
                let u = MathStructure::Unit {
                    id: *bu,
                    prefix: *bp,
                };
                factors.push(if *be == 1 {
                    u
                } else {
                    MathStructure::Power {
                        base: Box::new(u),
                        exponent: Box::new(MathStructure::from(*be as i64)),
                    }
                });
            }
            *m = if factors.len() == 1 {
                factors.pop().expect("one factor")
            } else {
                MathStructure::Multiplication(factors)
            };
        }
        return;
    }
    for i in 0..m.size() {
        if let Some(c) = m.get_mut(i) {
            convert_to_base_units(store, c);
        }
    }
}

/// Strip the numeric factors from a target expression, leaving the units.
fn units_only(m: &MathStructure) -> MathStructure {
    match m {
        MathStructure::Multiplication(v) => {
            let kept: Vec<MathStructure> =
                v.iter().filter(|f| !f.is_number()).cloned().collect();
            match kept.len() {
                0 => MathStructure::from(1),
                1 => kept.into_iter().next().expect("one"),
                _ => MathStructure::Multiplication(kept),
            }
        }
        other => other.clone(),
    }
}

/// The error type of a failed conversion, phrased like the C++ messages.
pub type ConvertError = String;

/// `Calculator::convert(mstruct, to_unit)` — convert `value` so it reads in
/// the units of `target`.
pub fn convert_to(
    store: &UnitStore,
    value: &MathStructure,
    target: &MathStructure,
    mix: bool,
) -> Result<MathStructure, ConvertError> {
    // An addition converts term by term (`5 ft + 3 in to cm`).
    if let MathStructure::Addition(terms) = value {
        let mut out = MathStructure::Addition(
            terms
                .iter()
                .map(|t| convert_to(store, t, target, false))
                .collect::<Result<Vec<_>, _>>()?,
        );
        let eo = EvaluationOptions::default();
        out.calculatesub(&eo);
        crate::sort::sort(&mut out);
        return Ok(out);
    }

    if let Some(r) = convert_nonlinear(store, value, target)? {
        return Ok(r);
    }

    let from = quantity_of(store, value)
        .ok_or_else(|| "the value is not a plain quantity".to_string())?;
    let to = quantity_of(store, target)
        .ok_or_else(|| "the conversion target is not a unit expression".to_string())?;
    if to.sig.is_empty() {
        return Err("the conversion target has no units".to_string());
    }
    // A bare number is taken to be in the target's base units, which is what
    // makes `1.74 to ft` mean `1.74 m to ft`.
    let mut coeff = from.coeff;
    if !from.sig.is_empty() && from.sig != to.sig {
        // The reciprocal target is accepted and inverts the value:
        // `5 m/s to s/m` is `0.2 s/m`.
        let inverted: BTreeMap<UnitId, i32> = to.sig.iter().map(|(k, e)| (*k, -e)).collect();
        if from.sig != inverted || !coeff.recip() {
            return Err("the units are not convertible".to_string());
        }
    }
    if !coeff.divide(&to.coeff) {
        return Err("conversion overflowed".to_string());
    }
    let units = units_only(target);
    let mut out = MathStructure::Multiplication(vec![MathStructure::Number(coeff), units]);
    let eo = EvaluationOptions::default();
    out.calculatesub(&eo);
    crate::sort::sort(&mut out);
    if mix {
        mix_units(store, &mut out);
    }
    Ok(out)
}

/// Temperature-style conversion: a single non-linear alias on either side.
///
/// `AliasUnit::convertToBaseUnit` evaluates `<relation>` with `\x` bound to
/// the value; `convertFromBaseUnit` uses `<inverse_relation>` when the
/// relation cannot be inverted by the calculator.
fn convert_nonlinear(
    store: &UnitStore,
    value: &MathStructure,
    target: &MathStructure,
) -> Result<Option<MathStructure>, ConvertError> {
    let from_u = single_unit(value);
    let to_u = single_unit(target);
    let from_nonlinear = from_u.is_some_and(|(id, _)| is_nonlinear(store, id));
    let to_nonlinear = to_u.is_some_and(|(id, _)| is_nonlinear(store, id));
    if !from_nonlinear && !to_nonlinear {
        return Ok(None);
    }
    let Some((_, coeff)) = split_scalar(value) else {
        return Ok(None);
    };
    let Some((to_id, _)) = to_u else {
        return Ok(None);
    };
    // Down to the shared base unit first.
    let mut n = coeff;
    if let Some((from_id, _)) = from_u {
        n = to_base_value(store, from_id, &n)
            .ok_or_else(|| "unsupported unit relation".to_string())?;
    }
    let n = from_base_value(store, to_id, &n)
        .ok_or_else(|| "unsupported unit relation".to_string())?;
    let mut out = MathStructure::Multiplication(vec![
        MathStructure::Number(n),
        MathStructure::unit(to_id),
    ]);
    let eo = EvaluationOptions::default();
    out.calculatesub(&eo);
    Ok(Some(out))
}

fn is_nonlinear(store: &UnitStore, id: UnitId) -> bool {
    store.base_form(id).is_some_and(|f| f.nonlinear)
}

/// `value` as `(unit, prefix)` when it is `n * unit` or a bare unit.
fn single_unit(m: &MathStructure) -> Option<(UnitId, Option<PrefixId>)> {
    match m {
        MathStructure::Unit { id, prefix } => Some((*id, *prefix)),
        MathStructure::Multiplication(v) => {
            let mut found = None;
            for f in v {
                match f {
                    MathStructure::Number(_) => {}
                    MathStructure::Unit { id, prefix } if found.is_none() => {
                        found = Some((*id, *prefix))
                    }
                    _ => return None,
                }
            }
            found
        }
        _ => None,
    }
}

/// The numeric coefficient of `n * unit`.
fn split_scalar(m: &MathStructure) -> Option<((), Number)> {
    match m {
        MathStructure::Number(n) => Some(((), n.clone())),
        MathStructure::Unit { .. } => Some(((), Number::from_i64(1))),
        MathStructure::Multiplication(v) => {
            let mut n = Number::from_i64(1);
            for f in v {
                if let MathStructure::Number(x) = f {
                    if !n.multiply(x) {
                        return None;
                    }
                }
            }
            Some(((), n))
        }
        _ => None,
    }
}

/// Walk an alias chain up to the base unit, evaluating each `<relation>`.
fn to_base_value(store: &UnitStore, id: UnitId, value: &Number) -> Option<Number> {
    let mut n = value.clone();
    let mut cur = id;
    for _ in 0..32 {
        match &store.reg.unit(cur).kind {
            UnitKind::Alias { base, relation, .. } => {
                n = if relation.contains("\\x") {
                    eval_relation_with(relation, &n)?
                } else {
                    let r = eval_number(relation)?;
                    let mut v = n.clone();
                    v.multiply(&r).then_some(v)?
                };
                cur = *base;
            }
            _ => return Some(n),
        }
    }
    None
}

/// The inverse walk: from the base unit down to `id`.
fn from_base_value(store: &UnitStore, id: UnitId, value: &Number) -> Option<Number> {
    // Collect the chain root-first.
    let mut chain = Vec::new();
    let mut cur = id;
    for _ in 0..32 {
        match &store.reg.unit(cur).kind {
            UnitKind::Alias { base, .. } => {
                chain.push(cur);
                cur = *base;
            }
            _ => break,
        }
    }
    let mut n = value.clone();
    for uid in chain.iter().rev() {
        let UnitKind::Alias {
            relation,
            inverse_relation,
            ..
        } = &store.reg.unit(*uid).kind
        else {
            return None;
        };
        if relation.contains("\\x") {
            let inv = inverse_relation.as_deref()?;
            n = eval_relation_with(inv, &n)?;
        } else {
            let r = eval_number(relation)?;
            let mut v = n.clone();
            v.divide(&r).then_some(())?;
            n = v;
        }
    }
    Some(n)
}

// ----------------------------------------------------------------------
// Mixed units (`1.74 m to ft` -> `5 ft + 8.503937008 in`)
// ----------------------------------------------------------------------

/// `Calculator::convertToMixedUnits` restricted to the downward pass, which is
/// the mode an explicit `to` expression selects
/// (`MIXED_UNITS_CONVERSION_DOWNWARDS_KEEP`, Calculator-convert.cc:2296).
///
/// The `<mix>` priority controls the walk: a unit with `|priority| > 1` is
/// "obsolete" (the `hand` between `ft` and `in`) and is stepped over rather
/// than emitted.
fn mix_units(store: &UnitStore, m: &mut MathStructure) {
    let MathStructure::Multiplication(v) = m else {
        return;
    };
    if v.len() != 2 {
        return;
    }
    let MathStructure::Number(nr0) = &v[0] else {
        return;
    };
    let MathStructure::Unit { id, prefix } = &v[1] else {
        return;
    };
    if prefix.is_some() {
        return;
    }
    let original_u = *id;
    let mut u = original_u;
    let mut nr = nr0.clone();
    if !nr.is_real() || nr.is_one() {
        return;
    }
    let negated = nr.is_negative();
    if negated {
        nr.negate();
    }
    let accept_obsolete = mix_priority(store, u).abs() > 1;
    let mut terms: Vec<(Number, UnitId)> = Vec::new();
    for _ in 0..8 {
        if !mixable(store, u) || nr.is_integer() || nr.is_zero() {
            break;
        }
        let mut int_nr = nr.clone();
        if !int_nr.trunc() {
            break;
        }
        if int_nr.is_zero() && u == original_u {
            break;
        }
        let mut frac = nr.clone();
        if !frac.subtract(&int_nr) {
            break;
        }
        // Descend to the next unit down, skipping obsolete intermediates.
        let Some(rel) = numeric_relation(store, u) else {
            break;
        };
        let mut m2 = frac.clone();
        if !m2.multiply(&rel) {
            break;
        }
        let mut cur = u;
        let mut ok = true;
        loop {
            let Some(next) = alias_base(store, cur) else {
                break;
            };
            if accept_obsolete || mix_priority(store, next).abs() <= 1 {
                break;
            }
            cur = next;
            if mix_priority(store, cur) > 0 {
                let Some(r2) = numeric_relation(store, cur) else {
                    ok = false;
                    break;
                };
                if !m2.multiply(&r2) {
                    ok = false;
                    break;
                }
            } else {
                ok = false;
                break;
            }
        }
        if !ok {
            break;
        }
        let Some(next) = alias_base(store, cur) else {
            break;
        };
        if !m2.is_greater_than(&frac) {
            break;
        }
        terms.push((int_nr, u));
        u = next;
        nr = m2;
    }
    if terms.is_empty() {
        return;
    }
    let mut out: Vec<MathStructure> = Vec::new();
    for (n, uid) in terms {
        let mut n = n;
        if negated {
            n.negate();
        }
        out.push(MathStructure::Multiplication(vec![
            MathStructure::Number(n),
            MathStructure::unit(uid),
        ]));
    }
    let mut last = nr;
    if negated {
        last.negate();
    }
    out.push(MathStructure::Multiplication(vec![
        MathStructure::Number(last),
        MathStructure::unit(u),
    ]));
    *m = MathStructure::Addition(out);
}

/// The loop guard of the downward pass: a base unit, or an alias with a
/// non-composite base of exponent one that opted into mixing.
fn mixable(store: &UnitStore, id: UnitId) -> bool {
    match &store.reg.unit(id).kind {
        UnitKind::Base => true,
        UnitKind::Alias { base, exponent, mix_priority, .. } => {
            *exponent == 1
                && *mix_priority != 0
                && !matches!(store.reg.unit(*base).kind, UnitKind::Composite { .. })
        }
        UnitKind::Composite { .. } => false,
    }
}

fn mix_priority(store: &UnitStore, id: UnitId) -> i32 {
    match &store.reg.unit(id).kind {
        UnitKind::Alias { mix_priority, .. } => *mix_priority,
        _ => 0,
    }
}

fn alias_base(store: &UnitStore, id: UnitId) -> Option<UnitId> {
    match &store.reg.unit(id).kind {
        UnitKind::Alias { base, exponent, .. } if *exponent == 1 => Some(*base),
        _ => None,
    }
}

/// The `<relation>` of an alias, when it is a plain number
/// (`expression().find_first_not_of(NUMBERS)` in the C++).
fn numeric_relation(store: &UnitStore, id: UnitId) -> Option<Number> {
    match &store.reg.unit(id).kind {
        UnitKind::Alias { relation, .. } => {
            if relation.chars().all(|c| c.is_ascii_digit()) && !relation.is_empty() {
                eval_number(relation)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ----------------------------------------------------------------------
// Optimal unit (automatic post-conversion)
// ----------------------------------------------------------------------

/// Points and flags of a unit expression, as counted by
/// `Calculator::convertToOptimalUnit`.
struct Points {
    points: i32,
    minus: bool,
    si: bool,
}

fn count_points(store: &UnitStore, m: &MathStructure) -> Points {
    let mut p = Points {
        points: 0,
        minus: true,
        si: true,
    };
    let factors: Vec<&MathStructure> = match m {
        MathStructure::Multiplication(v) => v.iter().collect(),
        other => vec![other],
    };
    for f in factors {
        let Some((id, _, e)) = unit_exp_parts(f) else {
            continue;
        };
        if p.si && store.reg.unit(id).system != "SI" {
            p.si = false;
        }
        if e < 0 {
            p.points -= e;
        } else {
            p.points += e;
            p.minus = false;
        }
    }
    p
}

/// `Calculator::convertToOptimalUnit(mstruct, eo, /*convert_to_si_units=*/true)`,
/// reduced to a single-named-unit search plus the base-unit fallback.
pub fn convert_to_optimal(store: &UnitStore, m: &mut MathStructure) {
    if let MathStructure::Addition(terms) = m {
        for t in terms.iter_mut() {
            convert_to_optimal(store, t);
        }
        let eo = EvaluationOptions::default();
        m.calculatesub(&eo);
        crate::sort::sort(m);
        return;
    }
    if !contains_unit(m) {
        return;
    }
    // Only pure quantities are rewritten: leaving `2x^2 * a` alone matches the
    // reference, which never reaches the conversion path for such a product.
    let Some(q) = quantity_of(store, m) else {
        return;
    };
    if q.sig.is_empty() {
        return;
    }
    let old = count_points(store, m);
    if old.si && old.points <= 1 && !old.minus {
        return;
    }
    let candidate = named_si_unit(store, &q).or_else(|| {
        let mut base = m.clone();
        convert_to_base_units(store, &mut base);
        let eo = EvaluationOptions::default();
        base.calculatesub(&eo);
        crate::sort::sort(&mut base);
        Some(base)
    });
    let Some(new) = candidate else { return };
    let np = count_points(store, &new);
    // The acceptance test from Calculator-convert.cc:2043 with
    // convert_to_si_units = true.
    if np.points == 0
        || (np.points > old.points && (old.si || !np.si))
        || (np.points == old.points && (np.minus || !old.minus) && !np.si)
    {
        return;
    }
    *m = new;
}

/// Search for one non-hidden SI unit whose base signature matches `q` exactly.
fn named_si_unit(store: &UnitStore, q: &Quantity) -> Option<MathStructure> {
    for u in store.reg.units() {
        if !u.active || u.hidden || u.system != "SI" || u.pending_exchange_rate {
            continue;
        }
        let UnitKind::Alias { exponent, base, .. } = &u.kind else {
            continue;
        };
        // Only "derived" units are interesting: an alias that is a plain
        // rescaling of a base unit would just rename it.
        if *exponent == 1 && matches!(store.reg.unit(*base).kind, UnitKind::Base) {
            continue;
        }
        let Some(form) = store.base_form(u.id) else {
            continue;
        };
        if form.nonlinear || form.sig != q.sig {
            continue;
        }
        let mut coeff = q.coeff.clone();
        if !coeff.divide(&form.factor) {
            continue;
        }
        return Some(MathStructure::Multiplication(vec![
            MathStructure::Number(coeff),
            MathStructure::unit(u.id),
        ]));
    }
    None
}

// ----------------------------------------------------------------------
// Explicit output prefixes (`to b?byte`)
// ----------------------------------------------------------------------

/// Attach the largest prefix of the requested family whose value does not
/// exceed the magnitude of the coefficient (`Calculator::getBestPrefix`).
pub fn apply_prefix_mode(store: &UnitStore, m: &mut MathStructure, mode: PrefixMode) {
    if mode == PrefixMode::None {
        return;
    }
    let MathStructure::Multiplication(v) = m else {
        return;
    };
    if v.len() != 2 {
        return;
    }
    let MathStructure::Number(n) = &v[0] else {
        return;
    };
    let MathStructure::Unit { id, prefix: None } = &v[1] else {
        return;
    };
    let (id, mut value) = (*id, n.clone());
    if !value.is_real() || value.is_zero() {
        return;
    }
    let mut mag = value.clone();
    let _ = mag.abs();
    let want_binary = mode == PrefixMode::Binary;
    let mut best: Option<(PrefixId, Number)> = None;
    for p in store.reg.prefixes() {
        let is_binary = p.kind == PrefixKind::Binary;
        if is_binary != want_binary {
            continue;
        }
        if p.exponent <= 0 {
            continue;
        }
        if p.value().is_greater_than(&mag) {
            continue;
        }
        match &best {
            Some((_, bv)) if !p.value().is_greater_than(bv) => {}
            _ => best = Some((p.id, p.value().clone())),
        }
    }
    let Some((pid, pv)) = best else { return };
    if !value.divide(&pv) {
        return;
    }
    *m = MathStructure::Multiplication(vec![
        MathStructure::Number(value),
        MathStructure::Unit {
            id,
            prefix: Some(pid),
        },
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    fn ev(s: &str) -> String {
        Session::new().evaluate_line(s).expect("evaluates")
    }

    fn have_units() -> bool {
        store().is_some()
    }

    macro_rules! need_units {
        () => {
            if !have_units() {
                eprintln!("skipping: no Qalculate definitions directory");
                return;
            }
        };
    }

    // Every expectation below was produced by the reference binary in the
    // mode `--test-file` uses (`qalc -t +u8` with default settings).

    #[test]
    fn simple_quantity_prints_with_a_space() {
        need_units!();
        assert_eq!(ev("5 m"), "5 m");
        assert_eq!(ev("2 m^2"), "2 m^2");
        assert_eq!(ev("15 m*s"), "15 m*s");
    }

    #[test]
    fn prefixed_unit_names_resolve() {
        need_units!();
        assert_eq!(ev("3 m to cm"), "300 cm");
        assert_eq!(ev("1 m to km"), "0.001 km");
        assert_eq!(ev("1 kg to g"), "1000 g");
    }

    #[test]
    fn real_unit_names_beat_prefix_splits() {
        need_units!();
        // `min` is the minute, not milli-inch; `mi` is the mile.
        assert_eq!(ev("2 min to s"), "120 s");
        assert_eq!(ev("1 mi to yd"), "1760 yd");
    }

    /// `<hidden>` keeps an item out of menus and listings (`qalc -u`,
    /// `defs2doc`, the composite-unit search behind `convertToOptimalUnit`);
    /// `Calculator::getUnit`/`getActiveUnit` never look at it, so a hidden
    /// name still resolves.
    ///
    /// While `lookup_unit` rejected them, the prefix-split fallback took over
    /// and produced answers wrong in *dimension*: `kph` became kilo-phot,
    /// `cc` the speed of light squared, `mHg` an `H*g*m` product.
    #[test]
    fn hidden_units_still_resolve_by_name() {
        need_units!();
        assert_eq!(ev("1 kph to km/h"), "1 km/h");
        assert_eq!(ev("1 kph to m/s"), "0.2777777778 m/s");
        assert_eq!(ev("100 kph to mph"), "62.13711922 mph");
        assert_eq!(ev("60 kmph to km/h"), "60 km/h");
        assert_eq!(ev("1 cc to cm^3"), "1 cm^3");
        assert_eq!(ev("1 cc to mm^3"), "1000 mm^3");
        assert_eq!(ev("1 CC to mm^3"), "1000 mm^3");
        assert_eq!(ev("5 cc to mL"), "5 mL");
        assert_eq!(ev("1 mHg to kPa"), "133.3223684 kPa");
        assert_eq!(ev("1 mHg to Pa"), "133322.3684 Pa");
        assert_eq!(ev("1 mHg to mmHg"), "1000 mmHg");
        assert_eq!(ev("1 cumec"), "1 m^3/s");
        assert_eq!(ev("1 cumec to L/s"), "1000 L/s");
        assert_eq!(ev("1 cal_IUNS"), "4.182 J");
        assert_eq!(ev("1 cal_IUNS to J"), "4.182 J");
    }

    /// The bare forms of the same units. What still separates these from the
    /// reference (`1 km/h`, `1000 mm^3`, `133.3223684 kPa`) is the unported
    /// optimal-unit and optimal-prefix search, which treats the spelled-out
    /// spelling in exactly the same way: `1 knot` and `1 cm^3` are off by the
    /// same step without a hidden unit anywhere in sight.
    #[test]
    fn hidden_unit_names_behave_like_their_spelled_out_form() {
        need_units!();
        assert_eq!(ev("1 kph"), ev("1 km/h"));
        assert_eq!(ev("60 kmph"), ev("60 km/h"));
        assert_eq!(ev("1 cc"), ev("1 cm^3"));
        assert_eq!(ev("1 CC"), ev("1 cm^3"));
        assert_eq!(ev("1 mHg"), ev("1000 mmHg"));
    }

    #[test]
    fn trailing_digits_are_an_exponent() {
        need_units!();
        assert_eq!(ev("5 dm3 to L"), "5 L");
        assert_eq!(ev("25 dm^3 to L"), "25 L");
    }

    #[test]
    fn unit_arithmetic_through_division() {
        need_units!();
        assert_eq!(ev("20 miles / 2h to km/h"), "16.09344 km/h");
        assert_eq!(ev("5 m/s to s/m"), "0.2 s/m");
    }

    #[test]
    fn dimensionless_input_is_taken_in_base_units() {
        need_units!();
        assert_eq!(ev("1.74 to ft"), "5 ft + 8.503937008 in");
    }

    #[test]
    fn mixed_unit_output_skips_obsolete_units() {
        need_units!();
        // ft -> (hand, priority 2, skipped) -> in
        assert_eq!(ev("1.74 m to ft"), "5 ft + 8.503937008 in");
    }

    #[test]
    fn leading_minus_suppresses_mixing() {
        need_units!();
        assert_eq!(ev("1.74 m to -ft"), "5.708661417 ft");
    }

    #[test]
    fn imperial_chain_relations() {
        need_units!();
        assert_eq!(ev("100 lbf * 60 mph to hp"), "15.99999752 hp");
    }

    #[test]
    fn derived_units_combine_into_a_named_unit() {
        need_units!();
        assert_eq!(ev("50 ohm * 2 A"), "100 V");
        assert_eq!(ev("50 \u{3a9} * 2 A"), "100 V");
    }

    #[test]
    fn to_base_expands_to_base_units() {
        need_units!();
        assert_eq!(ev("50 ohm * 2 A to base"), "100 kg*m^2/(A*s^3)");
        assert_eq!(ev("1 N to base"), "1 kg*m/s^2");
        assert_eq!(ev("1 J to base"), "1 kg*m^2/s^2");
    }

    #[test]
    fn derived_units_cancel_to_a_base_power() {
        need_units!();
        assert_eq!(ev("10 N / 5 Pa"), "2 m^2");
        assert_eq!(ev("(10 N)/(5 Pa)"), "2 m^2");
    }

    #[test]
    fn explicit_binary_prefix_request() {
        need_units!();
        assert_eq!(ev("500 megabit/s * 2 h to b?byte"), "419.0951586 GiB");
    }

    #[test]
    fn base_forms_are_computed_for_the_shipped_definitions() {
        need_units!();
        let s = store().expect("store");
        let n = s.registry().find_unit_id("N").expect("newton");
        let form = s.base_form(n).expect("newton base form");
        // 1 N = 1000 g*m/s^2
        assert!(form.factor.equals_i64(1000));
        let g = s.registry().find_unit_id("g").expect("gram");
        let m = s.registry().find_unit_id("m").expect("meter");
        let sec = s.registry().find_unit_id("s").expect("second");
        assert_eq!(form.sig.get(&g), Some(&1));
        assert_eq!(form.sig.get(&m), Some(&1));
        assert_eq!(form.sig.get(&sec), Some(&-2));
    }

    #[test]
    fn liter_is_a_cubic_decimeter() {
        need_units!();
        let s = store().expect("store");
        let l = s.registry().find_unit_id("L").expect("liter");
        let form = s.base_form(l).expect("liter base form");
        let m = s.registry().find_unit_id("m").expect("meter");
        assert_eq!(form.sig.get(&m), Some(&3));
        assert!(form.factor.equals(&Number::from_ints(1, 1000, 0), false, false));
    }

    #[test]
    fn nonlinear_relations_are_flagged() {
        need_units!();
        let s = store().expect("store");
        let c = s.registry().find_unit_id("celsius").expect("celsius");
        assert!(s.base_form(c).expect("celsius").nonlinear);
    }

    #[test]
    fn temperature_conversion_uses_the_relation_strings() {
        need_units!();
        // Verified with the reference binary.
        assert_eq!(ev("0 oC to K"), "273.15 K");
        assert_eq!(ev("100 oC to K"), "373.15 K");
        // `oF` reaches `K` through `<inverse_relation>`.
        assert_eq!(ev("20 oC to oF"), "68 oF");
    }

    #[test]
    fn a_zero_quantity_keeps_its_unit() {
        need_units!();
        // `eo.keep_zero_units`: the reference prints `0 m`, not `0`.
        assert_eq!(ev("0 m"), "0 m");
        assert_eq!(ev("0 m * 5"), "0 m");
    }

    #[test]
    fn reciprocal_targets_invert_the_value() {
        need_units!();
        assert_eq!(ev("5 m/s to s/m"), "0.2 s/m");
        assert_eq!(ev("4 s to 1/Hz"), "4 Hz^-1");
    }

    #[test]
    fn unit_sums_stay_in_one_unit() {
        need_units!();
        assert_eq!(ev("5 m + 3 m"), "8 m");
        assert_eq!(ev("3 kg + 2 kg"), "5 kg");
    }

    #[test]
    fn unit_powers_multiply() {
        need_units!();
        assert_eq!(ev("(5 m)^2"), "25 m^2");
        assert_eq!(ev("5 m^-1"), "5 m^-1");
    }

    #[test]
    fn unknown_names_still_stay_symbolic() {
        need_units!();
        assert_eq!(ev("x + x"), "2x");
        assert_eq!(ev("y*x"), "xy");
    }
}

#[cfg(test)]
mod mixed_unit_tests {
    use crate::session::Session;

    fn ev(s: &str) -> String {
        Session::new().evaluate_line(s).expect("evaluates")
    }

    #[test]
    fn decreasing_units_parse_as_a_sum() {
        // Oracle: `10h 31min` is 10 h + 31 min, not 310 h*min.
        assert_eq!(ev("10h 31min + 8h 30min to time"), "19:01");
        assert_eq!(ev("90min to time"), "1:30");
    }

    #[test]
    fn unrelated_units_stay_a_product() {
        assert_eq!(ev("5 m 2 s"), "10 m*s");
    }

    #[test]
    fn a_non_duration_keeps_base_ten_numbers() {
        // MathStructure-print.cc:4603 — the coefficient of a unit that time
        // format cannot absorb still prints sexagesimally, as in the C++.
        assert_eq!(ev("5 m to time"), "5:00 m");
    }
}
