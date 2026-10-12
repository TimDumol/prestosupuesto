//! Exact-decimal finance projections used during the database shadow phase.
//!
//! Amounts remain decimal text at the persistence boundary. FX calculations
//! use rational arithmetic until the final posting precision is applied.

use std::{collections::BTreeMap, str::FromStr};

use num_bigint::BigInt;

// Owner-approved 2% proxy for suboptimal non-EUR transaction FX and possible
// foreign transaction fees. Imported historical valuations remain frozen.
const SOURCE_FX_ADJUSTMENT: &str = "1.02";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Decimal {
    coefficient: BigInt,
    scale: u32,
}

impl Decimal {
    fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value.is_empty() {
            return Err("empty decimal".into());
        }
        let (negative, unsigned) = if let Some(rest) = value.strip_prefix('-') {
            (true, rest)
        } else if let Some(rest) = value.strip_prefix('+') {
            (false, rest)
        } else {
            (false, value)
        };
        let mut exponent_parts = unsigned.split(['e', 'E']);
        let mantissa = exponent_parts.next().unwrap_or_default();
        let exponent = exponent_parts
            .next()
            .map(|part| part.parse::<i32>().map_err(|_| "invalid exponent"))
            .transpose()?
            .unwrap_or(0);
        if exponent_parts.next().is_some() || exponent.unsigned_abs() > 10_000 {
            return Err("unsupported decimal exponent".into());
        }
        let mut dot = false;
        let mut fractional_digits = 0i32;
        let mut digits = String::new();
        for character in mantissa.chars() {
            match character {
                '.' if !dot => dot = true,
                '.' => return Err("invalid decimal point".into()),
                '0'..='9' => {
                    digits.push(character);
                    if dot {
                        fractional_digits += 1;
                    }
                }
                _ => return Err("invalid decimal digit".into()),
            }
        }
        if digits.is_empty() {
            return Err("decimal has no digits".into());
        }
        let mut scale = fractional_digits - exponent;
        if scale < 0 {
            digits.extend(std::iter::repeat_n('0', (-scale) as usize));
            scale = 0;
        }
        if scale > 10_000 {
            return Err("decimal scale is too large".into());
        }
        let mut coefficient = BigInt::from_str(&digits).map_err(|_| "invalid digits")?;
        if negative {
            coefficient = -coefficient;
        }
        Ok(Self {
            coefficient,
            scale: scale as u32,
        }
        .normalized())
    }

    fn normalized(mut self) -> Self {
        while self.scale > 0 && (&self.coefficient % 10u8) == BigInt::from(0u8) {
            self.coefficient /= 10u8;
            self.scale -= 1;
        }
        self
    }

    fn add(&self, other: &Self) -> Self {
        let scale = self.scale.max(other.scale);
        let left = &self.coefficient * ten_pow(scale - self.scale);
        let right = &other.coefficient * ten_pow(scale - other.scale);
        Self {
            coefficient: left + right,
            scale,
        }
        .normalized()
    }

    fn negated(&self) -> Self {
        Self {
            coefficient: -&self.coefficient,
            scale: self.scale,
        }
    }

    fn as_ratio(&self) -> Ratio {
        Ratio {
            numerator: self.coefficient.clone(),
            denominator: ten_pow(self.scale),
        }
        .normalized()
    }

    fn round_decimal(&self, scale: u32, mode: RoundingMode) -> Self {
        self.as_ratio().round_decimal(scale, mode)
    }

    fn format(&self) -> String {
        let negative = self.coefficient < BigInt::from(0u8);
        let mut digits = self.coefficient.magnitude().to_string();
        if self.scale > 0 {
            let scale = self.scale as usize;
            if digits.len() <= scale {
                digits = format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits);
            }
            digits.insert(digits.len() - scale, '.');
        }
        if negative && self.coefficient != BigInt::from(0u8) {
            format!("-{digits}")
        } else {
            digits
        }
    }
}

#[derive(Clone, Debug)]
struct Ratio {
    numerator: BigInt,
    denominator: BigInt,
}

impl Ratio {
    fn normalized(mut self) -> Self {
        if self.denominator < BigInt::from(0u8) {
            self.numerator = -self.numerator;
            self.denominator = -self.denominator;
        }
        self
    }

    fn multiply(&self, other: &Self) -> Self {
        Self {
            numerator: &self.numerator * &other.numerator,
            denominator: &self.denominator * &other.denominator,
        }
        .normalized()
    }

    fn divide(&self, other: &Self) -> Result<Self, String> {
        if other.numerator == BigInt::from(0u8) {
            return Err("cannot divide by a zero FX quote".into());
        }
        Ok(Self {
            numerator: &self.numerator * &other.denominator,
            denominator: &self.denominator * &other.numerator,
        }
        .normalized())
    }

