//! PostgreSQL-only re-exports of official SQLx types and functions.
//!
//! The general SQLx facade resolves unused MySQL/SQLite packages into Cargo.lock.
//! Keeping only the driver we use removes their dependencies without replacing
//! query execution, pooling, transactions, TLS or database types.
pub use sqlx_core::error::Error;
pub use sqlx_core::query::query;
pub use sqlx_core::query_scalar::query_scalar;
pub use sqlx_core::raw_sql::raw_sql;
pub use sqlx_core::row::Row;
pub use sqlx_core::transaction::Transaction;
pub use sqlx_postgres as postgres;
pub use sqlx_postgres::{PgPool, Postgres};
