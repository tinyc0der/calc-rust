//! Port of libqalculate's `QalculateDateTime`
//! (`QalculateDateTime.h`/`QalculateDateTime.cc`).
//!
//! Core Gregorian-calendar date/time value with arbitrary-precision seconds
//! (`qalc_num::Number`). Like the C++ original, the calendar is the
//! *proleptic* Gregorian calendar: leap-year rules are applied to all years,
//! including years before the Gregorian reform and non-positive years.
//!
//! Methods keep the C++ mutate-and-return-`bool` shape: `false` means the
//! operation was not applicable and `self` was left unchanged.
//!
//! TODO(port): not ported in this pass —
//! - non-Gregorian calendars (`CalendarSystem`, `calendarToDate`,
//!   `dateToCalendar`, Hebrew/Islamic/Chinese/... month tables)
//! - astronomy (`solarLongitude`, `lunarPhase`, `findNextSolarLongitude`,
//!   `findNextLunarPhase`)
//! - time zones (`dateTimeZone`): all values are treated as being in a
//!   single zone; the `convert_to_utc` flags are accepted for API parity
//!   but behave as `false`. Explicit offsets in parsed strings ("+02:00",
//!   "CET", ...) are normalized to UTC.
//! - leap seconds (`countLeapSeconds`, `nextLeapSecond`, `prevLeapSecond`):
//!   the `count_leap_seconds` flags behave as `false`. A stored second
//!   value of 60 (from `set_time`) is still tolerated by the
//!   `remove_leap_second(s)` parameters, as in the C++.

use std::cmp::Ordering;

use qalc_num::options::TimeZoneMode;
use qalc_num::{Number, PrintOptions};

/// `SECONDS_PER_DAY`
pub const SECONDS_PER_DAY: i64 = 86400;

/// `isLeapYear` — proleptic Gregorian rule for any year.
pub fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// `daysPerYear(year, basis)` — basis as in the C++ financial functions.
pub fn days_per_year(year: i64, basis: i32) -> i64 {
    match basis {
        0 | 2 | 4 => 360,
        1 => {
            if is_leap_year(year) {
                366
            } else {
                365
            }
        }
        3 => 365,
        _ => -1,
    }
}

