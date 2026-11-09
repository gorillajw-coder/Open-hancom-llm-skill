//! Per-diem regulation table (overseas travel): rank group x grade -> USD rates, country ->
//! grade, duration discount bands. Pure data + lookups; the host decides which file to load
//! (shipped sample in the library, or a company table kept outside the library).
use crate::error::{Error, Result};
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Regulation {
    pub id: String,
    pub title: String,
    /// `sample` (anonymized, shipped) or `company` (the user's real table).
    pub status: String,
    pub unit: String,
    #[serde(default)]
    pub note: String,
    #[serde(rename = "group")]
    pub groups: Vec<Group>,
    #[serde(rename = "rate")]
    pub rates: Vec<Rate>,
    #[serde(rename = "grade")]
    pub grades: Vec<Grade>,
    #[serde(default, rename = "alias")]
    pub aliases: Vec<Alias>,
    /// Optional duration bands (none in the 2023.08.01 table; dispatch lodging is a card input).
    #[serde(default, rename = "discount")]
    pub discounts: Vec<Discount>,
    /// Cities graded on their own: the country they belong to and other spellings.
    #[serde(default, rename = "city")]
    pub cities: Vec<City>,
    /// `verified` only when the bands come from the written regulation; empty when no bands.
    #[serde(default)]
    pub discount_status: String,
    #[serde(default)]
    pub discount_basis: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: String,
    pub label: String,
    pub titles: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rate {
    pub group: String,
    pub grade: String,
    #[serde(deserialize_with = "de_dec")]
    pub lodging: Decimal,
    #[serde(deserialize_with = "de_dec")]
    pub meal: Decimal,
    #[serde(deserialize_with = "de_dec")]
    pub daily: Decimal,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grade {
    pub id: String,
    /// Countries and cities listed under this grade.
    pub places: Vec<String>,
}
/// A place missing from the table, mapped by a stated basis (shown as a warning).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Alias {
    pub place: String,
    pub grade: String,
    pub basis: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct City {
    pub name: String,
    pub country: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discount {
    /// Band applies from this many total nights (inclusive).
    pub min_nights: u32,
    #[serde(deserialize_with = "de_dec")]
    pub rate: Decimal,
}

/// TOML integer or float -> exact Decimal via its shortest text form (0.85 stays 0.85).
fn de_dec<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Decimal, D::Error> {
    let v = toml::Value::deserialize(d)?;
    let s = match v {
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => format!("{f}"),
        _ => return Err(serde::de::Error::custom("number expected")),
    };
    s.parse().map_err(serde::de::Error::custom)
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

impl Regulation {
    pub fn from_toml(src: &str) -> Result<Regulation> {
        let r: Regulation = toml::from_str(src).map_err(|e| Error::new("invalid_regulation", e.to_string()))?;
        if !["sample", "company"].contains(&r.status.as_str()) || !["", "verified", "unverified"].contains(&r.discount_status.as_str()) {
            return Err(Error::new("invalid_regulation", "status: sample|company, discount_status: verified|unverified"));
        }
        if !r.discounts.is_empty() && r.discounts.iter().all(|d| d.min_nights > 1) {
            return Err(Error::new("invalid_regulation", "discount bands must start at 1 night or less"));
        }
        for x in &r.rates {
            if !r.groups.iter().any(|g| g.id == x.group) || !r.grades.iter().any(|g| g.id == x.grade) {
                return Err(Error::new("invalid_regulation", format!("rate {} / {}", x.group, x.grade)));
            }
        }
        for a in &r.aliases {
            if !r.grades.iter().any(|g| g.id == a.grade) {
                return Err(Error::new("invalid_regulation", format!("alias {}", a.place)));
            }
        }
        Ok(r)
    }
    /// (grade, basis) — basis is `listed` or the alias basis.
    pub fn grade(&self, place: &str) -> Option<(String, String)> {
        let p = norm(place);
        if p.is_empty() {
            return None;
        }
        if let Some(g) = self.grades.iter().find(|g| g.places.iter().any(|x| norm(x) == p)) {
            return Some((g.id.clone(), "listed".into()));
        }
        self.aliases.iter().find(|a| norm(&a.place) == p).map(|a| (a.grade.clone(), format!("미등재: {}", a.basis)))
    }
    /// Table spelling of a city (aliases such as 도쿄 → 동경).
    pub fn city_name(&self, city: &str) -> String {
        let c = norm(city);
        self.cities.iter().find(|x| norm(&x.name) == c || x.aliases.iter().any(|a| norm(a) == c)).map(|x| x.name.clone()).unwrap_or_else(|| city.to_string())
    }
    /// The city's own grade when the table lists it and it belongs to `country`.
    pub fn city_grade(&self, country: &str, city: &str) -> Option<String> {
        let name = self.city_name(city);
        let (g, b) = self.grade(&name)?;
        let info = self.cities.iter().find(|x| x.name == name)?;
        (b == "listed" && norm(&info.country) == norm(country)).then_some(g)
    }
    /// A city listed with a grade but under a different country (likely a wrong city/country).
    pub fn city_mismatch(&self, country: &str, city: &str) -> Option<String> {
        let name = self.city_name(city);
        let info = self.cities.iter().find(|x| x.name == name)?;
        (norm(&info.country) != norm(country)).then(|| format!("{city}은(는) {} 도시라 {country} 등급으로 계산", info.country))
    }
    /// A segment's grade: the city when the table grades it and it belongs to the country,
    /// else the country (else an alias of the city, e.g. 누크).
    pub fn place_grade(&self, country: &str, city: &str) -> Option<(String, String)> {
        if let Some(g) = self.city_grade(country, city) {
            return Some((g, "listed".into()));
        }
        self.grade(country).or_else(|| self.grade(city))
    }
    pub fn group_of(&self, title: &str) -> Option<&Group> {
        let t = norm(title);
        self.groups.iter().find(|g| g.titles.iter().any(|x| norm(x) == t))
    }
    pub fn rate(&self, kind: &str, grade: &str, title: &str) -> Option<Decimal> {
        let g = self.group_of(title)?;
        let r = self.rates.iter().find(|r| r.group == g.id && r.grade == grade)?;
        match kind {
            "lodging" => Some(r.lodging),
            "meal" => Some(r.meal),
            "daily" => Some(r.daily),
            _ => None,
        }
    }
    pub fn discount(&self, nights: u32) -> Decimal {
        self.discounts.iter().filter(|d| d.min_nights <= nights).max_by_key(|d| d.min_nights).map(|d| d.rate).unwrap_or(Decimal::ONE)
    }
}
