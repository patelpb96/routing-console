//! Seed data of contract section 7. This slice seeds the contributors; the six tasks come later.

use rusqlite::Connection;

use crate::domain::Area::{self, *};
use crate::store::write::insert_contributor;

/// Invented people. Nobody has `veterinary`, so a veterinary task can never be matched.
pub const CONTRIBUTORS: [(&str, &[Area], i64); 8] = [
    ("Ada Brook", &[Finance, DataAnalysis], 2),
    ("Ben Okafor", &[Healthcare], 1),
    ("Clara Voss", &[Healthcare, Operations], 2),
    ("Dev Raman", &[DataAnalysis], 2),
    ("Elena Ruiz", &[Finance], 1),
    ("Farid Haddad", &[Operations, DataAnalysis, Finance], 2),
    ("Grace Lin", &[Healthcare, DataAnalysis], 1),
    ("Hugo Marsh", &[Operations], 1),
];

/// Seeds an empty database; leaves one that already has contributors alone.
pub fn contributors_if_empty(conn: &Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM contributors", [], |r| r.get(0))?;
    if count == 0 {
        for (name, areas, capacity) in CONTRIBUTORS {
            insert_contributor(conn, name, areas, capacity)?;
        }
    }
    Ok(())
}
