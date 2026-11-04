//! Instantáneas de ejecución del escaneo.
//!
//! Cada ejecución queda registrada como una fila de `scan_runs`, comparable con
//! las demás: de ahí salen las series del histórico y el diff entre ejecuciones.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Instantánea de una ejecución del escaneo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanRun {
    /// Identificador de la ejecución.
    pub id: i64,
    /// Instante en que arrancó el escaneo.
    pub started_at: DateTime<Utc>,
    /// Instante en que terminó el escaneo.
    pub finished_at: DateTime<Utc>,
    /// Repositorios considerados (correctos más fallidos).
    pub total_repos: i64,
    /// Repositorios recolectados con éxito.
    pub ok: i64,
    /// Repositorios que fallaron.
    pub failed: i64,
    /// Alertas presentes al terminar la ejecución.
    pub alerts_total: i64,
    /// Versión del esquema de datos con el que se escribió la fila.
    pub schema_version: i64,
}
