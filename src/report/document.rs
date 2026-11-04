//! Documento del informe: modelo estable, versionado y serializable.
//!
//! [`ReportDocument`] es la representación intermedia entre el resultado del
//! escaneo ([`ScanSummary`]) y las tres salidas del informe (consola, Markdown y
//! JSON). Se construye una sola vez con [`ReportDocument::from_scan`] y a partir
//! de ahí es una estructura de datos pura: no consulta la red ni la base de
//! datos.
//!
//! El JSON es **la única fuente de verdad observable**: todas las claves son
//! estables y el documento declara su [`SCHEMA_VERSION`] para que un consumidor
//! pueda evolucionar sin adivinar.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::alert::{Alert, Severity};
use crate::scan::ScanSummary;
use crate::store::HistoryPoint;

/// Versión del esquema del informe.
///
/// Se declara en el JSON (`schema_version`) para que el formato pueda crecer de
/// forma compatible.
pub const SCHEMA_VERSION: u32 = 1;

/// Número de repositorios que lista la tabla de cabecera del informe.
///
/// La tabla muestra **hasta** [`REPOS_EN_INFORME`] repositorios: si hay menos
/// con alertas, se listan solo los disponibles.
const REPOS_EN_INFORME: usize = 15;

/// Número máximo de ejecuciones que vuelca la sección de evolución.
///
/// `from_scan` acota la historia recibida a las **últimas** [`HISTORY_EN_INFORME`]
/// ejecuciones; la serie se conserva en orden cronológico.
pub const HISTORY_EN_INFORME: usize = 30;

/// Clave estable del desglose por ecosistema para las alertas sin ecosistema.
///
/// Garantiza que el desglose no pierda alertas: la suma de sus cubos coincide
/// con el total.
const ECOSISTEMA_DESCONOCIDO: &str = "desconocido";

/// Desglose de alertas por severidad normalizada.
///
/// `unknown` recoge tanto las alertas sin severidad como las que la fuente marcó
/// con un valor no reconocido; nunca se pierde ninguna alerta del total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeverityCounts {
    /// Alertas críticas.
    pub critical: u64,
    /// Alertas altas.
    pub high: u64,
    /// Alertas medias.
    pub medium: u64,
    /// Alertas bajas.
    pub low: u64,
    /// Alertas sin severidad conocida.
    pub unknown: u64,
}

impl SeverityCounts {
    /// Cuenta las alertas por severidad.
    #[must_use]
    pub fn from_alerts<'a>(alerts: impl IntoIterator<Item = &'a Alert>) -> Self {
        let mut conteo = Self::default();
        for alerta in alerts {
            match alerta.severity {
                Some(Severity::Critical) => conteo.critical += 1,
                Some(Severity::High) => conteo.high += 1,
                Some(Severity::Medium) => conteo.medium += 1,
                Some(Severity::Low) => conteo.low += 1,
                None | Some(Severity::Unknown) => conteo.unknown += 1,
            }
        }
        conteo
    }

    /// Total de alertas contabilizadas.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.critical + self.high + self.medium + self.low + self.unknown
    }
}

/// Alertas de un ecosistema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EcosystemCount {
    /// Nombre del ecosistema (tal cual lo aporta la alerta).
    pub ecosystem: String,
    /// Número de alertas de ese ecosistema.
    pub alerts: usize,
}

/// Alertas de un repositorio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoCount {
    /// Nombre completo del repositorio (`owner/name`).
    pub repo: String,
    /// Número de alertas del repositorio.
    pub alerts: usize,
}

/// Acción propuesta para un paquete: la versión que corrige y su alcance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    /// Paquete afectado.
    pub package: String,
    /// Primera versión que corrige la vulnerabilidad, si consta.
    pub fixed_version: Option<String>,
    /// Número de repositorios distintos afectados.
    pub repos: usize,
}

/// Entrada de la serie histórica, ya con el neto calculado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Identificador de la ejecución.
    pub run_id: i64,
    /// Instante de inicio de la ejecución.
    pub started_at: DateTime<Utc>,
    /// Alertas presentes al terminar la ejecución.
    pub total: i64,
    /// Alertas nuevas de esa ejecución.
    pub new: i64,
    /// Alertas resueltas en esa ejecución.
    pub resolved: i64,
    /// Saldo neto de la ejecución (`new - resolved`).
    pub net: i64,
}

