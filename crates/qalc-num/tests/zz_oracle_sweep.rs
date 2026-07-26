// TEMPORARY oracle-comparison harness (deleted after the sweep).
use qalc_num::{Number, PrintOptions};

fn po() -> PrintOptions {
    let mut po = PrintOptions::default();
    po.show_ending_zeroes = true;
    po
}

fn parse(s: &str) -> Number {
    Number::parse(s, &qalc_num::ParseOptions::default())
}

#[test]
fn sweep() {
    let args = [
        "0.1", "0.25", "0.75", "1.3", "2.2", "3.3", "4.9", "7.1", "12.34", "0.001", "60.5",
        "-0.1", "-1.2", "-2.5", "-3.9", "-7.25", "100.5", "0.5", "5", "1",
    ];
    let funcs: [(&str, fn(&mut Number) -> bool); 9] = [
        ("gamma", Number::gamma),
        ("digamma", Number::digamma),
        ("erf", Number::erf),
        ("erfc", Number::erfc),
        ("erfi", Number::erfi),
        ("zeta", Number::zeta),
        ("Ei", Number::expint),
        ("Si", Number::sinint),
        ("Ci", Number::cosint),
    ];
    for (name, f) in funcs {
        for a in args {
            let mut n = parse(a);
            let ok = f(&mut n);
            println!("{name}({a}) = {}", if ok { n.print(&po()) } else { "FAIL".into() });
        }
    }
    for a in ["2", "3.5", "10", "0.5", "-1", "100"] {
        let mut n = parse(a);
        let ok = n.logint();
        println!("li({a}) = {}", if ok { n.print(&po()) } else { "FAIL".into() });
    }
}
