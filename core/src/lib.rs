//! YYaccountant core: double-entry books for households and sole proprietors with
//! receivables/payables, budgets, plans, cost management and forecasting.
//! All arithmetic uses decimal numbers (`rust_decimal`).
pub mod budget;
pub mod change;
pub mod chart;
pub mod claims;
pub mod cost;
pub mod forecast;
pub mod model;
pub mod num;
pub mod plans;
pub mod reports;
mod store;

pub use change::{Change, QuickEntry, QuickKind, Settlement, Target};
pub use model::*;
pub use store::{Applied, BookInfo, NewBook, Store};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("他の利用者が先に更新しました。再読み込みしてからやり直してください")]
    Conflict,
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
