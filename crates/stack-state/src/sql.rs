//! The SQL the shared tables are read and written with, once for both databases: SQLite (the
//! local `state.db`) and, behind the `postgres` feature, a team's Postgres database.
//!
//! Statements use `$1`-style placeholders and stay within what both dialects take. A
//! `FOR UPDATE` suffix locks the selected rows on Postgres; SQLite drops it, since its write
//! transactions already hold the database's single write lock.

use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::{Transaction, params_from_iter};

use crate::error::{Error, Result};

/// A value bound to a placeholder, or read back from a column.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    /// A `NULL` text.
    Null,
    /// A `NULL` integer.
    NullInt,
    /// An integer.
    Int(i64),
    /// A boolean: an integer on SQLite, a `BOOLEAN` on Postgres.
    Bool(bool),
    /// A text.
    Text(String),
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_owned())
    }
}

impl From<&String> for Value {
    fn from(v: &String) -> Self {
        Value::Text(v.clone())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

impl From<Option<String>> for Value {
    fn from(v: Option<String>) -> Self {
        v.map_or(Value::Null, Value::Text)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Value::Int(v.into())
    }
}

impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Value::Int(v as i64)
    }
}

impl From<Option<u32>> for Value {
    fn from(v: Option<u32>) -> Self {
        v.map_or(Value::NullInt, Value::from)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

/// The placeholder values of a statement, in order.
macro_rules! args {
    ($($v:expr),* $(,)?) => {
        vec![$($crate::sql::Value::from($v)),*]
    };
}
pub(crate) use args;

/// A row read back: its columns in `SELECT` order.
#[derive(Debug)]
pub(crate) struct Row(pub(crate) Vec<Value>);

impl Row {
    fn get(&self, i: usize) -> Result<&Value> {
        self.0.get(i).ok_or_else(|| column_error(i, "missing"))
    }

    pub(crate) fn text(&self, i: usize) -> Result<String> {
        match self.get(i)? {
            Value::Text(s) => Ok(s.clone()),
            other => Err(column_error(i, &format!("expected text, got {other:?}"))),
        }
    }

    pub(crate) fn opt_text(&self, i: usize) -> Result<Option<String>> {
        match self.get(i)? {
            Value::Null | Value::NullInt => Ok(None),
            _ => self.text(i).map(Some),
        }
    }

    pub(crate) fn int(&self, i: usize) -> Result<i64> {
        match self.get(i)? {
            Value::Int(n) => Ok(*n),
            Value::Bool(b) => Ok(i64::from(*b)),
            other => Err(column_error(
                i,
                &format!("expected an integer, got {other:?}"),
            )),
        }
    }

    pub(crate) fn opt_int(&self, i: usize) -> Result<Option<i64>> {
        match self.get(i)? {
            Value::Null | Value::NullInt => Ok(None),
            _ => self.int(i).map(Some),
        }
    }

    pub(crate) fn u32(&self, i: usize) -> Result<u32> {
        let n = self.int(i)?;
        u32::try_from(n).map_err(|_| column_error(i, &format!("{n} is out of range")))
    }

    pub(crate) fn opt_u32(&self, i: usize) -> Result<Option<u32>> {
        match self.opt_int(i)? {
            None => Ok(None),
            Some(n) => u32::try_from(n)
                .map(Some)
                .map_err(|_| column_error(i, &format!("{n} is out of range"))),
        }
    }

    pub(crate) fn u64(&self, i: usize) -> Result<u64> {
        let n = self.int(i)?;
        u64::try_from(n).map_err(|_| column_error(i, &format!("{n} is out of range")))
    }

    pub(crate) fn bool(&self, i: usize) -> Result<bool> {
        match self.get(i)? {
            Value::Bool(b) => Ok(*b),
            _ => Ok(self.int(i)? != 0),
        }
    }
}

fn column_error(i: usize, what: &str) -> Error {
    Error::Column(format!("column {i}: {what}"))
}

/// Why a statement failed, as far as the store's callers care.
#[derive(Debug)]
pub(crate) enum DbError {
    /// A `UNIQUE` or primary-key constraint refused it.
    Unique(Error),
    /// A foreign key refused it.
    ForeignKey(Error),
    /// Anything else.
    Other(Error),
}

impl From<DbError> for Error {
    fn from(e: DbError) -> Self {
        match e {
            DbError::Unique(e) | DbError::ForeignKey(e) | DbError::Other(e) => e,
        }
    }
}

impl From<Error> for DbError {
    fn from(e: Error) -> Self {
        DbError::Other(e)
    }
}

/// A connection or transaction on one of the two databases.
pub(crate) trait Sql {
    /// Run `sql`, returning how many rows it changed.
    fn execute(&mut self, sql: &str, args: Vec<Value>) -> Result<u64, DbError>;

    /// Run `sql`, returning the rows it selects.
    fn query(&mut self, sql: &str, args: Vec<Value>) -> Result<Vec<Row>, DbError>;

    /// The first row `sql` selects, if any.
    fn query_opt(&mut self, sql: &str, args: Vec<Value>) -> Result<Option<Row>> {
        Ok(self.query(sql, args)?.into_iter().next())
    }
}

/// `sql` in SQLite's spelling: `?1` placeholders, no row locks.
fn sqlite_sql(sql: &str) -> String {
    sql.replace(" FOR UPDATE", "").replace('$', "?")
}

impl rusqlite::ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            Value::Null | Value::NullInt => ToSqlOutput::Borrowed(ValueRef::Null),
            Value::Int(n) => ToSqlOutput::from(*n),
            Value::Bool(b) => ToSqlOutput::from(*b),
            Value::Text(s) => ToSqlOutput::Borrowed(ValueRef::Text(s.as_bytes())),
        })
    }
}

