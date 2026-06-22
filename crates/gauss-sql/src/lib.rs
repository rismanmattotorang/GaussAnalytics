//! SQL runner implementations for GaussAnalytics. Each implements
//! [`gauss_engine::traits::SqlRunner`] and is feature-gated:
//!
//! - `sqlite` (default) → [`SqliteRunner`]
//! - `postgres` → [`PostgresRunner`]
//! - `snowflake` → [`SnowflakeRunner`]

#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteRunner;

#[cfg(feature = "sqlite")]
mod csv_ingest;
#[cfg(feature = "sqlite")]
pub use csv_ingest::{ingest_csv, CsvColumn, CsvIngestSummary};

#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PostgresRunner;

#[cfg(feature = "snowflake")]
mod snowflake;
#[cfg(feature = "snowflake")]
pub use snowflake::{SnowflakeContext, SnowflakeRunner};
