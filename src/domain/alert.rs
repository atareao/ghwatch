//! Modelo único de alerta.
//!
//! Independientemente de su origen (Dependabot, secret scanning o code
//! scanning), toda alerta se normaliza a un [`Alert`] con los mismos campos. Los
//! que una fuente no aporta quedan a `None`: por ejemplo, el `cvss` de una
//! alerta de secreto o el `secret_type` de una alerta de Dependabot.

use serde::{Deserialize, Serialize};

/// Origen de una alerta.
///
/// Conjunto cerrado: la API de GitHub solo ofrece estas tres fuentes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSource {
    /// Alertas de Dependabot.
    Dependabot,
    /// Alertas de secret scanning.
    Secret,
    /// Alertas de code scanning.
    Code,
}

/// Severidad normalizada de una alerta.
///
/// GitHub usa etiquetas distintas según la fuente; aquí se unifican. Cualquier
/// valor no reconocido se degrada a [`Severity::Unknown`] en lugar de fallar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Severidad crítica.
    Critical,
    /// Severidad alta.
    High,
    /// Severidad media.
    Medium,
    /// Severidad baja.
    Low,
    /// Severidad desconocida o no aportada.
    Unknown,
}

impl AlertSource {
    /// Representación textual estable, en `snake_case`.
    ///
    /// Coincide con la serialización de `serde` y con el valor almacenado en la
    /// base de datos.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dependabot => "dependabot",
            Self::Secret => "secret",
            Self::Code => "code",
        }
    }

    /// Interpreta la representación textual; `None` si no se reconoce.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dependabot" => Some(Self::Dependabot),
            "secret" => Some(Self::Secret),
            "code" => Some(Self::Code),
            _ => None,
        }
    }
}

impl Severity {
    /// Interpreta la etiqueta de severidad que devuelve GitHub.
    ///
    /// La comparación es insensible a mayúsculas. Un valor no reconocido (o
    /// vacío) devuelve [`Severity::Unknown`]: una etiqueta nueva de GitHub no
    /// debe romper la recolección.
    #[must_use]
    pub fn from_github(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "critical" => Self::Critical,
            "high" | "error" => Self::High,
            "medium" | "warning" => Self::Medium,
            "low" | "note" => Self::Low,
            _ => Self::Unknown,
        }
    }

    /// Orden relativo de la severidad, de menor a mayor.
    ///
    /// [`Severity::Unknown`] vale cero, de modo que pasar de una severidad
    /// desconocida a una conocida cuenta como subida.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Critical => 4,
            Self::High => 3,
            Self::Medium => 2,
            Self::Low => 1,
            Self::Unknown => 0,
        }
    }

    /// Representación textual estable, en `snake_case`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Unknown => "unknown",
        }
    }
}

/// Alerta normalizada, con independencia de su origen.
///
/// Los campos que una fuente no aporta quedan a `None`. `cvss` es la única
/// magnitud numérica y por eso se guarda como `Option<f64>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    /// Fuente de la alerta.
    pub source: AlertSource,
    /// Identificador de la alerta dentro de su fuente (el `number` de GitHub).
    pub external_id: String,
    /// Identificador GHSA, si lo hay.
    pub ghsa_id: Option<String>,
    /// Identificador CVE, si lo hay.
    pub cve_id: Option<String>,
    /// Severidad normalizada, si la fuente la aporta.
    pub severity: Option<Severity>,
    /// Puntuación CVSS, si la fuente la aporta.
    pub cvss: Option<f64>,
    /// Resumen legible de la alerta.
    pub summary: Option<String>,
    /// Nombre del paquete afectado.
    pub package: Option<String>,
    /// Ecosistema del paquete afectado.
    pub ecosystem: Option<String>,
    /// Ruta del manifiesto en el que aparece la dependencia.
    pub manifest_path: Option<String>,
    /// Ámbito de la dependencia (`runtime`, `development`…).
    pub scope: Option<String>,
    /// Rango de versiones vulnerables.
    pub vulnerable_range: Option<String>,
    /// Primera versión que corrige la vulnerabilidad.
    pub first_patched: Option<String>,
    /// Tipo de secreto expuesto (solo en alertas de secret scanning).
    pub secret_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `from_github` reconoce las etiquetas habituales sin distinguir mayúsculas.
    #[test]
    fn severity_from_github_reconoce_las_etiquetas() {
        assert_eq!(Severity::from_github("critical"), Severity::Critical);
        assert_eq!(Severity::from_github("HIGH"), Severity::High);
        assert_eq!(Severity::from_github("Medium"), Severity::Medium);
        assert_eq!(Severity::from_github("low"), Severity::Low);
    }

    /// Las etiquetas propias de code scanning se mapean a la severidad análoga.
    #[test]
    fn severity_from_github_mapea_las_etiquetas_de_code_scanning() {
        assert_eq!(Severity::from_github("error"), Severity::High);
        assert_eq!(Severity::from_github("warning"), Severity::Medium);
        assert_eq!(Severity::from_github("note"), Severity::Low);
    }

    /// Una etiqueta desconocida o vacía no rompe: se degrada a `Unknown`.
    #[test]
    fn severity_from_github_etiqueta_desconocida_es_unknown() {
        assert_eq!(Severity::from_github(""), Severity::Unknown);
        assert_eq!(Severity::from_github("inventada"), Severity::Unknown);
    }

    /// El origen se serializa en `snake_case` como exige el modelo de datos.
    #[test]
    fn alert_source_se_serializa_en_snake_case() {
        assert_eq!(
            serde_json::to_string(&AlertSource::Dependabot).expect("serializable"),
            "\"dependabot\""
        );
        assert_eq!(
            serde_json::to_string(&AlertSource::Secret).expect("serializable"),
            "\"secret\""
        );
        assert_eq!(
            serde_json::to_string(&AlertSource::Code).expect("serializable"),
            "\"code\""
        );
    }
}
