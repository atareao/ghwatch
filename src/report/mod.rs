//! Informes del escaneo: consola, Markdown y JSON.
//!
//! Este módulo presenta el estado actual y la evolución de las vulnerabilidades.
//! Se apoya solo en tipos puros ([`crate::scan`], [`crate::domain`] y
//! [`crate::store`]): **no** consulta la red ni la base de datos, de modo que se
//! puede generar el informe a partir de datos ya recolectados.
//!
//! Las piezas son:
//!
//! - [`render_console`]: la tabla de consola, agrupada por severidad y ordenada
//!   por prioridad, con color por severidad.
//! - [`ReportDocument`]: el modelo estable y versionado del informe, del que
//!   derivan el Markdown ([`ReportDocument::to_markdown`]) y el JSON
//!   ([`ReportDocument::to_json`]).
//! - [`write_reports`]: la escritura del informe en disco.
//!
//! # Independencia de la notificación
//!
//! La escritura del informe nunca depende del resultado de la notificación: se
//! escribe primero y solo después se intenta notificar. La coordinación con
//! Valet vive en [`crate::notify`] (dirección de dependencia `notify → report`),
//! de modo que este módulo no conoce la red. Un Valet caído deja los informes
//! completos y no altera el resultado del escaneo.

pub mod console;
pub mod document;
mod emit;
mod markdown;

pub use console::{render_console, severity_color};
pub use document::{
    Action, EcosystemCount, HISTORY_EN_INFORME, HistoryEntry, RepoCount, ReportDocument,
    SCHEMA_VERSION, SeverityCounts,
};
pub use emit::{ReportPaths, write_reports};

/// Error de la generación del informe.
///
/// Ninguna variante incluye datos sensibles: los informes no transportan
/// credenciales.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// No se pudo escribir el informe en disco.
    #[error("no se pudo escribir el informe {path:?}: {source}")]
    Write {
        /// Ruta del informe que no se pudo escribir.
        path: std::path::PathBuf,
        /// Causa del fallo de escritura.
        #[source]
        source: std::io::Error,
    },
    /// No se pudo serializar el informe a JSON.
    #[error("no se pudo serializar el informe JSON: {0}")]
    Serialize(#[from] serde_json::Error),
}
