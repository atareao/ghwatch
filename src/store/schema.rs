//! Esquema de la base de datos y su aplicación.
//!
//! Las migraciones se **embeben** en el binario con `include_str!` y se aplican
//! al abrir el almacén: no hace falta `DATABASE_URL` en tiempo de compilación ni
//! ningún paso manual. El seguimiento de las migraciones ya aplicadas lo lleva
//! sqlx en su propia tabla (`_sqlx_migrations`), con una suma de comprobación por
//! migración.
//!
//! Para añadir una migración basta con crear `migrations/000N_descripcion.sql`
//! y registrarla aquí con el siguiente número de versión.

use std::borrow::Cow;

use sqlx::SqlSafeStr;
use sqlx::migrate::{Migration, MigrationType, Migrator};

/// Migración inicial: inventario, ejecuciones, alertas y eventos.
const INICIAL: &str = include_str!("../../migrations/0001_initial.sql");

/// Construye el conjunto de migraciones embebido, en orden de versión.
#[must_use]
pub fn migrator() -> Migrator {
    Migrator::with_migrations(vec![Migration::new(
        1,
        Cow::Borrowed("esquema inicial"),
        MigrationType::Simple,
        INICIAL.into_sql_str(),
        false,
    )])
}