/// `daysPerMonth(month, year)`.
pub fn days_per_month(month: i64, year: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Port of the `QalculateDateTime` class.
#[derive(Debug, Clone)]
pub struct QalculateDateTime {
    i_year: i64,
    i_month: i64,
    i_day: i64,
    i_hour: i64,
    i_min: i64,
    n_sec: Number,
    b_time: bool,
    /// `parsed_string` — the original input when constructed from a string.
    pub parsed_string: String,
}

impl Default for QalculateDateTime {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for QalculateDateTime {
    fn eq(&self, o: &Self) -> bool {
        self.i_year == o.i_year
            && self.i_month == o.i_month
            && self.i_day == o.i_day
            && self.i_hour == o.i_hour
            && self.i_min == o.i_min
            && self.n_sec.equals(&o.n_sec, false, false)
    }
}

impl PartialOrd for QalculateDateTime {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        if self.i_year != o.i_year {
            return Some(self.i_year.cmp(&o.i_year));
        }
        if self.i_month != o.i_month {
            return Some(self.i_month.cmp(&o.i_month));
        }
        if self.i_day != o.i_day {
            return Some(self.i_day.cmp(&o.i_day));
        }
        if self.i_hour != o.i_hour {
            return Some(self.i_hour.cmp(&o.i_hour));
        }
        if self.i_min != o.i_min {
            return Some(self.i_min.cmp(&o.i_min));
        }
        if self.n_sec.equals(&o.n_sec, false, false) {
            Some(Ordering::Equal)
        } else if self.n_sec.is_less_than(&o.n_sec) {
            Some(Ordering::Less)
        } else if self.n_sec.is_greater_than(&o.n_sec) {
            Some(Ordering::Greater)
        } else {
            None
        }
    }
}

impl QalculateDateTime {
    /// `QalculateDateTime()` — 0000-01-01, no time set.
    pub fn new() -> Self {
        QalculateDateTime {
            i_year: 0,
            i_month: 1,
            i_day: 1,
            i_hour: 0,
            i_min: 0,
            n_sec: Number::new(),
            b_time: false,
            parsed_string: String::new(),
        }
    }

    /// `QalculateDateTime(year, month, day)` — invalid input leaves the
    /// default value, as in the C++ constructor.
    pub fn from_date(year: i64, month: i64, day: i64) -> Self {
        let mut dt = Self::new();
        dt.set_date(year, month, day);
        dt
    }

    /// `QalculateDateTime(const Number &timestamp)`.
    pub fn from_timestamp(ts: &Number) -> Self {
        let mut dt = Self::new();
        dt.set_timestamp(ts);
        dt
    }

    /// `QalculateDateTime(std::string)`.
    pub fn from_str(s: &str) -> Option<Self> {
        let mut dt = Self::new();
        if dt.set_str(s) {
            Some(dt)
        } else {
            None
        }
    }

    // ------------------------------------------------------------------
    // Accessors
    // ------------------------------------------------------------------

    pub fn year(&self) -> i64 {
        self.i_year
    }
    pub fn month(&self) -> i64 {
        self.i_month
    }
    pub fn day(&self) -> i64 {
        self.i_day
    }
    pub fn hour(&self) -> i64 {
        self.i_hour
    }
    pub fn minute(&self) -> i64 {
        self.i_min
    }
    pub fn second(&self) -> &Number {
        &self.n_sec
    }
    pub fn set_year(&mut self, newyear: i64) {
        self.i_year = newyear;
    }
    pub fn time_is_set(&self) -> bool {
        self.b_time
    }

    // ------------------------------------------------------------------
    // Setting
    // ------------------------------------------------------------------

    /// `set(long newyear, int newmonth, int newday)` — validates against the
    /// proleptic Gregorian calendar and clears the time.
    pub fn set_date(&mut self, newyear: i64, newmonth: i64, newday: i64) -> bool {
        self.parsed_string.clear();
        if newmonth < 1 || newmonth > 12 {
            return false;
        }
        if newday < 1 || newday > days_per_month(newmonth, newyear) {
            return false;
        }
        self.i_year = newyear;
        self.i_month = newmonth;
        self.i_day = newday;
        self.i_hour = 0;
        self.i_min = 0;
        self.n_sec.clear(false);
        self.b_time = false;
        true
    }

    /// `set(const Number &newtimestamp)` — seconds since the Unix epoch.
    ///
    /// TODO(port): the C++ converts the result to the local time zone
    /// (`addMinutes(dateTimeZone(*this, true))`); without time-zone support
    /// the result stays in UTC.
    pub fn set_timestamp(&mut self, newtimestamp: &Number) -> bool {
        self.parsed_string.clear();
        if !newtimestamp.is_real() || newtimestamp.is_interval(false) {
            return false;
        }
        let tmbak = self.clone();
        self.i_year = 1970;
        self.i_month = 1;
        self.i_day = 1;
        self.i_hour = 0;
        self.i_min = 0;
        self.n_sec.clear(false);
        self.b_time = true;
        if !self.add_seconds(newtimestamp, false, false) {
            *self = tmbak;
            return false;
        }
        true
    }

    /// `setTime(hour, min, sec)`.
    pub fn set_time(&mut self, ihour: i64, imin: i64, nsec: &Number) -> bool {
        self.parsed_string.clear();
        self.i_hour = ihour;
        self.i_min = imin;
        self.n_sec = nsec.clone();
        self.b_time = true;
        true
    }

    /// `setToCurrentTime()`.
    ///
    /// TODO(port): uses UTC (whole seconds); the C++ uses local time with
    /// microsecond resolution.
    pub fn set_to_current_time(&mut self) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.set_timestamp(&Number::from_i64(secs));
    }

    /// `setToCurrentDate()`.
    pub fn set_to_current_date(&mut self) {
        self.set_to_current_time();
        self.i_hour = 0;
        self.i_min = 0;
        self.n_sec.clear(false);
        self.b_time = false;
    }

    // ------------------------------------------------------------------
    // Printing
    // ------------------------------------------------------------------

    /// `toISOString()` — "YYYY-MM-DD" or "YYYY-MM-DDTHH:MM:SS". The year is
    /// zero-padded to at least four digits; negative years get a leading
    /// minus. Fractional seconds are truncated, as in the C++.
    pub fn to_iso_string(&self) -> String {
        let mut str = String::new();
        let mut y = self.i_year;
        if y < 0 {
            y = -y;
            str.push('-');
        }
        str.push_str(&format!("{:04}-{:02}-{:02}", y, self.i_month, self.i_day));
        if self.b_time || !self.n_sec.is_zero() || self.i_hour != 0 || self.i_min != 0 {
            let mut nsect = self.n_sec.clone();
            nsect.trunc();
            let sec = nsect.to_i64().unwrap_or(0);
            str.push_str(&format!("T{:02}:{:02}:{:02}", self.i_hour, self.i_min, sec));
        }
        str
    }

    /// `print(const PrintOptions&)` (QalculateDateTime.cc:628).
    ///
    /// Values are stored in UTC here (`set_str` normalizes a parsed offset
    /// away), which is also what the reference's local zone resolves to in the
    /// `--test-file` environment, so `TIME_ZONE_LOCAL` prints the stored value
    /// unchanged and the other two modes shift from it directly.
    ///
    /// TODO(port): locale output (`DATE_TIME_FORMAT_LOCALE` / `toLocalString`).
    pub fn print(&self, po: &PrintOptions) -> String {
        match po.time_zone {
            TimeZoneMode::Local => self.to_iso_string(),
            TimeZoneMode::Utc => format!("{}Z", self.to_iso_string()),
            TimeZoneMode::Custom => {
                let tz = po.custom_time_zone;
                let mut shifted = self.clone();
                shifted.add_minutes(&Number::from_i64(tz as i64), false, false);
                format!(
                    "{}{}{:02}:{:02}",
                    shifted.to_iso_string(),
                    if tz < 0 { '-' } else { '+' },
                    tz.abs() / 60,
                    tz.abs() % 60
                )
            }
        }
    }

    // ------------------------------------------------------------------
    // Arithmetic
    // ------------------------------------------------------------------

    /// `addHours(nhours)`.
    pub fn add_hours(&mut self, nhours: &Number) -> bool {
        let mut nmins = nhours.clone();
        nmins.multiply_i64(60);
        self.add_minutes(&nmins, true, true)
    }

    /// `addMinutes(nminutes, remove_leap_second, convert_to_utc)`.
    ///
    /// TODO(port): `convert_to_utc` is a no-op (no time-zone support).
    pub fn add_minutes(
        &mut self,
        nminutes: &Number,
        remove_leap_second: bool,
        convert_to_utc: bool,
    ) -> bool {
        self.parsed_string.clear();
        if !nminutes.is_real() || nminutes.is_interval(false) {
            return false;
        }
        self.b_time = true;
        if !nminutes.is_integer() {
            let mut newmins = nminutes.clone();
            newmins.trunc();
            let dtbak = self.clone();
            if !self.add_minutes(&newmins, remove_leap_second, convert_to_utc) {
                return false;
            }
            let mut nsec = nminutes.clone();
            nsec.frac();
            nsec.multiply_i64(60);
            if !self.add_seconds(&nsec, false, false) {
                *self = dtbak;
                return false;
            }
            return true;
        }
        let dtbak = self.clone();
        if remove_leap_second && self.n_sec.is_greater_than_or_equal_to(&Number::from_i64(60)) {
            self.n_sec.add_i64(-1);
        }
        let mut nmins = nminutes.clone();
        nmins.divide_i64(60);
        let mut nhours = nmins.clone();
        nhours.trunc();
        nmins.frac();
        nmins.multiply_i64(60);
        self.i_min += nmins.to_i64().unwrap_or(0);
        if self.i_min >= 60 {
            self.i_min -= 60;
            nhours.add_i64(1);
        } else if self.i_min < 0 {
            self.i_min += 60;
            nhours.add_i64(-1);
        }
        nhours.divide_i64(24);
        let mut ndays = nhours.clone();
        ndays.trunc();
        nhours.frac();
        nhours.multiply_i64(24);
        self.i_hour += nhours.to_i64().unwrap_or(0);
        if self.i_hour >= 24 {
            self.i_hour -= 24;
            ndays.add_i64(1);
        } else if self.i_hour < 0 {
            self.i_hour += 24;
            ndays.add_i64(-1);
        }
        if !self.add_days(&ndays) {
            *self = dtbak;
            return false;
        }
        true
    }

    /// `addSeconds(seconds, count_leap_seconds, convert_to_utc)`.
    ///
    /// TODO(port): leap seconds and time zones are not supported;
    /// `count_leap_seconds` and `convert_to_utc` behave as `false`.
    pub fn add_seconds(
        &mut self,
        seconds: &Number,
        _count_leap_seconds: bool,
        _convert_to_utc: bool,
    ) -> bool {
        self.parsed_string.clear();
        if !seconds.is_real() || seconds.is_interval(false) {
            return false;
        }
        if seconds.is_zero() {
            return true;
        }
        let dtbak = self.clone();
        self.b_time = true;
        if !self.n_sec.add(seconds) {
            *self = dtbak;
            return false;
        }
        if self.n_sec.is_negative() {
            if self.n_sec.is_less_than_i64(-60) {
                self.n_sec.divide_i64(60);
                let mut nmins = self.n_sec.clone();
                nmins.trunc();
                self.n_sec.frac();
                self.n_sec.multiply_i64(60);
                if self.n_sec.is_negative() {
                    self.n_sec.add_i64(60);
                    nmins.add_i64(-1);
                }
                if !self.add_minutes(&nmins, false, false) {
                    *self = dtbak;
                    return false;
                }
            } else {
                self.n_sec.add_i64(60);
                if !self.add_minutes(&Number::from_i64(-1), false, false) {
                    *self = dtbak;
                    return false;
                }
            }
        } else if self.n_sec.is_greater_than_or_equal_to(&Number::from_i64(60)) {
            self.n_sec.divide_i64(60);
            let mut nmins = self.n_sec.clone();
            nmins.trunc();
            self.n_sec.frac();
            self.n_sec.multiply_i64(60);
            if !self.add_minutes(&nmins, false, false) {
                *self = dtbak;
                return false;
            }
        }
        true
    }

    /// `addDays(ndays)` — arbitrary-precision day offset. Fractional days
    /// spill into the time of day.
    pub fn add_days(&mut self, ndays: &Number) -> bool {
        self.parsed_string.clear();
        if !ndays.is_real() || ndays.is_interval(false) {
            return false;
        }
        if ndays.is_zero() {
            return true;
        }
        if !ndays.is_integer() {
            let mut newdays = ndays.clone();
            newdays.trunc();
            let dtbak = self.clone();
            if !self.add_days(&newdays) {
                return false;
            }
            let mut nmin = ndays.clone();
            nmin.frac();
            nmin.multiply_i64(1440);
            if !self.add_minutes(&nmin, true, true) {
                *self = dtbak;
                return false;
            }
            return true;
        }

        let mut newmonth = self.i_month;
        let mut newyear = self.i_year;
        let mut newnday = ndays.clone();
        newnday.add_i64(self.i_day);
        if ndays.is_negative() {
            // 146097 days per 400 Gregorian years.
            while newnday.is_less_than_or_equal_to(&Number::from_i64(-14609700)) {
                newnday.add_i64(14609700);
                newyear -= 40000;
            }
            while newnday.is_less_than_or_equal_to(&Number::from_i64(-146097)) {
                newnday.add_i64(146097);
                newyear -= 400;
            }
            loop {
                // In Jan/Feb the previous year's leap day is inside the span.
                let dpy = days_per_year(if newmonth <= 2 { newyear - 1 } else { newyear }, 1);
                if !newnday.is_less_than_or_equal_to(&Number::from_i64(-dpy)) {
                    break;
                }
                newnday.add_i64(dpy);
                newyear -= 1;
            }
            while newnday.is_less_than_i64(1) {
                newmonth -= 1;
                if newmonth < 1 {
                    newyear -= 1;
                    newmonth = 12;
                }
                newnday.add_i64(days_per_month(newmonth, newyear));
            }
        } else {
            while newnday.is_greater_than_i64(14609700) {
                newnday.add_i64(-14609700);
                newyear += 40000;
            }
            while newnday.is_greater_than_i64(146097) {
                newnday.add_i64(-146097);
                newyear += 400;
            }
            loop {
                let dpy = days_per_year(if newmonth <= 2 { newyear } else { newyear + 1 }, 1);
                if !newnday.is_greater_than_i64(dpy) {
                    break;
                }
                newnday.add_i64(-dpy);
                newyear += 1;
            }
            while newnday.is_greater_than_i64(days_per_month(newmonth, newyear)) {
                newnday.add_i64(-days_per_month(newmonth, newyear));
                newmonth += 1;
                if newmonth > 12 {
                    newyear += 1;
                    newmonth = 1;
                }
            }
        }
        let Some(newday) = newnday.to_i64() else {
            return false; // overflow
        };
        self.i_day = newday;
        self.i_month = newmonth;
        self.i_year = newyear;
        true
    }

    /// `addMonths(nmonths)`.
    ///
    /// NOTE (C++ subtlety, ported faithfully): there is *no* month-end
    /// clamping. When the day-of-month does not exist in the target month,
    /// the excess days roll over into the following month, e.g.
    /// 2020-01-31 + 1 month = 2020-03-02.
    pub fn add_months(&mut self, nmonths: &Number) -> bool {
        self.parsed_string.clear();
        if !nmonths.is_real() || nmonths.is_interval(false) {
            return false;
        }
        if !nmonths.is_integer() {
            let mut newmonths = nmonths.clone();
            newmonths.trunc();
            let dtbak = self.clone();
            if !self.add_months(&newmonths) {
                return false;
            }
            let mut nday = nmonths.clone();
            nday.frac();
            let dpm = days_per_month(self.i_month, self.i_year);
            if nday.is_negative() {
                nday.negate();
                nday.multiply_i64(dpm);
                if nday.is_greater_than_or_equal_to(&Number::from_i64(self.i_day - 1)) {
                    // Crosses into the previous month: scale the part beyond
                    // the current month by the previous month's length.
                    nday.divide_i64(dpm);
                    let mut idayfrac = Number::from_i64(self.i_day - 1);
                    let mut secfrac = Number::from_i64(self.i_hour * 3600 + self.i_min * 60);
                    secfrac.add(&self.n_sec);
                    secfrac.divide_i64(86400);
                    idayfrac.add(&secfrac);
                    idayfrac.divide_i64(dpm);
                    nday.subtract(&idayfrac);
                    nday.multiply_i64(days_per_month(
                        if self.i_month == 1 { 12 } else { self.i_month - 1 },
                        self.i_year,
                    ));
                    idayfrac.multiply_i64(dpm);
                    nday.add(&idayfrac);
                }
                nday.negate();
            } else {
                nday.multiply_i64(dpm);
                if nday.is_greater_than_or_equal_to(&Number::from_i64(dpm - self.i_day)) {
                    nday.divide_i64(dpm);
                    let mut idayfrac = Number::from_i64(dpm - self.i_day);
                    let mut secfrac = Number::from_i64(self.i_hour * 3600 + self.i_min * 60);
                    secfrac.add(&self.n_sec);
                    secfrac.divide_i64(86400);
                    idayfrac.subtract(&secfrac);
                    idayfrac.divide_i64(dpm);
                    nday.subtract(&idayfrac);
                    nday.multiply_i64(days_per_month(
                        if self.i_month == 12 { 1 } else { self.i_month + 1 },
                        self.i_year,
                    ));
                    idayfrac.multiply_i64(dpm);
                    nday.add(&idayfrac);
                }
            }
            if !self.add_days(&nday) {
                *self = dtbak;
                return false;
            }
            return true;
        }
        let Some(months) = nmonths.to_i64() else {
            return false;
        };
        let Some(newyear) = self.i_year.checked_add(months / 12) else {
            return false;
        };
        self.i_year = newyear;
        self.i_month += months % 12;
        if self.i_month > 12 {
            self.i_month -= 12;
            self.i_year += 1;
        } else if self.i_month < 1 {
            self.i_month += 12;
            self.i_year -= 1;
        }
        if self.i_day > days_per_month(self.i_month, self.i_year) {
            self.i_day -= days_per_month(self.i_month, self.i_year);
            self.i_month += 1;
            if self.i_month > 12 {
                self.i_month -= 12;
                self.i_year += 1;
            }
        }
        true
    }

    /// `addYears(nyears)`. Same rollover-instead-of-clamp behavior as
    /// `add_months`: 2020-02-29 + 1 year = 2021-03-01.
    pub fn add_years(&mut self, nyears: &Number) -> bool {
        self.parsed_string.clear();
        if !nyears.is_real() || nyears.is_interval(false) {
            return false;
        }
        if !nyears.is_integer() {
            let mut newyears = nyears.clone();
            newyears.trunc();
            let dtbak = self.clone();
            if !self.add_years(&newyears) {
                return false;
            }
            let mut nday = nyears.clone();
            nday.frac();
            if nday.is_zero() {
                return true;
            }
            let idoy = self.yearday();
            let dpy = days_per_year(self.i_year, 1);
            if nday.is_negative() {
                nday.negate();
                nday.multiply_i64(dpy);
                if nday.is_greater_than_or_equal_to(&Number::from_i64(idoy - 1)) {
                    nday.divide_i64(dpy);
                    let mut idayfrac = Number::from_i64(idoy - 1);
                    let mut secfrac = Number::from_i64(self.i_hour * 3600 + self.i_min * 60);
                    secfrac.add(&self.n_sec);
                    secfrac.divide_i64(86400);
                    idayfrac.add(&secfrac);
                    idayfrac.divide_i64(dpy);
                    nday.subtract(&idayfrac);
                    nday.multiply_i64(days_per_year(self.i_year - 1, 1));
                    idayfrac.multiply_i64(dpy);
                    nday.add(&idayfrac);
                }
                nday.negate();
            } else {
                nday.multiply_i64(dpy);
                if nday.is_greater_than_or_equal_to(&Number::from_i64(dpy - idoy)) {
                    nday.divide_i64(dpy);
                    let mut idayfrac = Number::from_i64(idoy - 1);
                    let mut secfrac = Number::from_i64(self.i_hour * 3600 + self.i_min * 60);
                    secfrac.add(&self.n_sec);
                    secfrac.divide_i64(86400);
                    idayfrac.subtract(&secfrac);
                    idayfrac.divide_i64(dpy);
                    nday.subtract(&idayfrac);
                    nday.multiply_i64(days_per_year(self.i_year + 1, 1));
                    idayfrac.multiply_i64(dpy);
                    nday.add(&idayfrac);
                }
            }
            if !self.add_days(&nday) {
                *self = dtbak;
                return false;
            }
            return true;
        }
        let Some(years) = nyears.to_i64() else {
            return false;
        };
        let Some(newyear) = self.i_year.checked_add(years) else {
            return false;
        };
        self.i_year = newyear;
        if self.i_day > days_per_month(self.i_month, self.i_year) {
            self.i_day -= days_per_month(self.i_month, self.i_year);
            self.i_month += 1;
            if self.i_month > 12 {
                self.i_month -= 12;
                self.i_year += 1;
            }
        }
        true
    }

    /// `add(const QalculateDateTime&)` — add another date/time interpreted
    /// as an interval (years + months + days + time).
    pub fn add(&mut self, date: &QalculateDateTime) -> bool {
        self.parsed_string.clear();
        let dtbak = self.clone();
        if date.time_is_set() {
            self.b_time = true;
        }
        if !self.add_years(&Number::from_i64(date.year()))
            || !self.add_months(&Number::from_i64(date.month()))
            || !self.add_days(&Number::from_i64(date.day()))
        {
            *self = dtbak;
            return false;
        }
        if !date.second().is_zero() || date.hour() != 0 || date.minute() != 0 {
            let mut nsec = Number::from_i64(date.hour() * 3600 + date.minute() * 60);
            nsec.add(date.second());
            if !self.add_seconds(&nsec, true, true) {
                *self = dtbak;
                return false;
            }
        }
        true
    }

    // ------------------------------------------------------------------
    // Calendar queries
    // ------------------------------------------------------------------

    /// `weekday()` — ISO: 1 = Monday .. 7 = Sunday. Anchored on 2017-07-31
    /// (a Monday), as in the C++.
    pub fn weekday(&self) -> i32 {
        let mut nr = self.days_to(&QalculateDateTime::from_date(2017, 7, 31), 1, true, true);
        if nr.is_infinite(true) {
            return -1;
        }
        nr.negate();
        nr.trunc();
        nr.rem(&Number::from_i64(7));
        let v = nr.to_i64().unwrap_or(0);
        if v < 0 {
            (8 + v) as i32
        } else {
            (v + 1) as i32
        }
    }

    /// `week(start_sunday)` — ISO 8601 week number by default (week 1 is the
    /// week containing the first Thursday of the year; late-December days
    /// can be week 1 of the next year, early-January days week 52/53 of the
    /// previous year).
    pub fn week(&self, start_sunday: bool) -> i32 {
        if start_sunday {
            let yday = self.yearday();
            let date1 = QalculateDateTime::from_date(self.i_year, 1, 1);
            let mut wday = date1.weekday() + 1;
            if wday < 0 {
                return -1;
            }
            if wday == 8 {
                wday = 1;
            }
            let yday = yday + (wday as i64 - 2);
            let mut week = (yday / 7 + 1) as i32;
            if week > 52 {
                week = 1;
            }
            return week;
        }
        if self.i_month == 12 && self.i_day >= 29 && (self.weekday() as i64) <= self.i_day - 28 {
            return 1;
        }
        // C++ `week_rerun` goto loop: when the date belongs to the last week
        // of the previous year, recompute for Dec 31 of that year.
        let mut date = QalculateDateTime::from_date(self.i_year, self.i_month, self.i_day);
        loop {
            let mut day1 = date.yearday();
            let date1 = QalculateDateTime::from_date(date.year(), 1, 1);
            let wday = date1.weekday();
            if wday < 0 {
                return -1;
            }
            let wday = wday as i64;
            day1 -= 8 - wday;
            let mut week1: i32 = if wday <= 4 { 1 } else { 0 };
            if day1 > 0 {
                day1 -= 1;
                week1 += (day1 / 7 + 1) as i32;
            }
            if week1 == 0 {
                date = QalculateDateTime::from_date(date.year() - 1, 12, 31);
                continue;
            }
            return week1;
        }
    }

    /// `yearday()` — 1-based day of the year.
    pub fn yearday(&self) -> i64 {
        let mut yday = 0;
        for i in 1..self.i_month {
            yday += days_per_month(i, self.i_year);
        }
        yday + self.i_day
    }

    // ------------------------------------------------------------------
    // Differences
    // ------------------------------------------------------------------

    /// `timestamp()` — seconds since the Unix epoch.
    ///
    /// TODO(port): no time-zone conversion (the value is treated as UTC).
    pub fn timestamp(&self) -> Number {
        let epoch = QalculateDateTime::from_date(1970, 1, 1);
        epoch.seconds_to(self, false, false)
    }

    /// `secondsTo(date, count_leap_seconds, convert_to_utc)`.
    ///
    /// TODO(port): leap-second counting and time-zone conversion are not
    /// supported (both flags behave as `false`).
    pub fn seconds_to(
        &self,
        date: &QalculateDateTime,
        count_leap_seconds: bool,
        _convert_to_utc: bool,
    ) -> Number {
        let mut nr = self.days_to(date, 1, true, !count_leap_seconds);
        nr.multiply_i64(SECONDS_PER_DAY);
        nr
    }

    /// `daysTo(date, basis, date_func, remove_leap_seconds)` — (possibly
    /// fractional) days between two dates. `basis` follows the financial
    /// day-count conventions (0/4 = 30/360, 1 = actual with time-of-day,
    /// 2/3 = actual whole days).
    pub fn days_to(
        &self,
        date: &QalculateDateTime,
        basis: i32,
        date_func: bool,
        remove_leap_seconds: bool,
    ) -> Number {
        let basis = if !(0..=4).contains(&basis) { 1 } else { basis };

        let mut neg = false;

        let mut day1 = self.i_day;
        let mut month1 = self.i_month;
        let mut year1 = self.i_year;
        let mut day2 = date.i_day;
        let mut month2 = date.i_month;
        let mut year2 = date.i_year;
        let mut t1 = self.n_sec.clone();
        let mut t2 = date.n_sec.clone();
        let sixty = Number::from_i64(60);
        if remove_leap_seconds {
            if t1.is_greater_than_or_equal_to(&sixty) {
                t1.add_i64(-1);
            }
            if t2.is_greater_than_or_equal_to(&sixty) {
                t2.add_i64(-1);
            }
        }
        t1.add_i64(self.i_hour * 3600 + self.i_min * 60);
        t2.add_i64(date.i_hour * 3600 + date.i_min * 60);

        if year1 > year2
            || (year1 == year2 && month1 > month2)
            || (year1 == year2 && month1 == month2 && day1 > day2)
            || (basis == 1
                && date_func
                && year1 == year2
                && month1 == month2
                && day1 == day2
                && t1.is_greater_than(&t2))
        {
            std::mem::swap(&mut year1, &mut year2);
            std::mem::swap(&mut month1, &mut month2);
            std::mem::swap(&mut day1, &mut day2);
            std::mem::swap(&mut t1, &mut t2);
            neg = true;
        }

        if basis == 0 {
            if month1 == 2
                && month2 == 2
                && day1 == days_per_month(month1, year1)
                && day2 == days_per_month(month2, year2)
            {
                day2 = 30;
            }
            if month1 == 2 && day1 == days_per_month(month1, year1) {
                day1 = 30;
            }
            if day2 == 31 && day1 >= 30 {
                day2 = 30;
            }
            if day1 == 31 {
                day1 = 30;
            }
        } else if basis == 4 {
            if day2 == 31 {
                day2 = 30;
            }
            if day1 == 31 {
                day1 = 30;
            }
        }

        let years = year2 - year1;
        let days = day2 - day1;

        let mut nr;
        match basis {
            0 | 4 => {
                nr = Number::from_i64(years);
                nr.multiply_i64(12);
                nr.add_i64(month2 - month1);
                nr.multiply_i64(30);
                nr.add_i64(days);
            }
            _ => {
                // basis 1, 2, 3
                let mut month4 = month2;
                let mut b = years > 0;
                if b {
                    month4 = 12;
                }
                nr = Number::from_i64(days);
                let mut month1 = month1;
                loop {
                    if !(month1 < month4 || b) {
                        break;
                    }
                    if month1 > month4 && b {
                        b = false;
                        month1 = 1;
                        month4 = month2;
                        if month1 == month2 {
                            break;
                        }
                    }
                    if !b {
                        nr.add_i64(days_per_month(month1, year2));
                    } else {
                        nr.add_i64(days_per_month(month1, year1));
                    }
                    month1 += 1;
                }
                if basis == 1 && !t1.equals(&t2, false, false) {
                    t2.subtract(&t1);
                    t2.divide_i64(86400);
                    nr.add(&t2);
                }
                if years != 0 {
                    for year in (year1 + 1)..year2 {
                        nr.add_i64(if is_leap_year(year) { 366 } else { 365 });
                    }
                }
            }
        }
        if neg {
            nr.negate();
        }
        nr
    }

    /// `yearsTo(date, basis, date_func, remove_leap_seconds)`.
    pub fn years_to(
        &self,
        date: &QalculateDateTime,
        basis: i32,
        date_func: bool,
        remove_leap_seconds: bool,
    ) -> Number {
        let basis = if !(0..=4).contains(&basis) { 1 } else { basis };
        let mut nr;
        if basis == 1 {
            if date.i_year == self.i_year {
                nr = self.days_to(date, basis, date_func, true);
                nr.divide_i64(days_per_year(self.i_year, basis));
            } else {
                let mut neg = false;
                let mut day1 = self.i_day;
                let mut month1 = self.i_month;
                let mut year1 = self.i_year;
                let mut day2 = date.i_day;
                let mut month2 = date.i_month;
                let mut year2 = date.i_year;
                let mut nr_leap = Number::new();
                nr = Number::new();
                let mut t1 = self.n_sec.clone();
                let mut t2 = date.n_sec.clone();
                let sixty = Number::from_i64(60);
                if remove_leap_seconds {
                    if t1.is_greater_than_or_equal_to(&sixty) {
                        t1.add_i64(-1);
                    }
                    if t2.is_greater_than_or_equal_to(&sixty) {
                        t2.add_i64(-1);
                    }
                }
                t1.add_i64(self.i_hour * 3600 + self.i_min * 60);
                t2.add_i64(date.i_hour * 3600 + date.i_min * 60);
                if year1 > year2 {
                    std::mem::swap(&mut year1, &mut year2);
                    std::mem::swap(&mut month1, &mut month2);
                    std::mem::swap(&mut day1, &mut day2);
                    std::mem::swap(&mut t1, &mut t2);
                    neg = true;
                }
                t1.divide_i64(86400);
                t2.divide_i64(86400);
                {
                    let nr_cur = if is_leap_year(year1) { &mut nr_leap } else { &mut nr };
                    for month in (month1 + 1)..=12 {
                        nr_cur.add_i64(days_per_month(month, year1));
                    }
                    nr_cur.add_i64(days_per_month(month1, year1) - day1 + 1);
                    nr_cur.subtract(&t1);
                }
                {
                    let nr_cur = if is_leap_year(year2) { &mut nr_leap } else { &mut nr };
                    for month in 1..month2 {
                        nr_cur.add_i64(days_per_month(month, year2));
                    }
                    nr_cur.add_i64(day2 - 1);
                    nr_cur.add(&t2);
                }
                for year in (year1 + 1)..year2 {
                    if is_leap_year(year) {
                        nr_leap.add_i64(days_per_year(year, basis));
                    } else {
                        nr.add_i64(days_per_year(year, basis));
                    }
                }
                nr_leap.divide_i64(366);
                nr.divide_i64(365);
                nr.add(&nr_leap);
                if neg {
                    nr.negate();
                }
            }
        } else {
            nr = self.days_to(date, basis, date_func, true);
            nr.divide_i64(days_per_year(0, basis));
        }
        nr
    }

    // ------------------------------------------------------------------
    // Parsing (`set(std::string)`)
    // ------------------------------------------------------------------

    /// `set(std::string)` — parse a date/time string.
    ///
    /// Supported (as in the C++): "now"/"today"/"tomorrow"/"yesterday",
    /// ISO "[-]YYYY-MM-DD", compact "YYYYMMDD", "M/D/Y", "D.M.Y" (any
    /// single-character separators) with the C++ field-swap heuristics and
    /// two-digit-year window, optional "T"- or space-separated time
    /// "HH:MM[:SS]" and a time-zone suffix ("Z", named zone or "±HH[:MM]",
    /// normalized to UTC).
    ///
    /// TODO(port): locale-specific formats (strptime "%x"/"%X"), compact
    /// time "HHMMSS" and compact two-digit-year dates "YYMMDD" are not
    /// supported.
    pub fn set_str(&mut self, date_string: &str) -> bool {
        let str_bak = date_string.to_string();
        let mut str: String = date_string.trim().to_string();
        while str.contains("  ") {
            str = str.replace("  ", " ");
        }

        match str.to_ascii_lowercase().as_str() {
            "now" => {
                self.set_to_current_time();
                self.parsed_string = str_bak;
                return true;
            }
            "today" => {
                self.set_to_current_date();
                self.parsed_string = str_bak;
                return true;
            }
            "tomorrow" => {
                self.set_to_current_date();
                self.add_days(&Number::from_i64(1));
                self.parsed_string = str_bak;
                return true;
            }
            "yesterday" => {
                self.set_to_current_date();
                self.add_days(&Number::from_i64(-1));
                self.parsed_string = str_bak;
                return true;
            }
            _ => {}
        }

        // Split off a time part: after 'T' (followed by a digit), or after a
        // space preceding "HH:MM".
        let mut b_t = false;
        let mut b_tz = false;
        let mut itz: i64 = 0;
        let mut newhour: i64 = 0;
        let mut newmin: i64 = 0;
        let mut newsec: i64 = 0;
        let mut time_part: Option<String> = None;
        if let Some(i_t) = str.find('T') {
            if str[i_t..].bytes().any(|c| c.is_ascii_digit()) {
                time_part = Some(str[i_t + 1..].trim().to_string());
                str.truncate(i_t);
                str = str.trim().to_string();
            }
        }
        if time_part.is_none() {
            if let Some(i_c) = str[1..].find(':').map(|p| p + 1) {
                let bytes = str.as_bytes();
                let mut start = i_c;
                while start > 0 && bytes[start - 1].is_ascii_digit() {
                    start -= 1;
                }
                if start > 0 && start < i_c && bytes[start - 1] == b' ' {
                    time_part = Some(str[start..].trim().to_string());
                    str.truncate(start - 1);
                    str = str.trim().to_string();
                } else {
                    // TODO(port): compact date+time without separator
                    // ("YYYYMMDDHH:MM") is not supported.
                    return false;
                }
            }
        }
        if let Some(time_str) = time_part {
            b_t = true;
            let Some((h, m, s, tz)) = parse_time(&time_str) else {
                return false;
            };
            newhour = h;
            newmin = m;
            newsec = s;
            if let Some(tzmin) = tz {
                b_tz = true;
                itz = tzmin;
            }
        }
        if newhour >= 24
            || newmin >= 60
            || newsec > 60
            || (newsec == 60 && (newhour != 23 || newmin != 59))
        {
            return false;
        }

        // The C++ replaces the Unicode minus sign with ASCII '-'.
        let mut str = str.replace('\u{2212}', "-");
        if !b_t && str.len() > 1 && (str.ends_with('Z') || str.ends_with('z')) {
            b_t = true;
            b_tz = true;
            str.pop();
        }

        let ymd = parse_iso_dashed(&str)
            .or_else(|| parse_compact_ymd(&str))
            .or_else(|| parse_separated(&str));
        let Some((newyear, newmonth, newday)) = ymd else {
            return false;
        };
        if !self.set_date(newyear, newmonth, newday) {
            return false;
        }
        if b_t {
            self.b_time = true;
            self.i_hour = newhour;
            self.i_min = newmin;
            self.n_sec = Number::from_i64(newsec);
            if b_tz && itz != 0 {
                // TODO(port): the C++ converts to the local zone
                // (`dateTimeZone(*this, true) - itz`); we normalize to UTC.
                if !self.add_minutes(&Number::from_i64(-itz), false, false) {
                    return false;
                }
            }
        }
        self.parsed_string = str_bak;
        true
    }
}