    fn round_decimal(&self, scale: u32, mode: RoundingMode) -> Decimal {
        let scaled_numerator = &self.numerator * ten_pow(scale);
        let mut quotient = &scaled_numerator / &self.denominator;
        let remainder = &scaled_numerator % &self.denominator;
        if remainder != BigInt::from(0u8) {
            let twice_remainder = remainder.magnitude() * 2u8;
            let denominator = self.denominator.magnitude();
            let greater = twice_remainder > *denominator;
            let exact_half = twice_remainder == *denominator;
            let increment = match mode {
                RoundingMode::TowardZero => false,
                RoundingMode::HalfAwayFromZero => greater || exact_half,
                RoundingMode::HalfEven => {
                    greater || (exact_half && (&quotient % 2u8) != BigInt::from(0u8))
                }
            };
            if increment {
                if self.numerator < BigInt::from(0u8) {
                    quotient -= 1u8;
                } else {
                    quotient += 1u8;
                }
            }
        }
        Decimal {
            coefficient: quotient,
            scale,
        }
        .normalized()
    }
}

fn ten_pow(scale: u32) -> BigInt {
    BigInt::from(10u8).pow(scale)
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum RoundingMode {
    HalfAwayFromZero,
    HalfEven,
    TowardZero,
}

/// Owner-selected rounding for new posted monetary values.
pub(crate) const POSTING_ROUNDING_MODE: RoundingMode = RoundingMode::HalfEven;

#[allow(dead_code)]
pub(crate) fn round_posted_decimal_text(value: &str, scale: u32) -> Result<String, String> {
    Ok(Decimal::parse(value)?
        .round_decimal(scale, POSTING_ROUNDING_MODE)
        .format())
}

#[allow(dead_code)]
pub(crate) fn add_decimal_text(left: &str, right: &str) -> Result<String, String> {
    Ok(Decimal::parse(left)?.add(&Decimal::parse(right)?).format())
}

#[allow(dead_code)]
pub(crate) fn subtract_decimal_text(left: &str, right: &str) -> Result<String, String> {
    Ok(Decimal::parse(left)?
        .add(&Decimal::parse(right)?.negated())
        .format())
}

#[allow(dead_code)]
pub(crate) fn multiply_decimal_text(left: &str, right: &str) -> Result<String, String> {
    Ok(Decimal::parse(left)?
        .as_ratio()
        .multiply(&Decimal::parse(right)?.as_ratio())
        .round_decimal(2, POSTING_ROUNDING_MODE)
        .format())
}

/// Apply a proportion without intermediate rounding; post the final EUR or
/// native amount using the shared half-even policy.
pub(crate) fn proportional_decimal_text(value: &str, numerator: &str, denominator: &str) -> Result<String,String> {
    Ok(Decimal::parse(value)?.as_ratio()
        .multiply(&Decimal::parse(numerator)?.as_ratio())
        .divide(&Decimal::parse(denominator)?.as_ratio())?
        .round_decimal(2, POSTING_ROUNDING_MODE).format())
}

pub(crate) fn account_balance_eur(
    native_balance: &str,
    currency: &str,
    as_of: &str,
    observations: &[RateObservation],
) -> Result<String, String> {
    let native_balance = Decimal::parse(native_balance)?;
    let quote = Decimal::parse(&select_prior_rate(currency, as_of, observations)?.quote)?;
    Ok(native_balance
        .as_ratio()
        .divide(&quote.as_ratio())?
        .round_decimal(2, POSTING_ROUNDING_MODE)
        .format())
}

#[derive(Clone, Debug)]
pub(crate) struct RateObservation {
    pub currency: String,
    pub date: String,
    /// ECB quote: units of this currency for one EUR.
    pub currency_units_per_eur: String,
}

#[derive(Clone, Debug)]
pub(crate) struct SelectedRate {
    pub quote: String,
    pub date: Option<String>,
    pub stale_business_days: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct SideValuation {
    pub amount_native: String,
    pub amount_eur: String,
    pub exchange_to_eur_numerator: String,
    pub exchange_to_eur_denominator: String,
    pub exchange_to_native_numerator: String,
    pub exchange_to_native_denominator: String,
    pub applied_rate_dates: BTreeMap<String, Option<String>>,
    pub stale: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct TransactionValuation {
    pub from: SideValuation,
    pub to: SideValuation,
}

pub(crate) fn select_prior_rate(
    currency: &str,
    as_of: &str,
    observations: &[RateObservation],
) -> Result<SelectedRate, String> {
    if currency == "EUR" {
        return Ok(SelectedRate {
            quote: "1".into(),
            date: None,
            stale_business_days: 0,
        });
    }
    let selected = observations
        .iter()
        .filter(|rate| rate.currency == currency && rate.date.as_str() <= as_of)
        .max_by(|left, right| left.date.cmp(&right.date))
        .ok_or_else(|| format!("no prior ECB observation for {currency}"))?;
    let quote = Decimal::parse(&selected.currency_units_per_eur)?;
    if quote.coefficient <= BigInt::from(0u8) {
        return Err("ECB quote must be positive".into());
    }
    Ok(SelectedRate {
        quote: selected.currency_units_per_eur.clone(),
        date: Some(selected.date.clone()),
        stale_business_days: target_business_days_between(&selected.date, as_of)?,
    })
}

fn target_business_days_between(from: &str, through: &str) -> Result<u32, String> {
    let from = parse_date(from)?;
    let through = parse_date(through)?;
    if through <= from {
        return Ok(0);
    }
    let mut count = 0u32;
    let mut date = from + 1;
    while date <= through {
        if is_target_operating_day(date)? {
            count += 1;
        }
        date += 1;
    }
    Ok(count)
}

fn is_target_operating_day(days_since_epoch: i64) -> Result<bool, String> {
    if weekday(days_since_epoch) >= 5 {
        return Ok(false);
    }
    let (year, month, day) = civil_date_from_days(days_since_epoch);
    if (month == 1 && day == 1)
        || (month == 5 && day == 1)
        || (month == 12 && matches!(day, 25 | 26))
    {
        return Ok(false);
    }
    let (easter_month, easter_day) = gregorian_easter(year);
    let easter_sunday = parse_date(&format!("{year:04}-{easter_month:02}-{easter_day:02}"))?;
    Ok(days_since_epoch != easter_sunday - 2 && days_since_epoch != easter_sunday + 1)
}

fn civil_date_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_epoch + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn gregorian_easter(year: i64) -> (i64, i64) {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let value = h + l - 7 * m + 114;
    (value / 31, value % 31 + 1)
}

fn parse_date(value: &str) -> Result<i64, String> {
    let parts = value
        .split('-')
        .map(|part| part.parse::<i64>().map_err(|_| "invalid date"))
        .collect::<Result<Vec<_>, _>>()?;
    if parts.len() != 3 || parts[1] < 1 || parts[1] > 12 {
        return Err("invalid date".into());
    }
    let (mut year, month) = (parts[0], parts[1]);
    let day = parts[2];
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day < 1 || day > max_day {
        return Err("invalid date".into());
    }
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Ok(era * 146_097 + day_of_era - 719_468)
}

fn weekday(days_since_epoch: i64) -> i64 {
    (days_since_epoch + 3).rem_euclid(7)
}

pub(crate) fn calculate_valuation(
    amount: &str,
    input_currency: &str,
    from_account_currency: &str,
    to_amount: &str,
    to_account_currency: &str,
    as_of: &str,
    observations: &[RateObservation],
    rounding: RoundingMode,
) -> Result<TransactionValuation, String> {
    let amount = Decimal::parse(amount)?;
    let to_amount = Decimal::parse(to_amount)?;
    let input_rate = select_prior_rate(input_currency, as_of, observations)?;
    let from_account_rate = select_prior_rate(from_account_currency, as_of, observations)?;
    let to_account_rate = select_prior_rate(to_account_currency, as_of, observations)?;
    let source_adjustment = if input_currency == "EUR" {
        Decimal::parse("1")?
    } else {
        Decimal::parse(SOURCE_FX_ADJUSTMENT)?
    };

    let input_quote = Decimal::parse(&input_rate.quote)?.as_ratio();
    let from_account_quote = Decimal::parse(&from_account_rate.quote)?.as_ratio();
    let to_account_quote = Decimal::parse(&to_account_rate.quote)?.as_ratio();
    let source_amount = amount.as_ratio();
    let adjustment = source_adjustment.as_ratio();

    let from_eur_rate = adjustment.divide(&input_quote)?;
    let from_native_rate = if input_currency == from_account_currency {
        Ratio {
            numerator: BigInt::from(1u8),
            denominator: BigInt::from(1u8),
        }
    } else {
        adjustment
            .multiply(&from_account_quote)
            .divide(&input_quote)?
    };
    let from_eur = source_amount
        .multiply(&from_eur_rate)
        .round_decimal(2, rounding)
        .format();
    let from_native = source_amount
        .multiply(&from_native_rate)
        .round_decimal(2, rounding)
        .format();

    let to_eur_rate = Ratio {
        numerator: BigInt::from(1u8),
        denominator: BigInt::from(1u8),
    }
    .divide(&to_account_quote)?;
    let to_eur = to_amount
        .as_ratio()
        .multiply(&to_eur_rate)
        .round_decimal(2, rounding)
        .format();
    let to_native = to_amount.round_decimal(2, rounding).format();

    let applied_rate_dates = [
        (input_currency.to_string(), input_rate.date.clone()),
        (
            from_account_currency.to_string(),
            from_account_rate.date.clone(),
        ),
        (
            to_account_currency.to_string(),
            to_account_rate.date.clone(),
        ),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    let stale = [
        input_rate.stale_business_days,
        from_account_rate.stale_business_days,
        to_account_rate.stale_business_days,
    ]
    .into_iter()
    .any(|days| days > 3);

    Ok(TransactionValuation {
        from: SideValuation {
            amount_native: from_native,
            amount_eur: from_eur,
            exchange_to_eur_numerator: from_eur_rate.numerator.to_string(),
            exchange_to_eur_denominator: from_eur_rate.denominator.to_string(),
            exchange_to_native_numerator: from_native_rate.numerator.to_string(),
            exchange_to_native_denominator: from_native_rate.denominator.to_string(),
            applied_rate_dates: applied_rate_dates.clone(),
            stale,
        },
        to: SideValuation {
            amount_native: to_native,
            amount_eur: to_eur,
            exchange_to_eur_numerator: to_eur_rate.numerator.to_string(),
            exchange_to_eur_denominator: to_eur_rate.denominator.to_string(),
            exchange_to_native_numerator: "1".into(),
            exchange_to_native_denominator: "1".into(),
            applied_rate_dates,
            stale,
        },
    })
}

#[derive(Clone, Debug)]
pub(crate) struct TransactionEffect {
    pub from_account_id: Option<String>,
    pub to_account_id: Option<String>,
    pub from_category_id: Option<String>,
    pub to_category_id: Option<String>,
    pub kind: String,
    pub amount: String,
    pub valuation: TransactionValuation,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Projection {
    pub account_deltas: BTreeMap<String, String>,
    /// Net expense increases the selected category; its negative changes
    /// remaining balance by the inverse amount.
    pub category_expense_deltas: BTreeMap<String, String>,
    pub reallocation_impacts: BTreeMap<String, String>,
}

pub(crate) fn project_transaction(effect: &TransactionEffect) -> Result<Projection, String> {
    let mut projection = Projection::default();
    if let Some(account_id) = &effect.from_account_id {
        add_delta(
            &mut projection.account_deltas,
            account_id,
            &Decimal::parse(&effect.valuation.from.amount_native)?.negated(),
        );
    }
    if let Some(account_id) = &effect.to_account_id {
        add_delta(
            &mut projection.account_deltas,
            account_id,
            &Decimal::parse(&effect.valuation.to.amount_native)?,
        );
    }
    if let Some(category_id) = &effect.from_category_id {
        add_delta(
            &mut projection.category_expense_deltas,
            category_id,
            &Decimal::parse(&effect.valuation.from.amount_eur)?,
        );
    }
    if let Some(category_id) = &effect.to_category_id {
        add_delta(
            &mut projection.category_expense_deltas,
            category_id,
            &Decimal::parse(&effect.valuation.to.amount_eur)?.negated(),
        );
    }
    if effect.kind == "reallocate" {
        let amount = Decimal::parse(&effect.amount)?;
        let absolute = if amount.coefficient < BigInt::from(0u8) {
            amount.negated()
        } else {
            amount
        };
        if let Some(category_id) = &effect.from_category_id {
            add_delta(&mut projection.reallocation_impacts, category_id, &absolute);
        }
        if let Some(category_id) = &effect.to_category_id {
            add_delta(
                &mut projection.reallocation_impacts,
                category_id,
                &absolute.negated(),
            );
        }
    }
    Ok(projection)
}

fn add_delta(map: &mut BTreeMap<String, String>, key: &str, delta: &Decimal) {
    let current = map
        .get(key)
        .and_then(|value| Decimal::parse(value).ok())
        .unwrap_or_else(|| Decimal::parse("0").expect("zero is valid"));
    map.insert(key.to_string(), current.add(delta).format());
}

pub(crate) fn project_open_budget(
    current_expense: &str,
    current_remaining: &str,
    expense_delta: &str,
    is_month_open: bool,
) -> Result<(String, String), String> {
    let current_expense = Decimal::parse(current_expense)?;
    let current_remaining = Decimal::parse(current_remaining)?;
    if !is_month_open {
        return Ok((current_expense.format(), current_remaining.format()));
    }
    let delta = Decimal::parse(expense_delta)?;
    Ok((
        current_expense.add(&delta).format(),
        current_remaining.add(&delta.negated()).format(),
    ))
}
