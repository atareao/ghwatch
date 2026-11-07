//! Presentación del listado de repositorios del conjunto consultado.
//!
//! Este módulo es **presentación pura** sobre [`RepoAlertList`]: no consulta la
//! red ni la base de datos, solo compone las tres salidas (consola, Markdown y
//! JSON) a partir del listado ya agregado por el almacén. El mismo listado, los
//! mismos filtros y el mismo instante producen exactamente la misma salida en
//! los tres formatos.
//!
//! [`ReposDocument`] es el modelo estable y versionado del listado; de él
//! derivan [`ReposDocument::to_markdown`], [`ReposDocument::to_json`] y
//! [`ReposDocument::to_console`]. El JSON es la fuente de verdad observable:
//! todas sus claves son estables y el documento declara su
//! [`REPOS_SCHEMA_VERSION`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::alert::{AlertSource, Severity};
use crate::store::{RepoAlertCount, RepoAlertList};

/// Versión del esquema del documento del listado de repositorios.
///
/// Se declara en el JSON (`schema_version`) para que el formato pueda crecer de
/// forma compatible. La versión 2 añade los filtros aplicados (`state`,
/// `source`, `min_severity`, `include_empty`) y el total `listed_repos`.
pub const REPOS_SCHEMA_VERSION: u32 = 2;

/// Estado de las alertas que describe el listado.
///
/// Es un enum propio de la presentación: fija la etiqueta de la cabecera de
/// consola y el título del listado en Markdown. La consulta subyacente usa
/// `Option<AlertState>` (el almacén distingue «uno» de «todos»), pero el
/// documento declara siempre un estado concreto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoState {
    /// Alertas abiertas.
    Open,
    /// Alertas resueltas.
    Resolved,
    /// Abiertas y resueltas.
    All,
}

impl RepoState {
    /// Representación textual estable, en `snake_case`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::All => "all",
        }
    }

    /// Etiqueta de la cabecera de consola.
    #[must_use]
    pub const fn console_label(self) -> &'static str {
        match self {
            Self::Open => "alertas abiertas",
            Self::Resolved => "alertas resueltas",
            Self::All => "alertas",
        }
    }

    /// Título del listado en Markdown.
    #[must_use]
    pub const fn markdown_title(self) -> &'static str {
        match self {
            Self::Open => "Repositorios con alertas abiertas",
            Self::Resolved => "Repositorios con alertas resueltas",
            Self::All => "Repositorios y sus alertas",
        }
    }
}

/// Filtros que describen el conjunto del listado.
///
/// Agrupa los ejes de la consulta en un único valor para no inflar la firma de
/// [`ReposDocument::from_list`]. El documento solo los **refleja**: el filtrado
/// real ocurre en el almacén.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReposFilters {
    /// Estado de las alertas contadas.
    pub state: RepoState,
    /// Fuente contada (`None` = todas).
    pub source: Option<AlertSource>,
    /// Severidad mínima aplicada (`None` = sin mínimo).
    pub min_severity: Option<Severity>,
    /// Si se incluyeron los repositorios sin alertas.
    pub include_empty: bool,
}

/// Desglose por severidad de las alertas del conjunto consultado de un
/// repositorio.
///
/// Los cubos son `i64` —y no `u64` como en [`crate::report::SeverityCounts`]—
/// porque el almacén devuelve los recuentos de SQLite tal cual (`COUNT(*)` es
/// `i64`): así el documento los refleja **sin conversiones con `as`** ni
/// reescrituras de signo. La suma de los cinco cubos coincide con el recuento
/// total de alertas del repositorio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSeverityCounts {
    /// Alertas del conjunto de severidad crítica.
    pub critical: i64,
    /// Alertas del conjunto de severidad alta.
    pub high: i64,
    /// Alertas del conjunto de severidad media.
    pub medium: i64,
    /// Alertas del conjunto de severidad baja.
    pub low: i64,
    /// Alertas del conjunto sin severidad conocida (incluye las de severidad
    /// nula).
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

