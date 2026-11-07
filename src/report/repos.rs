//! Presentación del listado de repositorios con alertas abiertas.
//!
//! Este módulo es **presentación pura** sobre [`RepoAlertList`]: no consulta la
//! red ni la base de datos, solo compone las tres salidas (consola, Markdown y
//! JSON) a partir del listado ya agregado por el almacén. El mismo listado y el
//! mismo instante producen exactamente la misma salida en los tres formatos.
//!
//! [`ReposDocument`] es el modelo estable y versionado del listado; de él
//! derivan [`ReposDocument::to_markdown`], [`ReposDocument::to_json`] y
//! [`ReposDocument::to_console`]. El JSON es la fuente de verdad observable:
//! todas sus claves son estables y el documento declara su
//! [`REPOS_SCHEMA_VERSION`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::store::{RepoAlertCount, RepoAlertList};

/// Versión del esquema del documento del listado de repositorios.
///
/// Se declara en el JSON (`schema_version`) para que el formato pueda crecer de
/// forma compatible.
pub const REPOS_SCHEMA_VERSION: u32 = 1;

/// Desglose por severidad de las alertas abiertas de un repositorio.
///
/// Los cubos son `i64` —y no `u64` como en [`crate::report::SeverityCounts`]—
/// porque el almacén devuelve los recuentos de SQLite tal cual (`COUNT(*)` es
/// `i64`): así el documento los refleja **sin conversiones con `as`** ni
/// reescrituras de signo. La suma de los cinco cubos coincide con el recuento
/// total de alertas del repositorio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSeverityCounts {
    /// Alertas abiertas de severidad crítica.
    pub critical: i64,
    /// Alertas abiertas de severidad alta.
    pub high: i64,
    /// Alertas abiertas de severidad media.
    pub medium: i64,
    /// Alertas abiertas de severidad baja.
    pub low: i64,
    /// Alertas abiertas sin severidad conocida (incluye las de severidad nula).
    pub unknown: i64,
}

impl RepoSeverityCounts {
    /// Suma de los cinco cubos.
    #[must_use]
    pub fn total(&self) -> i64 {
        self.critical + self.high + self.medium + self.low + self.unknown
    }
}

impl From<&RepoAlertCount> for RepoSeverityCounts {
    fn from(recuento: &RepoAlertCount) -> Self {
        Self {
            critical: recuento.critical,
            high: recuento.high,
            medium: recuento.medium,
            low: recuento.low,
            unknown: recuento.unknown,
        }
    }
}

/// Una fila del listado: un repositorio con sus alertas abiertas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoEntry {
    /// Repositorio (`owner/name`).
    pub repo: String,
    /// Alertas abiertas del repositorio.
    pub alerts: i64,
    /// Desglose de esas alertas por severidad.
    pub by_severity: RepoSeverityCounts,
}

/// Documento del listado de repositorios.
///
/// Todas las claves del JSON derivan de estos campos; el orden y el nombre son
/// parte del contrato estable. Los totales reflejan **siempre** el conjunto
/// recibido (no el de toda la base de datos), con independencia del límite de
/// filas mostradas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReposDocument {
    /// Versión del esquema del documento.
    pub schema_version: u32,
    /// Instante de generación del documento.
    pub generated_at: DateTime<Utc>,
    /// Ejecución a la que se refiere el listado.
    pub run_id: i64,
    /// Repositorios conocidos (filas de la tabla `repos`).
    pub known_repos: i64,
    /// Repositorios con al menos una alerta abierta en el conjunto recibido
    /// (antes de aplicar `limit`; con `--min-severity` es el conjunto ya filtrado).
    pub repos_with_alerts: i64,
    /// Suma de las alertas abiertas del conjunto recibido (antes de aplicar
    /// `limit`; con `--min-severity` es el conjunto ya filtrado).
    pub total_alerts: i64,
    /// Filas del listado, en el orden que trae el almacén.
    pub repos: Vec<RepoEntry>,
}

