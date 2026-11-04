//! Recolección y normalización de las alertas de code scanning.
//!
//! Code scanning es opcional por repositorio: cuando no hay un análisis
//! configurado, la API responde `404` con `no analysis found` y el cliente lo
//! clasifica como [`GithubError::NotConfigured`]. Ese caso **no** es un fallo:
//! se traduce a un informe con `configured = false` y lista vacía.

use serde::Deserialize;

use crate::domain::alert::{Alert, AlertSource, Severity};

use super::client::GithubClient;
use super::error::GithubError;

/// Resultado de consultar las alertas de code scanning de un repositorio.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeScanningReport {
    /// Si el repositorio tiene un análisis de code scanning configurado.
    pub configured: bool,
    /// Alertas normalizadas; vacío si no hay análisis configurado.
    pub alerts: Vec<Alert>,
}

/// Recolecta y normaliza las alertas abiertas de code scanning.
///
/// # Errores
///
/// Propaga el [`GithubError`] de cualquier página salvo
/// [`GithubError::NotConfigured`] y [`GithubError::Disabled`], que se traducen a
/// un informe con `configured = false`.
pub async fn collect(
    client: &GithubClient,
    name_with_owner: &str,
) -> Result<CodeScanningReport, GithubError> {
    let path = format!("/repos/{name_with_owner}/code-scanning/alerts?state=open");

    match client.get_paginated::<RawAlert>(&path).await {
        Ok(crudas) => Ok(CodeScanningReport {
            configured: true,
            alerts: crudas.into_iter().map(RawAlert::normalize).collect(),
        }),
        // `Disabled` es el marcador que el cliente clasifica para un `403` de
        // capacidad deshabilitada (redacción de Dependabot). Se reutiliza aquí
        // porque tiene el **mismo significado operativo**: la capacidad no está
        // disponible en el repositorio, y el informe lo traduce a
        // `configured = false` sin abortar.
        Err(GithubError::NotConfigured | GithubError::Disabled) => Ok(CodeScanningReport {
            configured: false,
            alerts: Vec::new(),
        }),
        Err(otro) => Err(otro),
    }
}

/// Forma cruda de una alerta de code scanning.
#[derive(Debug, Deserialize)]
struct RawAlert {
    /// Número de la alerta dentro del repositorio.
    number: u64,
    #[serde(default)]
    rule: RawRule,
}

impl RawAlert {
    /// Normaliza la alerta al modelo único.
    fn normalize(self) -> Alert {
        Alert {
            source: AlertSource::Code,
            external_id: self.number.to_string(),
            ghsa_id: None,
            cve_id: None,
            severity: self
                .rule
                .security_severity_level
                .as_deref()
                .map(Severity::from_github),
            cvss: None,
            summary: self.rule.description,
            package: None,
            ecosystem: None,
            manifest_path: None,
            scope: None,
            vulnerable_range: None,
            first_patched: None,
            secret_type: None,
        }
    }
}

/// Regla que disparó la alerta.
#[derive(Debug, Default, Deserialize)]
struct RawRule {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    security_severity_level: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La normalización de code scanning toma severidad y resumen de la regla.
    #[test]
    fn normaliza_una_alerta_de_code_scanning() {
        let crudo = RawAlert {
            number: 3,
            rule: RawRule {
                description: Some("Uso de función insegura".to_string()),
                security_severity_level: Some("high".to_string()),
            },
        };
        let alerta = crudo.normalize();

        assert_eq!(alerta.source, AlertSource::Code);
        assert_eq!(alerta.external_id, "3");
        assert_eq!(alerta.severity, Some(Severity::High));
        assert_eq!(alerta.summary.as_deref(), Some("Uso de función insegura"));
        assert!(alerta.cvss.is_none());
    }
}
