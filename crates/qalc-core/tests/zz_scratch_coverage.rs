//! scratch: measure how many corpus integrands the port answers.
use qalc_core::options::EvaluationOptions;
use qalc_core::structure::MathStructure;
use qalc_core::{parser, Session};

const BASES: &[&str] = &[
    "4x+5", "-2x+7", "4.7x-5.2", "-4.3x-5", "4x", "-2.3x", "x+6", "x-7", "x", "x^2", "2x^2+5",
    "-2x^2-5", "sqrt(x)", "sqrt(3x+3)", "5*sqrt(3x)-2", "cbrt(3x+3)", "(3x+3)^(1/3)", "cbrt(x)",
    "x^(1/3)", "5^x",
];
const WRAPPERS: &[&str] = &[
    "ln(@)",
    "sin(@)",
    "cos(@)",
    "tan(@)",
    "asin(@)",
    "acos(@)",
    "atan(@)",
    "sinh(@)",
    "cosh(@)",
    "tanh(@)",
    "asinh(@)",
    "acosh(@)",
    "atanh(@)",
    "(@)^2",
    "(@)^(-1)",
    "(@)^(-2)",
    "(@)^3",
    "(@)^(-3)",
    "(@)^(1/3)",
];
const SHAPES: &[&str] = &["@", "(@)*x", "(@)*x^2", "(@)*x^(-1)", "(@)*($)", "(@)*(@)"];

fn contains_integrate(m: &MathStructure) -> bool {
    let want = qalc_core::integrate::function_id_for_name("integrate").unwrap();
    if let MathStructure::Function { id, .. } = m {
        if *id == want {
            return true;
        }
    }
    match m {
        MathStructure::Power { base, exponent } => {
            contains_integrate(base) || contains_integrate(exponent)
        }
        _ => m.children().any(contains_integrate),
    }
}

#[test]
fn scratch_measure() {
    let mut cases = Vec::new();
    for b in BASES {
        for w in WRAPPERS {
            let wrapped = w.replace('@', b);
            for s in SHAPES {
                cases.push((
                    s.replace('@', &wrapped).replace('$', b),
                    *w,
                    *s,
                ));
            }
        }
    }
    let (tx, rx) = std::sync::mpsc::channel::<(usize, bool)>();
    let cases2 = cases.clone();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let s = Session::new();
            let x = MathStructure::symbolic("x");
            for (i, (expr, _, _)) in cases2.iter().enumerate() {
                let ok = (|| {
                    let mut f = parser::parse_with(expr, &s.parse_options, &s).ok()?;
                    qalc_core::percent::apply(&mut f);
                    let eo = EvaluationOptions::default();
                    qalc_core::eval::evaluate_calculated_with(&mut f, &eo);
                    let mut a = qalc_core::integrate::integrate(&f, &x)?;
                    qalc_core::eval::evaluate_calculated_with(&mut a, &eo);
                    if contains_integrate(&a) {
                        return None;
                    }
                    Some(())
                })()
                .is_some();
                let _ = tx.send((i, ok));
            }
        })
        .unwrap();
    let mut answered = vec![false; cases.len()];
    let mut got = 0;
    let mut timed_out = Vec::new();
    while got < cases.len() {
        match rx.recv_timeout(std::time::Duration::from_secs(120)) {
            Ok((i, ok)) => {
                answered[i] = ok;
                got += 1;
            }
            Err(_) => {
                timed_out.push(got);
                break;
            }
        }
    }
    let n: usize = answered.iter().filter(|b| **b).count();
    println!("PORT ANSWERS {n}/{}", cases.len());
    if !timed_out.is_empty() {
        println!("STUCK at index {:?}: {:?}", timed_out, cases[got].0);
    }
    let mut by_w: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
    for (i, (_, w, _)) in cases.iter().enumerate() {
        let e = by_w.entry(w).or_default();
        e.1 += 1;
        if answered[i] {
            e.0 += 1;
        }
    }
    for (w, (a, t)) in &by_w {
        println!("  {w:>10}  {a:>3}/{t}");
    }
    let mut out = String::new();
    for (i, (e, _, _)) in cases.iter().enumerate() {
        out.push_str(if answered[i] { "A\t" } else { "D\t" });
        out.push_str(e);
        out.push('\n');
    }
    std::fs::write("/tmp/claude-0/-root-Project-rust-calc/8c71fdb9-6722-4419-aac7-c241eff9c12a/scratchpad/port_verdicts.txt", out).unwrap();
}