/// Documento completo del informe.
///
/// Todas las claves del JSON derivan de estos campos; el orden y el nombre son
/// parte del contrato estable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportDocument {
    /// Versión del esquema del informe.
    pub schema_version: u32,
    /// Instante de generación del informe.
    pub generated_at: DateTime<Utc>,
    /// Ejecución a la que se refiere el informe.
    pub run_id: i64,
    /// Repositorios escaneados con éxito.
    pub scanned: usize,
    /// Repositorios que fallaron durante el escaneo.
    pub failed: usize,
    /// Alertas totales presentes.
    pub total: usize,
    /// Desglose por severidad.
    pub by_severity: SeverityCounts,
    /// Desglose por ecosistema, ordenado por número de alertas descendente.
    pub by_ecosystem: Vec<EcosystemCount>,
    /// Repositorios con más alertas (hasta [`REPOS_EN_INFORME`]).
    pub worst_repos: Vec<RepoCount>,
    /// Alertas nuevas de esta ejecución.
    pub new: i64,
    /// Alertas resueltas en esta ejecución.
    pub resolved: i64,
    /// Acciones propuestas, ordenadas por repositorios afectados descendente.
    pub actions: Vec<Action>,
    /// Serie histórica, en orden cronológico.
    pub history: Vec<HistoryEntry>,
}

impl ReportDocument {
    /// Construye el documento a partir del resumen del escaneo y su histórico.
    ///
    /// `history` es la serie completa en orden cronológico; `history_limit`
    /// acota cuántas de las **últimas** ejecuciones se vuelcan en el informe
    /// ([`HISTORY_EN_INFORME`] es un valor razonable). `new` y `resolved` los
    /// aporta la capa de persistencia (la deriva de la ejecución): el informe no
    /// los recalcula para no divergir del histórico almacenado.
    #[must_use]
    pub fn from_scan(
        summary: &ScanSummary,
        history: &[HistoryPoint],
        history_limit: usize,
        run_id: i64,
        new: i64,
        resolved: i64,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            generated_at: now,
            run_id,
            scanned: summary.succeeded_count(),
            failed: summary.failed_count(),
            total: summary.alert_count(),
            by_severity: SeverityCounts::from_alerts(summary.alerts()),
            by_ecosystem: ecosistemas(summary),
            worst_repos: peores_repos(summary),
            new,
            resolved,
            actions: acciones(summary),
            history: recortar_historia(history, history_limit),
        }
    }

    /// Serializa el informe a JSON con claves estables y sangrado legible.
    ///
    /// # Errores
    ///
    /// Devuelve [`serde_json::Error`] si la serialización falla (no debería
    /// ocurrir con los tipos del documento).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Convierte un punto del histórico en una entrada del informe con su neto.
fn entrada_historica(punto: &HistoryPoint) -> HistoryEntry {
    HistoryEntry {
        run_id: punto.run_id,
        started_at: punto.started_at,
        total: punto.total,
        new: punto.new,
        resolved: punto.resolved,
        net: punto.new - punto.resolved,
    }
}

/// Acota la historia a las **últimas** `limite` ejecuciones, en orden.
///
/// La serie llega en orden cronológico (la más antigua primero); se conserva la
/// cola para que la evolución muestre lo más reciente. `limite` igual a cero no
/// produce ninguna entrada.
fn recortar_historia(history: &[HistoryPoint], limite: usize) -> Vec<HistoryEntry> {
    let inicio = history.len().saturating_sub(limite);
    history[inicio..].iter().map(entrada_historica).collect()
}

/// Agrega las alertas por ecosistema, de más a menos alertas.
///
/// Las alertas **sin ecosistema** (o con uno vacío) no se descartan: se agrupan
/// bajo la clave estable [`ECOSISTEMA_DESCONOCIDO`], de modo que la suma del
/// desglose coincide siempre con el total de alertas del informe.
fn ecosistemas(summary: &ScanSummary) -> Vec<EcosystemCount> {
    let mut por_ecosistema: BTreeMap<&str, usize> = BTreeMap::new();
    for alerta in summary.alerts() {
        let clave = alerta
            .ecosystem
            .as_deref()
            .filter(|ecosistema| !ecosistema.is_empty())
            .unwrap_or(ECOSISTEMA_DESCONOCIDO);
        *por_ecosistema.entry(clave).or_default() += 1;
    }

    let mut lista: Vec<EcosystemCount> = por_ecosistema
        .into_iter()
        .map(|(ecosystem, alerts)| EcosystemCount {
            ecosystem: ecosystem.to_string(),
            alerts,
        })
        .collect();
    lista.sort_by(|a, b| {
        b.alerts
            .cmp(&a.alerts)
            .then_with(|| a.ecosystem.cmp(&b.ecosystem))
    });
    lista
}

/// Selecciona los repositorios con más alertas, de más a menos.
///
/// La lista tiene **hasta** [`REPOS_EN_INFORME`] entradas: si menos repositorios
/// tienen alertas, se devuelven solo los disponibles (los sin alertas no
/// aparecen).
fn peores_repos(summary: &ScanSummary) -> Vec<RepoCount> {
    let mut lista: Vec<RepoCount> = summary
        .scanned
        .iter()
        .map(|repo| RepoCount {
            repo: repo.repo.name_with_owner.clone(),
            alerts: repo.alert_count(),
        })
        .filter(|entrada| entrada.alerts > 0)
        .collect();
    lista.sort_by(|a, b| b.alerts.cmp(&a.alerts).then_with(|| a.repo.cmp(&b.repo)));
    lista.truncate(REPOS_EN_INFORME);
    lista
}