// ----------------------------------------------------------------------
// Parsing helpers
// ----------------------------------------------------------------------

/// Parse "HH:MM[:SS]" plus an optional time-zone suffix. Returns
/// (hour, minute, second, tz offset in minutes; `None` = no zone given).
fn parse_time(time_str: &str) -> Option<(i64, i64, i64, Option<i64>)> {
    let bytes = time_str.as_bytes();
    let mut pos = 0usize;
    let h = parse_digits(bytes, &mut pos, 2)?;
    if pos >= bytes.len() || bytes[pos] != b':' {
        // TODO(port): compact "HHMMSS" time is not supported.
        return None;
    }
    pos += 1;
    let m = parse_digits(bytes, &mut pos, 2)?;
    let mut s = 0;
    if pos < bytes.len() && bytes[pos] == b':' {
        pos += 1;
        s = parse_digits(bytes, &mut pos, 2)?;
    }
    let stz: String = time_str[pos..].split_whitespace().collect();
    let tz = if stz.is_empty() {
        None
    } else {
        Some(parse_time_zone(&stz)?)
    };
    Some((h, m, s, tz))
}

/// The C++ named-zone table plus "±HH[:MM]" offsets, in minutes.
fn parse_time_zone(stz: &str) -> Option<i64> {
    match stz {
        "Z" | "GMT" | "UTC" | "WET" => return Some(0),
        "CET" | "WEST" => return Some(60),
        "CEST" | "EET" => return Some(2 * 60),
        "EEST" => return Some(3 * 60),
        "CT" | "CST" => return Some(8 * 60),
        "JST" => return Some(9 * 60),
        "EDT" => return Some(-4 * 60),
        "EST" => return Some(-5 * 60),
        "PDT" | "MST" => return Some(-7 * 60),
        "PST" => return Some(-8 * 60),
        _ => {}
    }
    let bytes = stz.as_bytes();
    if stz.len() > 1 && (bytes[0] == b'-' || bytes[0] == b'+') {
        let neg = bytes[0] == b'-';
        let rest = &stz[1..];
        let itz = if let Some(colon) = rest.find(':') {
            let (hs, ms) = (&rest[..colon], &rest[colon + 1..]);
            if hs.is_empty() || hs.len() > 2 || ms.is_empty() || ms.len() > 2 {
                return None;
            }
            if !hs.bytes().all(|c| c.is_ascii_digit()) || !ms.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            hs.parse::<i64>().ok()? * 60 + ms.parse::<i64>().ok()?
        } else {
            if rest.is_empty() || rest.len() > 2 || !rest.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            rest.parse::<i64>().ok()? * 60
        };
        return Some(if neg { -itz } else { itz });
    }
    None
}