fn sqlite_error(e: rusqlite::Error) -> DbError {
    match &e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
        {
            DbError::ForeignKey(e.into())
        }
        rusqlite::Error::SqliteFailure(f, _)
            if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                || f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY =>
        {
            DbError::Unique(e.into())
        }
        _ => DbError::Other(e.into()),
    }
}

/// [`Sql`] over a SQLite connection or transaction.
pub(crate) struct Sqlite<'a>(pub(crate) &'a rusqlite::Connection);

impl<'a> Sqlite<'a> {
    pub(crate) fn tx(tx: &'a Transaction<'_>) -> Self {
        Sqlite(tx)
    }
}

impl Sql for Sqlite<'_> {
    fn execute(&mut self, sql: &str, args: Vec<Value>) -> Result<u64, DbError> {
        let mut stmt = self
            .0
            .prepare_cached(&sqlite_sql(sql))
            .map_err(sqlite_error)?;
        let n = stmt.execute(params_from_iter(args)).map_err(sqlite_error)?;
        Ok(n as u64)
    }

    fn query(&mut self, sql: &str, args: Vec<Value>) -> Result<Vec<Row>, DbError> {
        let mut stmt = self
            .0
            .prepare_cached(&sqlite_sql(sql))
            .map_err(sqlite_error)?;
        let columns = stmt.column_count();
        let rows = stmt
            .query_map(params_from_iter(args), |r| {
                (0..columns)
                    .map(|i| {
                        Ok(match r.get_ref(i)? {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => Value::Int(n),
                            ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into()),
                            ValueRef::Real(f) => Value::Text(f.to_string()),
                            ValueRef::Blob(b) => Value::Text(String::from_utf8_lossy(b).into()),
                        })
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map(Row)
            })
            .map_err(sqlite_error)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sqlite_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_spelling_drops_locks_and_renumbers_placeholders() {
        assert_eq!(
            sqlite_sql("SELECT a FROM t WHERE id = $1 AND b = $2 FOR UPDATE"),
            "SELECT a FROM t WHERE id = ?1 AND b = ?2"
        );
    }

    #[test]
    fn rows_decode_and_check_ranges() {
        let row = Row(vec![
            Value::Int(-1),
            Value::Bool(true),
            Value::Null,
            Value::Text("x".into()),
        ]);
        assert!(row.u32(0).is_err());
        assert!(row.bool(1).unwrap());
        assert_eq!(row.int(1).unwrap(), 1);
        assert_eq!(row.opt_text(2).unwrap(), None);
        assert_eq!(row.opt_u32(2).unwrap(), None);
        assert_eq!(row.text(3).unwrap(), "x");
        assert!(row.int(3).is_err());
        assert!(row.text(9).is_err());
    }
}
