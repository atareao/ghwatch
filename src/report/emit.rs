//! Escritura del informe en disco.
//!
//! Esta es la última etapa del módulo de informes: convierte el
//! [`ReportDocument`] en Markdown y JSON y los persiste. La coordinación con la
//! notificación best-effort vive en [`crate::notify`], de modo que este módulo
//! **no** conoce la red: se limita a renderizar y escribir.

use std::path::{Path, PathBuf};

use super::{ReportDocument, ReportError};

/// Rutas de los informes escritos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPaths {
    /// Ruta del informe Markdown.
    pub markdown: PathBuf,
    /// Ruta del informe JSON.
    pub json: PathBuf,
}

/// Escribe el informe Markdown y el JSON en disco.
///
/// La escritura es síncrona; quien la invoque desde un contexto asíncrono debe
/// hacerlo fuera del hilo del runtime (por ejemplo con
/// [`tokio::task::spawn_blocking`]), como hace `notify::emit_and_notify`.
///
/// # Errores
///
/// Devuelve [`ReportError`] si no se puede serializar el JSON o escribir alguno
/// de los dos ficheros.
pub fn write_reports(
    document: &ReportDocument,
    markdown: &Path,
    json: &Path,
) -> Result<ReportPaths, ReportError> {
    escribir(markdown, document.to_markdown().as_bytes())?;
    escribir(json, document.to_json()?.as_bytes())?;
    Ok(ReportPaths {
        markdown: markdown.to_path_buf(),
        json: json.to_path_buf(),
    })
}

/// Escribe bytes en un fichero, tipando el error con su ruta.
fn escribir(path: &Path, contenido: &[u8]) -> Result<(), ReportError> {
    std::fs::write(path, contenido).map_err(|source| ReportError::Write {
        path: path.to_path_buf(),
        source,
    })
}