fn parse_digits(bytes: &[u8], pos: &mut usize, max_digits: usize) -> Option<i64> {
    let start = *pos;
    while *pos < bytes.len() && *pos - start < max_digits && bytes[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == start {
        return None;
    }
    std::str::from_utf8(&bytes[start..*pos]).ok()?.parse().ok()
}

/// "[-]Y-M-D" (sscanf "%ld-%lu-%lu"), including the C++ "DD-MM-YYYY" swap.
fn parse_iso_dashed(s: &str) -> Option<(i64, i64, i64)> {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let parts: Vec<&str> = rest.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    for p in &parts {
        if p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    let mut newyear: i64 = parts[0].parse().ok()?;
    let newmonth: i64 = parts[1].parse().ok()?;
    let mut newday: i64 = parts[2].parse().ok()?;
    if neg {
        newyear = -newyear;
    }
    // "15-03-2020": day-first with dashes.
    if newyear <= 31 && newyear > 0 && newday > 999 && newmonth <= 12 {
        std::mem::swap(&mut newyear, &mut newday);
    }
    Some((newyear, newmonth, newday))
}

/// "YYYYMMDD" (sscanf "%4ld%2lu%2lu").
fn parse_compact_ymd(s: &str) -> Option<(i64, i64, i64)> {
    if s.len() != 8 || !s.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((
        s[..4].parse().ok()?,
        s[4..6].parse().ok()?,
        s[6..8].parse().ok()?,
    ))
}

/// "M/D/Y" (US order with slashes) or "D<sep>M<sep>Y" with arbitrary
/// single-character separators, plus the C++ field-swap heuristics and
/// two-digit-year expansion.
fn parse_separated(s: &str) -> Option<(i64, i64, i64)> {
    let mut nums: Vec<i64> = Vec::new();
    let mut seps: Vec<char> = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    // Optional leading sign for the first number (sscanf %ld).
    if let Some(&c) = chars.peek() {
        if c == '-' || c == '+' {
            cur.push(c);
            chars.next();
        }
    }
    for c in chars {
        if c.is_ascii_digit() {
            cur.push(c);
        } else {
            if cur.is_empty() || cur == "-" || cur == "+" {
                return None;
            }
            nums.push(cur.parse().ok()?);
            cur.clear();
            seps.push(c);
        }
    }
    if cur.is_empty() || cur == "-" || cur == "+" {
        return None;
    }
    nums.push(cur.parse().ok()?);
    if nums.len() != 3 || seps.len() != 2 {
        return None;
    }
    let (mut newyear, mut newmonth, mut newday);
    if seps[0] == '/' && seps[1] == '/' {
        // sscanf "%ld/%ld/%ld" — month/day/year.
        newmonth = nums[0];
        newday = nums[1];
        newyear = nums[2];
    } else {
        // sscanf "%ld%1c%ld%1c%ld" — day/month/year, then inner swaps.
        newday = nums[0];
        newmonth = nums[1];
        newyear = nums[2];
        if newday > 31 {
            std::mem::swap(&mut newday, &mut newyear);
        }
        if newmonth > 12 {
            std::mem::swap(&mut newday, &mut newmonth);
        }
    }
    if newmonth > 12 {
        std::mem::swap(&mut newday, &mut newmonth);
    }
    if newday > 31 {
        std::mem::swap(&mut newday, &mut newyear);
    }
    if (0..100).contains(&newyear) {
        // C++ sliding window against the current year (localtime tm_year).
        let mut now = QalculateDateTime::new();
        now.set_to_current_time();
        if newyear + 70 > now.year() - 1900 {
            newyear += 1900;
        } else {
            newyear += 2000;
        }
    }
    Some((newyear, newmonth, newday))
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(y: i64, m: i64, d: i64) -> QalculateDateTime {
        let mut dt = QalculateDateTime::new();
        assert!(dt.set_date(y, m, d), "invalid test date {y}-{m}-{d}");
        dt
    }

    fn iso(dt: &QalculateDateTime) -> String {
        dt.to_iso_string()
    }

    #[test]
    fn leap_years_proleptic() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2020));
        assert!(is_leap_year(0)); // proleptic: year 0 is a leap year
        assert!(is_leap_year(-44));
        assert!(is_leap_year(1600));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(1500)); // proleptic Gregorian, not Julian
        assert!(!is_leap_year(2021));
    }

    #[test]
    fn days_per_month_feb() {
        assert_eq!(days_per_month(2, 2020), 29);
        assert_eq!(days_per_month(2, 2021), 28);
        assert_eq!(days_per_month(2, 1900), 28);
        assert_eq!(days_per_month(1, 2021), 31);
        assert_eq!(days_per_month(4, 2021), 30);
        assert_eq!(days_per_month(12, 2021), 31);
    }

    #[test]
    fn set_date_validation() {
        let mut d = QalculateDateTime::new();
        assert!(!d.set_date(2021, 2, 29));
        assert!(d.set_date(2020, 2, 29));
        assert!(!d.set_date(2021, 13, 1));
        assert!(!d.set_date(2021, 0, 1));
        assert!(!d.set_date(2021, 4, 31));
        assert!(d.set_date(2021, 4, 30));
    }

    #[test]
    fn add_days_batch_case() {
        // dates.batch: "2020-05-20" + 523d -> "2021-10-25"
        let mut d = dt(2020, 5, 20);
        assert!(d.add_days(&Number::from_i64(523)));
        assert_eq!(iso(&d), "2021-10-25");
    }

    #[test]
    fn add_days_negative_across_year() {
        let mut d = dt(2000, 1, 1);
        assert!(d.add_days(&Number::from_i64(-1)));
        assert_eq!(iso(&d), "1999-12-31");
        // Back across the century leap-day boundary.
        let mut d = dt(2000, 3, 1);
        assert!(d.add_days(&Number::from_i64(-366)));
        assert_eq!(iso(&d), "1999-03-01");
    }

    #[test]
    fn add_days_fractional() {
        let mut d = dt(2020, 5, 20);
        assert!(d.add_days(&Number::from_ints(3, 2, 0))); // 1.5 days
        assert_eq!(iso(&d), "2020-05-21T12:00:00");
    }

    #[test]
    fn days_to_batch_cases() {
        // dates.batch: "2020-11-05" - "2020-10-05" -> 31 d
        let nr = dt(2020, 10, 5).days_to(&dt(2020, 11, 5), 1, true, true);
        assert_eq!(nr.to_i64(), Some(31));
        // dates.batch: "2020-10-05" - "2020-10-15" -> -10 d
        let nr = dt(2020, 10, 15).days_to(&dt(2020, 10, 5), 1, true, true);
        assert_eq!(nr.to_i64(), Some(-10));
    }

    #[test]
    fn days_to_century() {
        // days(1900-01-01, 2000-01-01) = 36524 (1900 is not a leap year)
        let nr = dt(1900, 1, 1).days_to(&dt(2000, 1, 1), 1, true, true);
        assert_eq!(nr.to_i64(), Some(36524));
        let nr = dt(2000, 1, 1).days_to(&dt(2100, 1, 1), 1, true, true);
        assert_eq!(nr.to_i64(), Some(36525)); // 2000 is a leap year
    }

    #[test]
    fn timestamp_batch_case() {
        // dates.batch: timestamp(2020-05-20T00:00:00Z) -> 1589932800
        let d = dt(2020, 5, 20);
        assert_eq!(d.timestamp().to_i64(), Some(1589932800));
        // Round trip (stamptodate).
        let mut d2 = QalculateDateTime::new();
        assert!(d2.set_timestamp(&Number::from_i64(1589932800)));
        assert_eq!(iso(&d2), "2020-05-20T00:00:00");
        assert_eq!(d2.timestamp().to_i64(), Some(1589932800));
    }

    #[test]
    fn add_months_rollover_not_clamped() {
        // C++ addMonths rolls excess days into the next month.
        let mut d = dt(2020, 1, 31);
        assert!(d.add_months(&Number::from_i64(1)));
        assert_eq!(iso(&d), "2020-03-02"); // not 2020-02-29
        let mut d = dt(2021, 1, 31);
        assert!(d.add_months(&Number::from_i64(1)));
        assert_eq!(iso(&d), "2021-03-03");
        let mut d = dt(2020, 3, 31);
        assert!(d.add_months(&Number::from_i64(-1)));
        assert_eq!(iso(&d), "2020-03-02"); // Feb 31 -> Mar 2
        let mut d = dt(2020, 5, 20);
        assert!(d.add_months(&Number::from_i64(13)));
        assert_eq!(iso(&d), "2021-06-20");
    }

    #[test]
    fn add_years_leap_day_rollover() {
        let mut d = dt(2020, 2, 29);
        assert!(d.add_years(&Number::from_i64(1)));
        assert_eq!(iso(&d), "2021-03-01"); // not clamped to Feb 28
        let mut d = dt(2020, 2, 29);
        assert!(d.add_years(&Number::from_i64(4)));
        assert_eq!(iso(&d), "2024-02-29");
    }

    #[test]
    fn add_seconds_and_minutes_rollover() {
        let mut d = dt(2020, 12, 31);
        assert!(d.set_time(23, 59, &Number::from_i64(30)));
        assert!(d.add_seconds(&Number::from_i64(45), true, true));
        assert_eq!(iso(&d), "2021-01-01T00:00:15");
        // Backwards across midnight.
        assert!(d.add_minutes(&Number::from_i64(-2), true, true));
        assert_eq!(iso(&d), "2020-12-31T23:58:15");
        // add_hours across a day boundary.
        assert!(d.add_hours(&Number::from_i64(2)));
        assert_eq!(iso(&d), "2021-01-01T01:58:15");
    }

    #[test]
    fn seconds_to_with_time_of_day() {
        let mut d1 = dt(2020, 5, 20);
        d1.set_time(12, 0, &Number::from_i64(0));
        let mut d2 = dt(2020, 5, 21);
        d2.set_time(13, 30, &Number::from_i64(15));
        let nr = d1.seconds_to(&d2, false, false);
        assert_eq!(nr.to_i64(), Some(86400 + 3600 + 30 * 60 + 15));
        let nr = d2.seconds_to(&d1, false, false);
        assert_eq!(nr.to_i64(), Some(-(86400 + 3600 + 30 * 60 + 15)));
    }

    #[test]
    fn weekday_anchor_and_known_days() {
        assert_eq!(dt(2017, 7, 31).weekday(), 1); // Monday (C++ anchor)
        assert_eq!(dt(2000, 1, 1).weekday(), 6); // Saturday
        assert_eq!(dt(2026, 7, 26).weekday(), 7); // Sunday
        assert_eq!(dt(2017, 7, 30).weekday(), 7); // Sunday (before anchor)
        assert_eq!(dt(1970, 1, 1).weekday(), 4); // Thursday
    }

    #[test]
    fn iso_week_numbers() {
        // Jan 1 2021 (Friday) belongs to ISO week 53 of 2020.
        assert_eq!(dt(2021, 1, 1).week(false), 53);
        // Dec 31 2018 (Monday) belongs to ISO week 1 of 2019.
        assert_eq!(dt(2018, 12, 31).week(false), 1);
        // Jan 3 2016 (Sunday) belongs to ISO week 53 of 2015.
        assert_eq!(dt(2016, 1, 3).week(false), 53);
        assert_eq!(dt(2020, 12, 31).week(false), 53);
        assert_eq!(dt(2021, 1, 4).week(false), 1);
        assert_eq!(dt(2017, 7, 31).week(false), 31);
    }

    #[test]
    fn yearday_values() {
        assert_eq!(dt(2020, 1, 1).yearday(), 1);
        assert_eq!(dt(2020, 12, 31).yearday(), 366);
        assert_eq!(dt(2021, 12, 31).yearday(), 365);
        assert_eq!(dt(2020, 3, 1).yearday(), 61);
        assert_eq!(dt(2021, 3, 1).yearday(), 60);
    }

    #[test]
    fn parse_iso_strings() {
        let d = QalculateDateTime::from_str("2020-05-20").unwrap();
        assert_eq!((d.year(), d.month(), d.day()), (2020, 5, 20));
        assert!(!d.time_is_set());

        let d = QalculateDateTime::from_str("2020-05-20T14:30:15").unwrap();
        assert_eq!((d.year(), d.month(), d.day()), (2020, 5, 20));
        assert_eq!((d.hour(), d.minute()), (14, 30));
        assert_eq!(d.second().to_i64(), Some(15));
        assert!(d.time_is_set());
        assert_eq!(iso(&d), "2020-05-20T14:30:15");

        let d = QalculateDateTime::from_str("2020-05-20 14:30").unwrap();
        assert_eq!(iso(&d), "2020-05-20T14:30:00");

        let d = QalculateDateTime::from_str("20200520").unwrap();
        assert_eq!(iso(&d), "2020-05-20");

        let d = QalculateDateTime::from_str("-0044-03-15").unwrap();
        assert_eq!((d.year(), d.month(), d.day()), (-44, 3, 15));
        assert_eq!(iso(&d), "-0044-03-15");

        assert!(QalculateDateTime::from_str("2021-02-29").is_none());
        assert!(QalculateDateTime::from_str("2021-13-01").is_none());
        assert!(QalculateDateTime::from_str("hello").is_none());
    }

    #[test]
    fn parse_offset_normalized_to_utc() {
        // dates.batch: "2020-07-10T07:50CET" is 06:50 UTC.
        let d = QalculateDateTime::from_str("2020-07-10T07:50CET").unwrap();
        assert_eq!(iso(&d), "2020-07-10T06:50:00");
        let d = QalculateDateTime::from_str("2020-07-10T14:50:00+08:00").unwrap();
        assert_eq!(iso(&d), "2020-07-10T06:50:00");
        let d = QalculateDateTime::from_str("2020-05-20T00:00:00Z").unwrap();
        assert_eq!(d.timestamp().to_i64(), Some(1589932800));
    }

    #[test]
    fn parse_fallback_formats() {
        // D.M.Y with arbitrary separators.
        let d = QalculateDateTime::from_str("20.5.2020").unwrap();
        assert_eq!(iso(&d), "2020-05-20");
        // US M/D/Y.
        let d = QalculateDateTime::from_str("5/20/2020").unwrap();
        assert_eq!(iso(&d), "2020-05-20");
        // Y first with dots (day > 31 triggers the year swap).
        let d = QalculateDateTime::from_str("2020.5.20").unwrap();
        assert_eq!(iso(&d), "2020-05-20");
        // DD-MM-YYYY via the ISO-branch swap.
        let d = QalculateDateTime::from_str("15-03-2020").unwrap();
        assert_eq!(iso(&d), "2020-03-15");
        // Two-digit year window: 99 -> 1999.
        let d = QalculateDateTime::from_str("1.1.99").unwrap();
        assert_eq!(iso(&d), "1999-01-01");
    }

    #[test]
    fn comparisons() {
        let a = dt(2020, 5, 20);
        let b = dt(2020, 5, 21);
        assert!(a < b);
        assert!(b > a);
        assert!(a == dt(2020, 5, 20));
        let mut c = dt(2020, 5, 20);
        c.set_time(0, 0, &Number::from_i64(1));
        assert!(c > a);
        assert!(a <= c && a != c);
    }

    #[test]
    fn years_to_basis1() {
        let nr = dt(2000, 1, 1).years_to(&dt(2001, 1, 1), 1, true, true);
        assert!(nr.equals_i64(1), "expected 1, got {nr:?}");
        let nr = dt(2001, 1, 1).years_to(&dt(2000, 1, 1), 1, true, true);
        assert!(nr.equals_i64(-1), "expected -1, got {nr:?}");
        // Half of a non-leap year within one year.
        let mut nr = dt(2021, 1, 1).years_to(&dt(2021, 12, 31), 1, true, true);
        nr.multiply_i64(365);
        assert!(nr.equals_i64(364), "expected 364/365 years, got {nr:?}");
    }

    #[test]
    fn add_interval_datetime() {
        // 2020-05-20 + (1 year, 2 months, 10 days)
        let mut d = dt(2020, 5, 20);
        let mut ival = QalculateDateTime::new();
        ival.i_year = 1;
        ival.i_month = 2;
        ival.i_day = 10;
        assert!(d.add(&ival));
        assert_eq!(iso(&d), "2021-07-30");
    }
}

#[cfg(test)]
mod time_zone_print_tests {
    use super::*;

    #[test]
    fn zone_modes_shift_and_suffix() {
        let dt = QalculateDateTime::from_str("2020-07-10T07:50CET").expect("parses");
        let mut po = PrintOptions::default();
        // Stored in UTC: CET is one hour east.
        assert_eq!(dt.print(&po), "2020-07-10T06:50:00");
        po.time_zone = TimeZoneMode::Utc;
        assert_eq!(dt.print(&po), "2020-07-10T06:50:00Z");
        po.time_zone = TimeZoneMode::Custom;
        po.custom_time_zone = 8 * 60;
        assert_eq!(dt.print(&po), "2020-07-10T14:50:00+08:00");
        po.custom_time_zone = -(5 * 60 + 30);
        assert_eq!(dt.print(&po), "2020-07-10T01:20:00-05:30");
    }
}
