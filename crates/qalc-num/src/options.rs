//! Enums and option structs ported from libqalculate's `includes.h`.
//!
//! Only the parts consumed by the numeric core live here; expression-level
//! options (EvaluationOptions, SortOptions) live in `qalc-core`.

/// Result of a (possibly interval-aware) comparison. Mirrors `ComparisonResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonResult {
    Less,
    Greater,
    Equal,
    Unknown,
    EqualLimits,
    Contains,
    IsContained,
    OverlappingLess,
    OverlappingGreater,
    NotEqual,
    EqualOrLess,
    EqualOrGreater,
}

impl ComparisonResult {
    /// `COMPARISON_MIGHT_BE_EQUAL` macro.
    pub fn might_be_equal(self) -> bool {
        matches!(
            self,
            ComparisonResult::Unknown
                | ComparisonResult::EqualLimits
                | ComparisonResult::Contains
                | ComparisonResult::IsContained
                | ComparisonResult::OverlappingLess
                | ComparisonResult::OverlappingGreater
                | ComparisonResult::EqualOrLess
                | ComparisonResult::EqualOrGreater
        )
    }
    /// `COMPARISON_NOT_FULLY_KNOWN` macro.
    pub fn not_fully_known(self) -> bool {
        matches!(
            self,
            ComparisonResult::Unknown
                | ComparisonResult::Contains
                | ComparisonResult::IsContained
                | ComparisonResult::OverlappingLess
                | ComparisonResult::OverlappingGreater
                | ComparisonResult::EqualOrLess
                | ComparisonResult::EqualOrGreater
        )
    }
    pub fn is_equal_or_might_be(self) -> bool {
        self == ComparisonResult::Equal || self.might_be_equal()
    }
}

/// Rounding modes for `Number::round`. Mirrors `RoundingMode` in includes.h.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoundingMode {
    #[default]
    HalfAwayFromZero,
    HalfToEven,
    HalfToOdd,
    HalfTowardZero,
    HalfRandom,
    HalfUp,
    HalfDown,
    TowardZero,
    AwayFromZero,
    Up,
    Down,
}

/// How approximate numbers are displayed. Mirrors `IntervalDisplay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IntervalDisplay {
    #[default]
    SignificantDigits,
    Interval,
    PlusMinus,
    Midpoint,
    Lower,
    Upper,
    Concise,
    Relative,
}

/// Number base display style (`BaseDisplay`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BaseDisplay {
    None,
    #[default]
    Normal,
    Alternative,
    Suffix,
}

/// Exponent display style (`ExpDisplay`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExpDisplay {
    #[default]
    Default,
    UppercaseE,
    LowercaseE,
    PowerOf10,
}

/// Fraction display format (`NumberFractionFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NumberFractionFormat {
    #[default]
    Decimal,
    DecimalExact,
    Fractional,
    Combined,
    FractionalFixedDenominator,
    CombinedFixedDenominator,
    Percent,
    Permille,
    Permyriad,
}

/// Digit grouping (`DigitGrouping`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DigitGrouping {
    #[default]
    None,
    Standard,
    Locale,
}

/// How read precision is handled while parsing (`ReadPrecisionMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadPrecisionMode {
    #[default]
    DontRead,
    Always,
    WhenDecimals,
}

/// Parsing grammar mode (`ParsingMode`) — the parts relevant to number parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParsingMode {
    Adaptive,
    Implicit,
    #[default]
    Conventional,
    Chain,
    Rpn,
}

/// Special base constants, mirroring `BASE_*` defines in includes.h.
pub mod base {
    pub const ROMAN_NUMERALS: i32 = -1;
    pub const TIME: i32 = -2;
    pub const BINARY: i32 = 2;
    pub const OCTAL: i32 = 8;
    pub const DECIMAL: i32 = 10;
    pub const DUODECIMAL: i32 = 12;
    pub const HEXADECIMAL: i32 = 16;
    pub const SEXAGESIMAL: i32 = 60;
    pub const SEXAGESIMAL_2: i32 = 62;
    pub const SEXAGESIMAL_3: i32 = 63;
    pub const LATITUDE: i32 = 70;
    pub const LATITUDE_2: i32 = 72;
    pub const LONGITUDE: i32 = 71;
    pub const LONGITUDE_2: i32 = 73;
    pub const FP16: i32 = -30;
    pub const FP32: i32 = -31;
    pub const FP64: i32 = -32;
    pub const FP80: i32 = -33;
    pub const FP128: i32 = -34;
    pub const CUSTOM: i32 = -100;
    pub const UNICODE: i32 = -4;
    pub const GOLDEN_RATIO: i32 = -5;
    pub const SUPER_GOLDEN_RATIO: i32 = -6;
    pub const PI: i32 = -7;
    pub const E: i32 = -8;
    pub const SQRT2: i32 = -9;
    pub const BIJECTIVE_26: i32 = -26;
    pub const BINARY_DECIMAL: i32 = -20;
}

/// `EXP_*` scientific-notation constants.
pub mod exp_mode {
    pub const PRECISION: i32 = -1;
    pub const NONE: i32 = 0;
    pub const PURE: i32 = 1;
    pub const SCIENTIFIC: i32 = 3;
    pub const BASE_3: i32 = -3;
}

