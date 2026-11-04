//! Composición del mensaje y el metadato de una notificación.
//!
//! Valet impone dos límites duros: `message` no puede pasar de
//! [`MESSAGE_LIMIT`] bytes (tras `trim`) y `metadata` no puede pasar de
//! [`METADATA_LIMIT`] bytes serializados. El informe completo nunca viaja en el
//! mensaje: se resume y se recorta **por frontera de carácter** ([`truncate_utf8`]),
//! y se referencia la ruta del informe en disco. El metadato se construye a
//! partir de campos acotados y se **reduce** si excediera el límite, de modo que
//! jamás provoca un `400`.

use std::path::Path;

use serde::Serialize;

use crate::report::{RepoCount, ReportDocument, SeverityCounts};

/// Límite de bytes del campo `message` (tras `trim`).
pub const MESSAGE_LIMIT: usize = 4096;
/// Límite de bytes de la serialización del campo `metadata`.
pub const METADATA_LIMIT: usize = 8192;
/// Versión del esquema del metadato.
const METADATA_SCHEMA_VERSION: u32 = 1;
/// Número máximo de repositorios que viajan en el metadato.
const WORST_REPOS_EN_METADATO: usize = 5;
/// Bytes a los que se recorta la ruta del informe en la reducción extrema.
const REPORT_PATH_MINIMO: usize = 128;

/// Nivel de la notificación admitido por Valet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    /// Informativo: solo severidades medias o bajas.
    Info,
    /// Aviso: hay severidades altas y ninguna crítica.
    Warning,
    /// Crítico: hay al menos una alerta crítica.
    Critical,
}

impl NotifyLevel {
    /// Representación textual, la que espera Valet en el campo `level`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

/// Cuerpo de la notificación, tal cual viaja a Valet.
#[derive(Debug, Clone, Serialize)]
pub struct ValetNotification {
    /// Mensaje legible, de como máximo [`MESSAGE_LIMIT`] bytes.
    pub message: String,
    /// Nivel derivado de la severidad de los hallazgos.
    pub level: NotifyLevel,
    /// Metadato estructurado, de como máximo [`METADATA_LIMIT`] bytes.
    pub metadata: serde_json::Value,
}

/// Deriva el nivel a partir del desglose por severidad.
///
/// Cualquier alerta crítica produce `critical`; en su defecto, cualquier alerta
/// alta produce `warning`; en otro caso, `info`.
#[must_use]
pub fn level_for(counts: &SeverityCounts) -> NotifyLevel {
    if counts.critical > 0 {
        NotifyLevel::Critical
    } else if counts.high > 0 {
        NotifyLevel::Warning
    } else {
        NotifyLevel::Info
    }
}

/// Decide si hay que notificar.
///
/// Por defecto solo se notifica cuando hay alertas nuevas o resueltas (evita el
/// aviso diario sin novedades); `force` salta esa regla.
#[must_use]
pub fn should_notify(new: i64, resolved: i64, force: bool) -> bool {
    force || new > 0 || resolved > 0
}

/// Recorta `input` a lo sumo `max_bytes` bytes sin partir caracteres multibyte.
///
/// Devuelve el prefijo más largo que cabe en `max_bytes` y termina en una
/// frontera de carácter, de modo que el resultado siempre es UTF-8 válido.
#[must_use]
pub fn truncate_utf8(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut fin = max_bytes;
    while fin > 0 && !input.is_char_boundary(fin) {
        fin -= 1;
    }
    &input[..fin]
}

/// Compone la notificación (mensaje, nivel y metadato) de un informe.
#[must_use]
pub fn build_notification(document: &ReportDocument, report_path: &Path) -> ValetNotification {
    let ruta = report_path.display().to_string();
    let message = componer_mensaje(&cuerpo(document), &ruta);
    let level = level_for(&document.by_severity);
    let metadata = reducir_metadato(construir_metadato(document, &ruta));
    ValetNotification {
        message,
        level,
        metadata,
    }
}

/// Cuerpo textual de la notificación, antes de recortarlo al límite.
fn cuerpo(document: &ReportDocument) -> String {
    let mut lineas = vec![
        format!(
            "ghwatch: {} alertas en {} repositorios",
            document.total, document.scanned
        ),
        format!(
            "Nuevas: {} · Resueltas: {}",
            document.new, document.resolved
        ),
        format!(
            "Críticas: {} · Altas: {} · Medias: {} · Bajas: {}",
            document.by_severity.critical,
            document.by_severity.high,
            document.by_severity.medium,
            document.by_severity.low
        ),
    ];

    if !document.worst_repos.is_empty() {
        let peores = document
            .worst_repos
            .iter()
            .map(|repo| format!("{} ({})", repo.repo, repo.alerts))
            .collect::<Vec<_>>()
            .join(", ");
        lineas.push(format!("Peores: {peores}"));
    }

    lineas.join("\n")
}

/// Ensambla el mensaje: cuerpo recortado más la ruta del informe completo.
///
/// La ruta se añade **después** de recortar el cuerpo, de modo que nunca se
/// pierde: el cuerpo se recorta al presupuesto que deja libre la línea de la
/// ruta. Solo en el caso patológico de una ruta que por sí sola exceda el
/// límite se recorta también la ruta.
fn componer_mensaje(cuerpo: &str, ruta: &str) -> String {
    let linea = format!("Informe completo: {ruta}");
    let presupuesto = MESSAGE_LIMIT.saturating_sub(linea.len() + 1);
    let cuerpo = truncate_utf8(cuerpo.trim(), presupuesto);

    let mensaje = if cuerpo.is_empty() {
        linea
    } else {
        format!("{cuerpo}\n{linea}")
    };

    if mensaje.len() > MESSAGE_LIMIT {
        truncate_utf8(&mensaje, MESSAGE_LIMIT).to_string()
    } else {
        mensaje
    }
}

/// Metadato serializable, con los campos acotados por el contrato de Valet.
#[derive(Debug, Serialize)]
struct Metadato<'a> {
    /// Versión del esquema del metadato.
    schema_version: u32,
    /// Ejecución a la que se refiere.
    run_id: i64,
    /// Repositorios escaneados.
    scanned: usize,
    /// Desglose por severidad.
    by_severity: BySeverity,
    /// Alertas nuevas.
    new: i64,
    /// Alertas resueltas.
    resolved: i64,
    /// Los peores repositorios (hasta [`WORST_REPOS_EN_METADATO`]).
    worst_repos: Vec<PeorRepo<'a>>,
    /// Ruta del informe completo en disco.
    report_path: &'a str,
}

