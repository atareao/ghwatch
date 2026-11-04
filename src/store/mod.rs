//! Persistencia del histórico en SQLite.
//!
//! Este módulo guarda el resultado de cada escaneo y calcula su **deriva**
//! respecto a la ejecución anterior. Se apoya en [`sqlx`] con consultas en
//! runtime y sin `DATABASE_URL` en compilación: el esquema se aplica al abrir el
//! almacén ([`Store::connect`] o [`Store::in_memory`]).
//!
//! # Idempotencia
//!
//! La tabla `alerts` declara la clave única `(source, repo, external_id)`, de
//! modo que repetir un escaneo idéntico no duplica ni altera las alertas
//! existentes: solo avanza su `last_seen_run`.
//!
//! # Ciclo de vida
//!
//! Cada alerta registra `first_seen_run`, `last_seen_run` y `resolved_at`, y
//! cada cambio queda en `alert_events` con su categoría de
//! [`Drift`](crate::domain::Drift).
//!
//! La dirección de dependencias es `store` → `domain`: el dominio sigue puro.

mod persist;
mod query;
mod schema;

use std::str::FromStr;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

pub use persist::PersistOutcome;
pub use query::{Diff, HistoryPoint, StoredAlert, StoredEvent};

/// Versión del esquema de datos.
///
/// Se escribe en cada fila de `scan_runs` para que un histórico viejo pueda
/// reconocerse y migrarse.
pub const SCHEMA_VERSION: i64 = 1;

/// Error de la capa de persistencia.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Error de la base de datos.
    #[error("error de base de datos: {0}")]
    Database(#[from] sqlx::Error),
    /// Error al aplicar las migraciones.
    #[error("error al aplicar las migraciones: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// Un valor almacenado no pertenece al conjunto cerrado esperado.
    #[error("valor inesperado en la base de datos: {0}")]
    Corrupt(String),
}

/// Almacén de datos sobre SQLite.
pub struct Store {
    /// Pool de conexiones, con las migraciones ya aplicadas.
    pool: SqlitePool,
}

impl Store {
    /// Abre el almacén en `url` y aplica las migraciones pendientes.
    ///
    /// `url` sigue el formato de sqlx (`sqlite://ruta/fichero.db`); el fichero
    /// se crea si no existe.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si la URL no es válida, si no se puede abrir la
    /// base de datos o si falla una migración.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str(url)?.create_if_missing(true);
        let pool = SqlitePoolOptions::new().connect_with(options).await?;
        Self::from_pool(pool).await
    }

    /// Abre un almacén **en memoria**, aislado y sin tocar el disco.
    ///
    /// Se limita a una sola conexión: una base de datos `:memory:` vive mientras
    /// viva su conexión, así que el pool debe reutilizarla en lugar de abrir
    /// otras (cada conexión nueva vería una base de datos vacía y distinta).
    /// Es el almacén que usan los tests.
    ///
    /// # Errores
    ///
    /// Devuelve [`StoreError`] si falla la apertura o una migración.
    pub async fn in_memory() -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(options)
            .await?;
        Self::from_pool(pool).await
    }

    /// Aplica las migraciones y envuelve el pool ya listo.
    async fn from_pool(pool: SqlitePool) -> Result<Self, StoreError> {
        schema::migrator().run(&pool).await?;
        Ok(Self { pool })
    }

    /// Devuelve el pool subyacente.
    ///
    /// Útil para consultas de diagnóstico y para los tests, que comprueban el
    /// esquema directamente.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
