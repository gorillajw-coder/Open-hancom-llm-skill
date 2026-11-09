//! oh-recipe: document card engine. Pure bytes in, bytes out; the host reads/writes files.
mod bind;
mod card;
mod engine;
mod error;
mod expr;
mod regulation;
pub mod xlsx;

pub use bind::{bind, hex_sha256, CORE_VERSION, BoundCard, BoundNode, BoundTarget};
pub use card::{Frame, ProjectMap, Anchor, Calc, Card, CellTarget, RowsTarget, Extract, Carry, XlsxOut, XlsxCell, Field, DefaultEntry, Defaults, Kind, Rule, Severity, Slot, Style, Target};
pub use engine::*;
pub use error::{Error, Result};
pub use expr::{parse_date, parse_decimal};
pub use regulation::Regulation;