/// Options controlling expression/number parsing. Mirrors `ParseOptions`
/// (number-relevant subset; expression-level fields are added in qalc-core).
#[derive(Debug, Clone)]
pub struct ParseOptions {
    /// Numbers are parsed as approximate values with precision equal to the
    /// number of significant digits.
    pub read_precision: ReadPrecisionMode,
    /// Base of parsed numbers (10 default; see [`base`] constants).
    pub base: i32,
    /// Interpret comma as decimal separator.
    pub comma_as_separator: bool,
    /// Interpret dot as thousands separator.
    pub dot_as_separator: bool,
    pub parsing_mode: ParsingMode,
    /// Interpret binary/hex numbers as two's complement.
    pub twos_complement: bool,
    pub hexadecimal_twos_complement: bool,
    /// Number of bits used for two's complement (0 = auto).
    pub binary_bits: u32,
    /// Preserve the format of the parsed expression (e.g. keep 0.5 as 1/2 off).
    pub preserve_format: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions {
            read_precision: ReadPrecisionMode::DontRead,
            base: 10,
            comma_as_separator: false,
            dot_as_separator: false,
            parsing_mode: ParsingMode::Conventional,
            twos_complement: false,
            hexadecimal_twos_complement: false,
            binary_bits: 0,
            preserve_format: false,
        }
    }
}

/// Which time zone dates are printed in (`TIME_ZONE_*`, includes.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeZoneMode {
    /// The zone the value is stored in; no suffix.
    #[default]
    Local,
    /// UTC, with a trailing `Z`.
    Utc,
    /// `PrintOptions::custom_time_zone` minutes east of UTC, with a
    /// trailing `+HH:MM` / `-HH:MM`.
    Custom,
}

/// Options controlling number/expression printing. Mirrors `PrintOptions`
/// (the fields consumed by `Number::print`).
#[derive(Debug, Clone)]
pub struct PrintOptions {
    pub min_exp: i32,
    pub base: i32,
    pub base_display: BaseDisplay,
    pub lower_case_numbers: bool,
    pub exp_display: ExpDisplay,
    pub number_fraction_format: NumberFractionFormat,
    pub indicate_infinite_series: bool,
    pub show_ending_zeroes: bool,
    pub abbreviate_names: bool,
    pub use_denominator_prefix: bool,
    pub negative_exponents: bool,
    pub short_multiplication: bool,
    pub spacious: bool,
    pub excessive_parenthesis: bool,
    pub use_unicode_signs: bool,
    pub use_max_decimals: bool,
    pub max_decimals: i32,
    pub use_min_decimals: bool,
    pub min_decimals: i32,
    pub preserve_format: bool,
    pub preserve_precision: bool,
    pub restrict_to_parent_precision: bool,
    pub restrict_fraction_length: bool,
    pub round_halfway_to_even: bool,
    pub rounding: RoundingMode,
    pub interval_display: IntervalDisplay,
    pub digit_grouping: DigitGrouping,
    pub decimalpoint: String,
    pub comma: String,
    pub twos_complement: bool,
    pub hexadecimal_twos_complement: bool,
    pub binary_bits: u32,
    pub duodecimal_symbols: bool,
    /// `CALCULATOR->customOutputBase()` — the base selected by `to base <x>`
    /// when it is not one of the built-in integer bases.
    pub custom_base: Option<crate::Number>,
    /// Which zone dates are rendered in (`TIME_ZONE_*`).
    pub time_zone: TimeZoneMode,
    /// Custom time zone; also carries the TZ_TRUNCATE hack (see Number.cc).
    pub custom_time_zone: i32,
}

impl Default for PrintOptions {
    fn default() -> Self {
        PrintOptions {
            min_exp: exp_mode::PRECISION,
            base: 10,
            base_display: BaseDisplay::Normal,
            lower_case_numbers: false,
            exp_display: ExpDisplay::Default,
            number_fraction_format: NumberFractionFormat::Decimal,
            indicate_infinite_series: false,
            show_ending_zeroes: false,
            abbreviate_names: true,
            use_denominator_prefix: true,
            negative_exponents: false,
            short_multiplication: true,
            spacious: true,
            excessive_parenthesis: false,
            use_unicode_signs: false,
            use_max_decimals: false,
            max_decimals: -1,
            use_min_decimals: false,
            min_decimals: 0,
            preserve_format: false,
            preserve_precision: false,
            restrict_to_parent_precision: true,
            restrict_fraction_length: false,
            round_halfway_to_even: false,
            rounding: RoundingMode::HalfAwayFromZero,
            interval_display: IntervalDisplay::SignificantDigits,
            digit_grouping: DigitGrouping::None,
            decimalpoint: ".".to_string(),
            comma: ",".to_string(),
            twos_complement: true,
            hexadecimal_twos_complement: false,
            binary_bits: 0,
            duodecimal_symbols: false,
            custom_base: None,
            time_zone: TimeZoneMode::Local,
            custom_time_zone: 0,
        }
    }
}

impl PrintOptions {
    pub fn decimalpoint(&self) -> &str {
        if self.decimalpoint.is_empty() { "." } else { &self.decimalpoint }
    }
    pub fn comma(&self) -> &str {
        if self.comma.is_empty() { "," } else { &self.comma }
    }
}