/// Desglose por severidad del metadato (solo las cuatro severidades conocidas).
#[derive(Debug, Serialize)]
struct BySeverity {
    /// Alertas críticas.
    critical: u64,
    /// Alertas altas.
    high: u64,
    /// Alertas medias.
    medium: u64,
    /// Alertas bajas.
    low: u64,
}

/// Repositorio del metadato.
#[derive(Debug, Serialize)]
struct PeorRepo<'a> {
    /// Nombre completo del repositorio.
    repo: &'a str,
    /// Número de alertas.
    alerts: usize,
}

/// Construye el metadato, tomando los cinco peores repositorios del informe.
fn construir_metadato(document: &ReportDocument, ruta: &str) -> serde_json::Value {
    let metadato = Metadato {
        schema_version: METADATA_SCHEMA_VERSION,
        run_id: document.run_id,
        scanned: document.scanned,
        by_severity: BySeverity {
            critical: document.by_severity.critical,
            high: document.by_severity.high,
            medium: document.by_severity.medium,
            low: document.by_severity.low,
        },
        new: document.new,
        resolved: document.resolved,
        worst_repos: document
            .worst_repos
            .iter()
            .take(WORST_REPOS_EN_METADATO)
            .map(PeorRepo::from)
            .collect(),
        report_path: ruta,
    };
    serde_json::to_value(metadato).unwrap_or(serde_json::Value::Null)
}

impl<'a> From<&'a RepoCount> for PeorRepo<'a> {
    fn from(entrada: &'a RepoCount) -> Self {
        Self {
            repo: &entrada.repo,
            alerts: entrada.alerts,
        }
    }
}

/// Reduce el metadato hasta caber en [`METADATA_LIMIT`] bytes.
///
/// La estrategia es determinista y conserva las claves obligatorias: primero se
/// vacía `worst_repos` (que es la parte variable) y, si aun así no cabe, se
/// recorta `report_path`. Los campos restantes son de tamaño fijo y caben
/// holgadamente.
fn reducir_metadato(mut valor: serde_json::Value) -> serde_json::Value {
    if tamano(&valor) <= METADATA_LIMIT {
        return valor;
    }

    if let Some(objeto) = valor.as_object_mut() {
        objeto.insert(
            "worst_repos".to_string(),
            serde_json::Value::Array(Vec::new()),
        );
    }
    if tamano(&valor) <= METADATA_LIMIT {
        return valor;
    }

    if let Some(ruta) = valor
        .get("report_path")
        .and_then(|campo| campo.as_str())
        .map(str::to_owned)
    {
        let recortada = truncate_utf8(&ruta, REPORT_PATH_MINIMO).to_owned();
        valor["report_path"] = serde_json::Value::String(recortada);
    }

    valor
}

/// Tamaño en bytes de la serialización de un valor JSON.
fn tamano(valor: &serde_json::Value) -> usize {
    serde_json::to_vec(valor).map_or(usize::MAX, |bytes| bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El nivel depende solo de la presencia de críticas y altas.
    #[test]
    fn el_nivel_prioriza_criticas_y_altas() {
        assert_eq!(
            level_for(&SeverityCounts {
                critical: 0,
                high: 0,
                medium: 1,
                low: 0,
                unknown: 0,
            }),
            NotifyLevel::Info
        );
        assert_eq!(
            level_for(&SeverityCounts {
                critical: 0,
                high: 3,
                medium: 0,
                low: 0,
                unknown: 0,
            }),
            NotifyLevel::Warning
        );
        assert_eq!(
            level_for(&SeverityCounts {
                critical: 1,
                high: 0,
                medium: 0,
                low: 0,
                unknown: 0,
            }),
            NotifyLevel::Critical
        );
    }

    /// Un cuerpo vacío no deja el mensaje vacío: conserva la ruta del informe.
    #[test]
    fn el_mensaje_nunca_queda_vacio() {
        let mensaje = componer_mensaje("   ", "/tmp/informe.md");
        assert!(!mensaje.trim().is_empty());
        assert!(mensaje.contains("/tmp/informe.md"));
    }
}