/// Agrupa las alertas por paquete y versión corregida, contando repositorios.
fn acciones(summary: &ScanSummary) -> Vec<Action> {
    let mut por_paquete: BTreeMap<(String, Option<String>), BTreeSet<String>> = BTreeMap::new();
    for repo in &summary.scanned {
        for alerta in repo.alerts() {
            if let Some(paquete) = &alerta.package {
                por_paquete
                    .entry((paquete.clone(), alerta.first_patched.clone()))
                    .or_default()
                    .insert(repo.repo.name_with_owner.clone());
            }
        }
    }

    let mut lista: Vec<Action> = por_paquete
        .into_iter()
        .map(|((package, fixed_version), repos)| Action {
            package,
            fixed_version,
            repos: repos.len(),
        })
        .collect();
    lista.sort_by(|a, b| {
        b.repos
            .cmp(&a.repos)
            .then_with(|| a.package.cmp(&b.package))
            .then_with(|| a.fixed_version.cmp(&b.fixed_version))
    });
    lista
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::domain::alert::AlertSource;

    /// Alerta mínima con la severidad y el ecosistema indicados.
    fn alerta(severidad: Option<Severity>, ecosistema: Option<&str>) -> Alert {
        Alert {
            source: AlertSource::Dependabot,
            external_id: "1".to_string(),
            ghsa_id: None,
            cve_id: None,
            severity: severidad,
            cvss: None,
            summary: None,
            package: None,
            ecosystem: ecosistema.map(str::to_string),
            manifest_path: None,
            scope: None,
            vulnerable_range: None,
            first_patched: None,
            secret_type: None,
            created_at: None,
        }
    }

    /// El desglose por severidad incluye las alertas sin severidad conocida.
    #[test]
    fn severity_counts_cuenta_las_desconocidas() {
        let alertas = [
            alerta(Some(Severity::Critical), None),
            alerta(Some(Severity::High), None),
            alerta(None, None),
        ];
        let conteo = SeverityCounts::from_alerts(&alertas);
        assert_eq!(conteo.critical, 1);
        assert_eq!(conteo.high, 1);
        assert_eq!(conteo.unknown, 1);
        assert_eq!(conteo.total(), 2 + 1);
    }

    /// El neto del histórico es `new - resolved`.
    #[test]
    fn la_entrada_historica_calcula_el_neto() {
        let punto = HistoryPoint {
            run_id: 3,
            started_at: Utc
                .timestamp_opt(1_700_000_000, 0)
                .single()
                .expect("instante"),
            total: 10,
            new: 2,
            resolved: 5,
        };
        assert_eq!(entrada_historica(&punto).net, -3);
    }

    /// El documento serializado se puede volver a leer sin perder ningún campo:
    /// el JSON es la fuente de verdad que consumen `report` y `notify`.
    #[test]
    fn el_documento_sobrevive_a_un_viaje_por_json() {
        let original = ReportDocument {
            schema_version: SCHEMA_VERSION,
            generated_at: Utc
                .timestamp_opt(1_700_000_000, 0)
                .single()
                .expect("instante"),
            run_id: 7,
            scanned: 367,
            failed: 2,
            total: 1333,
            by_severity: SeverityCounts {
                critical: 3,
                high: 40,
                medium: 1290,
                low: 0,
                unknown: 0,
            },
            by_ecosystem: vec![EcosystemCount {
                ecosystem: "pip".to_string(),
                alerts: 1333,
            }],
            worst_repos: vec![RepoCount {
                repo: "atareao/uno".to_string(),
                alerts: 15,
            }],
            new: 10,
            resolved: 2,
            actions: vec![Action {
                package: "django".to_string(),
                fixed_version: Some("3.0.2".to_string()),
                repos: 4,
            }],
            history: vec![HistoryEntry {
                run_id: 7,
                started_at: Utc
                    .timestamp_opt(1_700_000_000, 0)
                    .single()
                    .expect("instante"),
                total: 1333,
                new: 10,
                resolved: 2,
                net: 8,
            }],
        };

        let json = original.to_json().expect("serializa el documento");
        let recuperado: ReportDocument =
            serde_json::from_str(&json).expect("deserializa el documento");

        assert_eq!(recuperado.schema_version, original.schema_version);
        assert_eq!(recuperado.generated_at, original.generated_at);
        assert_eq!(recuperado.run_id, original.run_id);
        assert_eq!(recuperado.scanned, original.scanned);
        assert_eq!(recuperado.failed, original.failed);
        assert_eq!(recuperado.total, original.total);
        assert_eq!(recuperado.by_severity, original.by_severity);
        assert_eq!(recuperado.by_ecosystem, original.by_ecosystem);
        assert_eq!(recuperado.worst_repos, original.worst_repos);
        assert_eq!(recuperado.new, original.new);
        assert_eq!(recuperado.resolved, original.resolved);
        assert_eq!(recuperado.actions, original.actions);
        assert_eq!(recuperado.history, original.history);
    }
}