/// Una fila del listado: un repositorio con sus alertas del conjunto consultado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoEntry {
    /// Repositorio (`owner/name`).
    pub repo: String,
    /// Alertas del repositorio en el conjunto consultado.
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
    /// Estado de las alertas contadas.
    pub state: RepoState,
    /// Fuente contada (`null` = todas).
    pub source: Option<AlertSource>,
    /// Severidad mínima aplicada (`null` = sin mínimo).
    pub min_severity: Option<Severity>,
    /// Si el listado incluye los repositorios sin alertas.
    pub include_empty: bool,
    /// Repositorios conocidos (filas de la tabla `repos`), al margen de los
    /// filtros.
    pub known_repos: i64,
    /// Repositorios del conjunto (pre-límite).
    pub listed_repos: i64,
    /// Repositorios del conjunto con al menos una alerta (pre-límite).
    pub repos_with_alerts: i64,
    /// Suma de las alertas del conjunto (pre-límite).
    pub total_alerts: i64,
    /// Filas del listado, en el orden que trae el almacén.
    pub repos: Vec<RepoEntry>,
}

impl ReposDocument {
    /// Construye el documento a partir del listado agregado y sus filtros.
    ///
    /// `limit` acota cuántas filas se incluyen en `repos`; `None` (o `Some(0)`)
    /// incluye todas. Los **totales** (`known_repos`, `listed_repos`,
    /// `repos_with_alerts`, `total_alerts`) reflejan SIEMPRE el conjunto
    /// recibido, no la parte mostrada.
    ///
    /// Semántica exacta de los totales (todos **pre-límite** de `--limit`):
    ///
    /// - `known_repos` es siempre el total de repositorios de la base de datos,
    ///   al margen de los filtros.
    /// - `listed_repos` son los repositorios del conjunto tras aplicar estado,
    ///   fuente, severidad mínima e `--include-empty` (coincide con
    ///   `list.repos.len()`).
    /// - `repos_with_alerts` son los repositorios del conjunto con al menos una
    ///   alerta; sin `--include-empty` coincide con `listed_repos`.
    /// - `total_alerts` es la suma de las alertas del conjunto.
    #[must_use]
    pub fn from_list(
        list: &RepoAlertList,
        run_id: i64,
        limit: Option<usize>,
        filters: ReposFilters,
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
            state: filters.state,
            source: filters.source,
            min_severity: filters.min_severity,
            include_empty: filters.include_empty,
            known_repos: list.known_repos,
            listed_repos: i64::try_from(list.repos.len()).unwrap_or(i64::MAX),
            repos_with_alerts: list.repos_with_alerts,
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
    /// Emite el título según el estado consultado, las viñetas con los filtros y
    /// los totales, y una tabla con una columna por severidad y una fila por
    /// repositorio; con el listado vacío, una única fila `(ninguno)`.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut md = String::with_capacity(1024);
        md.push_str(&format!("# {}\n\n", self.state.markdown_title()));
        md.push_str(&format!("- Ejecución: #{}\n", self.run_id));
        md.push_str(&format!(
            "- Generado: {}\n",
            self.generated_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        md.push_str(&format!("- Estado: {}\n", self.state.as_str()));
        md.push_str(&format!(
            "- Repositorios: {} de {}\n",
            self.listed_repos, self.known_repos
        ));
        md.push_str(&format!("- Con alertas: {}\n", self.repos_with_alerts));
        md.push_str(&format!("- Alertas: {}\n\n", self.total_alerts));

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
    /// Emite una línea de cabecera con los filtros y los totales —la etiqueta de
    /// las alertas depende del estado consultado— y una línea por repositorio
    /// con su nombre, su recuento y su desglose; con el listado vacío, una sola
    /// línea `sin repositorios que mostrar`.
    #[must_use]
    pub fn to_console(&self) -> String {
        let mut salida = String::with_capacity(512);
        salida.push_str(&format!(
            "ejecución #{} · generado {} · repositorios: {} de {} · con alertas: {} · {}: {}\n",
            self.run_id,
            self.generated_at.to_rfc3339(),
            self.listed_repos,
            self.known_repos,
            self.repos_with_alerts,
            self.state.console_label(),
            self.total_alerts
        ));

        if self.repos.is_empty() {
            salida.push_str("sin repositorios que mostrar\n");
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
