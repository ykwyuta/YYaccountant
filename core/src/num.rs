//! Decimal arithmetic. Amounts, quantities, rates and statistics never use binary floats.
//! JSON carries decimals as strings; integers are accepted on input, fractional JSON numbers are rejected.
use crate::{Result, invalid};
pub use rust_decimal::Decimal;
use rust_decimal::{MathematicalOps, RoundingStrategy};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::str::FromStr;

/// Absolute limit for stored amounts and quantities (10^15).
pub fn limit() -> Decimal {
    Decimal::from(1_000_000_000_000_000_i64)
}
pub fn hundred() -> Decimal {
    Decimal::ONE_HUNDRED
}

/// Parses user text. Accepts full-width digits, thousands separators and a leading sign; rejects exponents.
pub fn parse(text: &str) -> Result<Decimal> {
    let mut s = String::new();
    for c in text.trim().chars() {
        match c {
            '０'..='９' => s.push(char::from(b'0' + (c as u32 - '０' as u32) as u8)),
            '．' => s.push('.'),
            '－' | '−' | 'ー' => s.push('-'),
            '＋' => s.push('+'),
            ',' | '，' | '_' | ' ' | '¥' | '￥' | '円' => {}
            c => s.push(c),
        }
    }
    let s = s.strip_prefix('+').unwrap_or(&s);
    if s.is_empty()
        || !s
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
        || s.matches('.').count() > 1
        || s.rfind('-').is_some_and(|i| i != 0)
    {
        return Err(invalid(format!("数値を解釈できません: {text}")));
    }
    let d =
        Decimal::from_str_exact(s).map_err(|_| invalid(format!("数値が大きすぎます: {text}")))?;
    if d.abs() >= limit() {
        return Err(invalid("数値の絶対値は10^15未満にしてください"));
    }
    Ok(d.normalize())
}

pub fn text(d: Decimal) -> String {
    d.normalize().to_string()
}

/// Fixed number of fraction digits, for display and CSV.
pub fn fixed(d: Decimal, digits: u32) -> String {
    let mut r = d.round_dp_with_strategy(digits, RoundingStrategy::MidpointAwayFromZero);
    r.rescale(digits);
    r.to_string()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rounding {
    /// 四捨五入
    #[default]
    HalfUp,
    /// 切り捨て
    Down,
    /// 切り上げ
    Up,
    /// 銀行丸め（偶数丸め）
    HalfEven,
}
impl Rounding {
    pub const ALL: [Self; 4] = [Self::HalfUp, Self::Down, Self::Up, Self::HalfEven];
    pub fn label(self) -> &'static str {
        match self {
            Self::HalfUp => "四捨五入",
            Self::Down => "切り捨て",
            Self::Up => "切り上げ",
            Self::HalfEven => "銀行丸め",
        }
    }
    pub fn round(self, d: Decimal, digits: u32) -> Decimal {
        d.round_dp_with_strategy(
            digits,
            match self {
                Self::HalfUp => RoundingStrategy::MidpointAwayFromZero,
                Self::Down => RoundingStrategy::ToZero,
                Self::Up => RoundingStrategy::AwayFromZero,
                Self::HalfEven => RoundingStrategy::MidpointNearestEven,
            },
        )
        .normalize()
    }
}

/// Integer power used by loan formulas; stays decimal.
pub fn powi(base: Decimal, exponent: u32) -> Result<Decimal> {
    base.checked_powu(u64::from(exponent))
        .ok_or_else(|| invalid("計算が桁あふれしました"))
}
pub fn sqrt(d: Decimal) -> Decimal {
    d.sqrt().unwrap_or(Decimal::ZERO)
}
pub fn mean(values: &[Decimal]) -> Decimal {
    if values.is_empty() {
        Decimal::ZERO
    } else {
        values.iter().sum::<Decimal>() / Decimal::from(values.len())
    }
}
pub fn median(values: &[Decimal]) -> Decimal {
    let mut v = values.to_vec();
    v.sort();
    match v.len() {
        0 => Decimal::ZERO,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]) / Decimal::TWO,
    }
}
/// Sample standard deviation (n-1); zero for fewer than two values.
pub fn stdev(values: &[Decimal]) -> Decimal {
    if values.len() < 2 {
        return Decimal::ZERO;
    }
    let m = mean(values);
    let sum: Decimal = values.iter().map(|v| (*v - m) * (*v - m)).sum();
    sqrt(sum / Decimal::from(values.len() - 1))
}
/// Least squares line over x = 0..n-1, returning (intercept, slope).
pub fn regression(values: &[Decimal]) -> (Decimal, Decimal) {
    let n = values.len();
    if n < 2 {
        return (values.first().copied().unwrap_or_default(), Decimal::ZERO);
    }
    let xs: Vec<Decimal> = (0..n).map(Decimal::from).collect();
    let mx = mean(&xs);
    let my = mean(values);
    let mut sxy = Decimal::ZERO;
    let mut sxx = Decimal::ZERO;
    for (x, y) in xs.iter().zip(values) {
        sxy += (*x - mx) * (*y - my);
        sxx += (*x - mx) * (*x - mx);
    }
    let slope = if sxx.is_zero() {
        Decimal::ZERO
    } else {
        sxy / sxx
    };
    (my - slope * mx, slope)
}

