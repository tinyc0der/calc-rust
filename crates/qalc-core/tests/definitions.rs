//! Integration tests that load the real Qalculate definition files.
//!
//! The data directory is taken from `QALCULATE_DEFINITIONS_DIR`, falling back
//! to the libqalculate checkout that sits beside this one. Every test skips
//! gracefully when the directory is not there, so the suite still passes on a
//! machine without the C++ sources.

use std::path::PathBuf;
use std::sync::OnceLock;

use qalc_core::defs::{
    load_global_definitions, FunctionKind, PrefixKind, Registry, UnitKind, VariableValue,
    DEFINITIONS_DIR_ENV,
};

/// Where the shipped `*.xml.in` templates live.
fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DEFINITIONS_DIR_ENV) {
        let dir = PathBuf::from(dir);
        return dir.is_dir().then_some(dir);
    }
    for candidate in [
        "/root/Project/libqalculate/data",
        "../../../libqalculate/data",
        "../libqalculate/data",
    ] {
        let p = PathBuf::from(candidate);
        if p.join("prefixes.xml.in").is_file() {
            return Some(p);
        }
    }
    None
}

/// The registry, loaded once for the whole test binary.
fn registry() -> Option<&'static Registry> {
    static REG: OnceLock<Option<Registry>> = OnceLock::new();
    REG.get_or_init(|| {
        let dir = data_dir()?;
        let mut reg = Registry::new();
        load_global_definitions(&dir, &mut reg).expect("all five definition files must parse");
        Some(reg)
    })
    .as_ref()
}

/// Skips the test body when the data directory is missing.
macro_rules! reg_or_skip {
    () => {
        match registry() {
            Some(r) => r,
            None => {
                eprintln!(
                    "skipping: no Qalculate data directory (set {})",
                    DEFINITIONS_DIR_ENV
                );
                return;
            }
        }
    };
}

#[test]
fn all_five_files_parse() {
    let Some(dir) = data_dir() else {
        eprintln!("skipping: no Qalculate data directory");
        return;
    };
    // Load into a fresh registry so a parse error surfaces as a test failure
    // rather than a panic inside the shared OnceLock.
    let mut reg = Registry::new();
    load_global_definitions(&dir, &mut reg).unwrap();

    assert!(!reg.prefixes().is_empty(), "prefixes.xml produced nothing");
    assert!(!reg.units().is_empty(), "units.xml produced nothing");
    assert!(!reg.variables().is_empty(), "variables.xml produced nothing");
    assert!(!reg.functions().is_empty(), "functions.xml produced nothing");
}

#[test]
fn prefix_count_and_spot_checks() {
    let reg = reg_or_skip!();
    // prefixes.xml ships 34 prefixes: 24 decimal and 10 binary.
    assert_eq!(reg.prefixes().len(), 34, "prefix count from prefixes.xml");
    let decimal = reg
        .prefixes()
        .iter()
        .filter(|p| p.kind == PrefixKind::Decimal)
        .count();
    let binary = reg
        .prefixes()
        .iter()
        .filter(|p| p.kind == PrefixKind::Binary)
        .count();
    assert_eq!((decimal, binary), (24, 10));

    let kilo = reg.find_prefix("kilo").expect("kilo");
    assert_eq!(kilo.kind, PrefixKind::Decimal);
    assert_eq!(kilo.exponent, 3);
    assert!(kilo.value().equals_i64(1000));
    // The abbreviation resolves to the same prefix.
    assert_eq!(reg.find_prefix("k").map(|p| p.id), Some(kilo.id));

    let kibi = reg.find_prefix("kibi").expect("kibi");
    assert_eq!(kibi.kind, PrefixKind::Binary);
    assert_eq!(kibi.exponent, 10);
    assert!(kibi.value().equals_i64(1024));
}

#[test]
fn meter_abbreviation_and_long_name_are_the_same_base_unit() {
    let reg = reg_or_skip!();
    let m = reg.find_unit_id("m").expect("m");
    let meter = reg.find_unit_id("meter").expect("meter");
    let metre = reg.find_unit_id("metre").expect("metre");
    let meters = reg.find_unit_id("meters").expect("meters");
    assert_eq!(m, meter);
    assert_eq!(m, metre);
    assert_eq!(m, meters);
    assert!(reg.unit(m).is_base(), "the metre is a base unit");
    assert_eq!(reg.unit(m).category, "Length");
    assert_eq!(reg.unit(m).system, "SI");
}

#[test]
fn kilometre_is_a_composite_of_the_metre_and_the_kilo_prefix() {
    let reg = reg_or_skip!();
    let km = reg.find_unit("km_c").expect("km_c");
    let m = reg.find_unit_id("m").unwrap();
    match &km.kind {
        UnitKind::Composite { parts } => {
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].unit, m);
            assert_eq!(parts[0].exponent, 1);
            let p = reg.prefix(parts[0].prefix.expect("kilo prefix"));
            assert_eq!(p.kind, PrefixKind::Decimal);
            assert_eq!(p.exponent, 3);
        }
        other => panic!("km_c should be composite, got {other:?}"),
    }
}

