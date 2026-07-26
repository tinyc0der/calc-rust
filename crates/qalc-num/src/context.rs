//! Global numeric state, mirroring libqalculate's coupling to the
//! `CALCULATOR` singleton (`PRECISION`, `CREATE_INTERVAL`) and MPFR's
//! default precision. Thread-local, with the same defaults as libqalculate.

use astro_float::Consts;
use std::cell::{Cell, RefCell};

/// Default decimal precision (Calculator's `DEFAULT_PRECISION`).
pub const DEFAULT_PRECISION: i32 = 10;

/// log2(10) constant used by libqalculate's `BIT_PRECISION` macro.
pub const LOG2_10: f64 = 3.3219281;

/// `IntervalCalculation` (`includes.h`) — how uncertainties propagate.
///
/// `/set ic <n>` in the CLI. The default is the variance formula, which
/// carries an uncertainty alongside the value and pushes it through each
/// operation by that operation's derivative; interval arithmetic instead
/// widens the value itself into an interval and lets ordinary interval
/// propagation do the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalCalculation {
    None,
    VarianceFormula,
    IntervalArithmetic,
    SimpleIntervalArithmetic,
}

impl IntervalCalculation {
    pub fn from_i32(v: i32) -> Option<IntervalCalculation> {
        Some(match v {
            0 => IntervalCalculation::None,
            1 => IntervalCalculation::VarianceFormula,
            2 => IntervalCalculation::IntervalArithmetic,
            3 => IntervalCalculation::SimpleIntervalArithmetic,
            _ => return None,
        })
    }
}

thread_local! {
    static PRECISION: Cell<i32> = const { Cell::new(DEFAULT_PRECISION) };
    static INTERVAL_ARITHMETIC: Cell<bool> = const { Cell::new(true) };
    static INTERVAL_CALCULATION: Cell<IntervalCalculation> =
        const { Cell::new(IntervalCalculation::VarianceFormula) };
    static CONSTS: RefCell<Option<Consts>> = const { RefCell::new(None) };
}

/// `EvaluationOptions::interval_calculation`.
pub fn interval_calculation() -> IntervalCalculation {
    INTERVAL_CALCULATION.with(|i| i.get())
}

pub fn set_interval_calculation(v: IntervalCalculation) {
    INTERVAL_CALCULATION.with(|i| i.set(v));
}

/// Current global decimal precision (`PRECISION` macro).
pub fn precision() -> i32 {
    PRECISION.with(|p| p.get())
}

pub fn set_precision(prec: i32) {
    PRECISION.with(|p| p.set(prec.max(2)));
}

/// Whether interval arithmetic is used (`CREATE_INTERVAL` macro).
pub fn create_interval() -> bool {
    INTERVAL_ARITHMETIC.with(|i| i.get())
}

pub fn set_create_interval(b: bool) {
    INTERVAL_ARITHMETIC.with(|i| i.set(b));
}

/// Working float precision in bits: `BIT_PRECISION` = PRECISION*log2(10)+100.
pub fn bit_precision() -> usize {
    to_bit_precision(precision()) + 100
}

/// `TO_BIT_PRECISION(p)` = ceil(p * 3.3219281).
pub fn to_bit_precision(dec: i32) -> usize {
    ((dec as f64) * LOG2_10).ceil() as usize
}

/// `FROM_BIT_PRECISION(p)` = floor(p / 3.3219281).
pub fn from_bit_precision(bits: usize) -> i32 {
    ((bits as f64) / LOG2_10).floor() as i32
}

/// Run `f` with the thread-local astro-float constants cache (pi/e/ln2/ln10).
pub fn with_consts<R>(f: impl FnOnce(&mut Consts) -> R) -> R {
    CONSTS.with(|c| {
        let mut c = c.borrow_mut();
        let cc = c.get_or_insert_with(|| Consts::new().expect("constants cache"));
        f(cc)
    })
}