fn from_visitor<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Decimal, D::Error> {
    struct V;
    impl de::Visitor<'_> for V {
        type Value = Decimal;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a decimal string or an integer")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Decimal, E> {
            parse(v).map_err(E::custom)
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Decimal, E> {
            parse(&v.to_string()).map_err(E::custom)
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Decimal, E> {
            parse(&v.to_string()).map_err(E::custom)
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<Decimal, E> {
            Err(E::custom(
                "小数はJSON数値ではなく文字列で指定してください（例: \"1234.5\"）",
            ))
        }
    }
    d.deserialize_any(V)
}
/// `#[serde(with = "num::dec")]`
pub mod dec {
    use super::*;
    pub fn serialize<S: Serializer>(d: &Decimal, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&text(*d))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Decimal, D::Error> {
        from_visitor(d)
    }
}
/// `#[serde(with = "num::opt")]`
pub mod opt {
    use super::*;
    pub fn serialize<S: Serializer>(
        d: &Option<Decimal>,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_str(&text(*d)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Option<Decimal>, D::Error> {
        #[derive(Deserialize)]
        struct W(#[serde(with = "super::dec")] Decimal);
        Ok(Option::<W>::deserialize(d)?.map(|w| w.0))
    }
}
/// `#[serde(with = "num::list")]`
pub mod list {
    use super::*;
    use serde::ser::SerializeSeq;
    pub fn serialize<S: Serializer>(v: &[Decimal], s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for d in v {
            seq.serialize_element(&text(*d))?;
        }
        seq.end()
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Vec<Decimal>, D::Error> {
        #[derive(Deserialize)]
        struct W(#[serde(with = "super::dec")] Decimal);
        Ok(Vec::<W>::deserialize(d)?.into_iter().map(|w| w.0).collect())
    }
}
/// Serializable decimal wrapper for report structures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct D(pub Decimal);
impl Serialize for D {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&text(self.0))
    }
}
impl<'de> Deserialize<'de> for D {
    fn deserialize<De: Deserializer<'de>>(d: De) -> std::result::Result<Self, De::Error> {
        from_visitor(d).map(D)
    }
}
impl From<Decimal> for D {
    fn from(d: Decimal) -> Self {
        D(d)
    }
}
impl FromStr for D {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Self> {
        parse(s).map(D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_accepts_japanese_input_and_rejects_floats_and_exponents() {
        assert_eq!(text(parse("１，２３４．５０円").unwrap()), "1234.5");
        assert_eq!(text(parse("-0.10").unwrap()), "-0.1");
        assert!(parse("1e3").is_err());
        assert!(parse("1-2").is_err());
        assert!(parse("1000000000000000").is_err());
        assert!(serde_json::from_str::<D>("0.1").is_err());
        assert_eq!(
            serde_json::from_str::<D>("\"0.1\"").unwrap().0,
            parse("0.1").unwrap()
        );
        assert_eq!(
            serde_json::from_str::<D>("12").unwrap().0,
            Decimal::from(12)
        );
    }
    #[test]
    fn decimal_sums_are_exact_and_rounding_is_selectable() {
        let tenth = parse("0.1").unwrap();
        assert_eq!(tenth + tenth + tenth, parse("0.3").unwrap());
        let v = parse("2.5").unwrap();
        assert_eq!(Rounding::HalfUp.round(v, 0), Decimal::from(3));
        assert_eq!(Rounding::HalfEven.round(v, 0), Decimal::from(2));
        assert_eq!(
            Rounding::Down.round(parse("-2.7").unwrap(), 0),
            Decimal::from(-2)
        );
        assert_eq!(
            Rounding::Up.round(parse("2.01").unwrap(), 0),
            Decimal::from(3)
        );
        assert_eq!(fixed(parse("12").unwrap(), 2), "12.00");
    }
    #[test]
    fn statistics_are_decimal() {
        let v: Vec<_> = ["2", "4", "4", "4", "5", "5", "7", "9"]
            .iter()
            .map(|s| parse(s).unwrap())
            .collect();
        assert_eq!(mean(&v), Decimal::from(5));
        assert_eq!(median(&v), parse("4.5").unwrap());
        let s = stdev(&v);
        assert!((s - parse("2.138089935").unwrap()).abs() < parse("0.000001").unwrap());
        let (a, b) = regression(&[Decimal::from(1), Decimal::from(3), Decimal::from(5)]);
        assert_eq!((a, b), (Decimal::ONE, Decimal::TWO));
    }
}