#[test]
fn foot_is_an_alias_with_a_relation_string() {
    let reg = reg_or_skip!();
    let ft = reg.find_unit("ft").expect("ft");
    assert!(ft.is_alias());
    // units.xml defines the foot as three hands, not directly in metres.
    assert_eq!(ft.relation(), Some("3"));
    let base = reg.unit(ft.base_unit().unwrap());
    assert_eq!(base.reference_name(), "hand");
    // The chain still bottoms out at the metre.
    assert_eq!(reg.resolve_base_unit(ft.id), reg.find_unit_id("m").unwrap());
    // `feet` is the same unit.
    assert_eq!(reg.find_unit_id("feet"), Some(ft.id));

    // A relation may be a full expression, and may carry an explicit inverse.
    let celsius = reg.find_unit("oC").expect("oC");
    match &celsius.kind {
        UnitKind::Alias {
            relation,
            inverse_relation,
            ..
        } => {
            assert_eq!(relation, "\\x + 273.15");
            assert_eq!(inverse_relation.as_deref(), Some("\\x - 273.15"));
        }
        other => panic!("oC should be an alias, got {other:?}"),
    }
}

#[test]
fn currencies_are_loaded() {
    let reg = reg_or_skip!();
    let usd = reg.find_unit("USD").expect("USD");
    assert_eq!(usd.category, "Currency");
    assert!(usd.countries.contains("United States"));
    // `$` is an alternative name for the same unit.
    assert_eq!(reg.find_unit_id("$"), Some(usd.id));

    let eur = reg.find_unit("EUR").expect("EUR");
    assert!(eur.is_base(), "the euro is the base currency");
    assert_eq!(reg.find_unit_id("euro"), Some(eur.id));

    // Every currency but the euro hangs off it.
    assert_eq!(reg.resolve_base_unit(usd.id), eur.id);
    // TODO(port): rates come from loadExchangeRates, which is not ported.
    assert!(usd.pending_exchange_rate);

    // A currency defined outright in currencies.xml rather than by rate.
    let cent = reg.find_unit("cent").expect("cent");
    assert!(cent.is_alias());
}

#[test]
fn pi_variable_exists() {
    let reg = reg_or_skip!();
    let pi = reg.find_variable("pi").expect("pi");
    assert_eq!(pi.title, "Archimedes' Constant (pi)");
    // pi is a DynamicVariable in C++; the XML only names it.
    match &pi.value {
        VariableValue::Builtin { builtin_name } => assert_eq!(builtin_name, "pi"),
        other => panic!("pi should be a builtin variable, got {other:?}"),
    }
    // The unicode spelling resolves to the same variable.
    assert_eq!(reg.find_variable_id("\u{3c0}"), Some(pi.id));

    // A variable with an unevaluated value expression.
    let ppm = reg.find_variable("ppm").expect("ppm");
    match &ppm.value {
        VariableValue::Expression { expression, .. } => assert_eq!(expression, "1E-6"),
        other => panic!("ppm should hold an expression, got {other:?}"),
    }

    // And the unknowns, which carry assumptions.
    let x = reg.find_variable("x").expect("x");
    assert!(!x.is_known());
}

#[test]
fn sin_function_exists_with_one_argument() {
    let reg = reg_or_skip!();
    let sin = reg.find_function("sin").expect("sin");
    assert_eq!(sin.title, "Sine");
    assert!(sin.is_builtin());
    match &sin.kind {
        FunctionKind::Builtin { builtin_name } => assert_eq!(builtin_name, "sin"),
        other => panic!("sin should be builtin, got {other:?}"),
    }
    assert_eq!(sin.arguments.len(), 1);
    assert_eq!(sin.min_args, 1);
    assert_eq!(sin.max_args, 1);
    assert_eq!(sin.argument(1).unwrap().name, "Angle");
}

#[test]
fn user_functions_get_their_arity_from_the_formula() {
    let reg = reg_or_skip!();
    // `<function>` entries carry an `<expression>`; the argument counts come
    // from the \x placeholders in it.
    let user: Vec<_> = reg
        .functions()
        .iter()
        .filter(|f| !f.is_builtin())
        .collect();
    assert!(
        user.len() > 100,
        "functions.xml ships many user functions, found {}",
        user.len()
    );
    for f in &user {
        match &f.kind {
            FunctionKind::User { expression, .. } => {
                assert!(
                    !expression.is_empty(),
                    "{} has an empty formula",
                    f.reference_name()
                );
            }
            _ => unreachable!(),
        }
        assert!(f.min_args >= 0);
        assert!(f.max_args < 0 || f.max_args >= f.min_args);
    }
}

#[test]
fn every_alias_and_composite_resolved_its_dependencies() {
    let reg = reg_or_skip!();
    // Nothing should have been left permanently deferred: check that the
    // counts are in the right ballpark and every reference is in range.
    assert!(
        reg.units().len() > 600,
        "units.xml + currencies.xml define many units, found {}",
        reg.units().len()
    );
    for u in reg.units() {
        match &u.kind {
            UnitKind::Alias { base, .. } => {
                assert!(base.0 < reg.units().len() as u32);
            }
            UnitKind::Composite { parts } => {
                assert!(!parts.is_empty(), "{} has no parts", u.reference_name());
                for p in parts {
                    assert!(p.unit.0 < reg.units().len() as u32);
                }
            }
            UnitKind::Base => {}
        }
        // No cycle: resolving terminates at a base or composite unit.
        let base = reg.resolve_base_unit(u.id);
        assert!(reg.unit(base).base_unit().is_none());
    }
}

#[test]
fn categories_are_recorded_from_the_nesting() {
    let reg = reg_or_skip!();
    // Nested `<category>` elements produce slash-joined paths.
    assert!(
        reg.units().iter().any(|u| u.category.contains('/')),
        "expected at least one nested unit category"
    );
    assert_eq!(reg.find_function("sin").unwrap().category, "Trigonometry");
}