impl ReposDocument {
    /// Construye el documento a partir del listado agregado.
    ///
    /// `limit` acota cuántas filas se incluyen en `repos`; `None` (o `Some(0)`)
    /// incluye todas. Los **totales** (`known_repos`, `repos_with_alerts`,
    /// `total_alerts`) reflejan SIEMPRE el conjunto recibido, no la parte
    /// mostrada.
    ///
    /// Semántica exacta de los totales: `repos_with_alerts` y `total_alerts` son
    /// **post-filtro** de `--min-severity` y **pre-límite** de `--limit` —es
    /// decir, describen el conjunto que el almacén entregó, no toda la base de
    /// datos—; `known_repos` es siempre el total de repositorios de la base de
    /// datos, al margen del filtro.
    #[must_use]
    pub fn from_list(
        list: &RepoAlertList,
        run_id: i64,
        limit: Option<usize>,
        now: DateTime<Utc>,
    ) -> Self {
        let repos: Vec<RepoEntry> = match limit {
            None | Some(0) => list.repos.iter().map(fila).collect(),
            Some(limite) => list.repos.iter().take(limite).map(fila).collect(),
        };

        Self {
            schema_version: REPOS_SCHEMA_VERSION,
            generated_at: now,
            run_id,
            known_repos: list.known_repos,
            repos_with_alerts: i64::try_from(list.repos.len()).unwrap_or(i64::MAX),
            total_alerts: list.total_alerts,
            repos,
        }
    }

    /// Serializa el documento a JSON con claves estables y sangrado legible.
    ///
    /// # Errores
    ///
    /// Devuelve [`serde_json::Error`] si la serialización falla (no debería
    /// ocurrir con los tipos del documento).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Renderiza el listado en Markdown.
    ///
    /// Emite una tabla con una columna por severidad y una fila por repositorio;
    /// con el listado vacío, una única fila `(ninguno)`.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut md = String::with_capacity(1024);
        md.push_str("# Repositorios con alertas\n\n");
        md.push_str(&format!("- Ejecución: #{}\n", self.run_id));
        md.push_str(&format!(
            "- Generado: {}\n",
            self.generated_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        md.push_str(&format!(
            "- Repositorios con alertas: {} de {}\n",
            self.repos_with_alerts, self.known_repos
        ));
        md.push_str(&format!("- Alertas abiertas: {}\n\n", self.total_alerts));

        md.push_str(
            "| Repositorio | Alertas | Críticas | Altas | Medias | Bajas | Desconocidas |\n",
        );
        md.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
        if self.repos.is_empty() {
            md.push_str("| (ninguno) | 0 | 0 | 0 | 0 | 0 | 0 |\n");
        } else {
            for fila in &self.repos {
                md.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} |\n",
                    fila.repo,
                    fila.alerts,
                    fila.by_severity.critical,
                    fila.by_severity.high,
                    fila.by_severity.medium,
                    fila.by_severity.low,
                    fila.by_severity.unknown
                ));
            }
        }
        md
    }

    /// Renderiza el listado en texto de consola.
    ///
    /// Emite una línea de cabecera con los totales y una línea por repositorio
    /// con su nombre, su recuento y su desglose; con el listado vacío, una sola
    /// línea `sin repositorios con alertas`.
    #[must_use]
    pub fn to_console(&self) -> String {
        let mut salida = String::with_capacity(512);
        salida.push_str(&format!(
            "ejecución #{} · generado {} · repositorios con alertas: {} de {} · alertas abiertas: {}\n",
            self.run_id,
            self.generated_at.to_rfc3339(),
            self.repos_with_alerts,
            self.known_repos,
            self.total_alerts
        ));

        if self.repos.is_empty() {
            salida.push_str("sin repositorios con alertas\n");
        } else {
            for fila in &self.repos {
                salida.push_str(&format!(
                    "{}  {}  (critical={} high={} medium={} low={} unknown={})\n",
                    fila.repo,
                    fila.alerts,
                    fila.by_severity.critical,
                    fila.by_severity.high,
                    fila.by_severity.medium,
                    fila.by_severity.low,
                    fila.by_severity.unknown
                ));
            }
        }
        salida
    }
}

/// Convierte un recuento del almacén en una fila del documento.
fn fila(recuento: &RepoAlertCount) -> RepoEntry {
    RepoEntry {
        repo: recuento.repo.clone(),
        alerts: recuento.alerts,
        by_severity: RepoSeverityCounts::from(recuento),
    }
}
