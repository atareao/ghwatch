//! Recolección y normalización de las alertas de Dependabot.
//!
//! La recolección recorre todas las páginas con `per_page=100` (vía
//! [`GithubClient::get_paginated`]) y normaliza cada elemento al [`Alert`] único.
//! Un `403` de «Dependabot deshabilitado» se traduce a
//! [`DependabotState::Disabled`] y **no** es un fallo del escaneo; cualquier otro
//! error se propaga.

use serde::Deserialize;

use crate::domain::alert::{Alert, AlertSource, Severity};
use crate::domain::repo::DependabotState;

use super::client::GithubClient;
use super::datetime::parse_rfc3339;
use super::error::GithubError;

/// Resultado de consultar las alertas de Dependabot de un repositorio.
#[derive(Debug, Clone, PartialEq)]
pub struct DependabotReport {
    /// Estado de la capacidad en el repositorio.
    pub state: DependabotState,
    /// Alertas normalizadas; vacío si la capacidad está deshabilitada.
    pub alerts: Vec<Alert>,
}

/// Recolecta y normaliza las alertas abiertas de Dependabot de un repositorio.
///
/// # Errores
///
/// Propaga el [`GithubError`] de cualquier página salvo
/// [`GithubError::Disabled`], que se traduce a un informe vacío con estado
/// `disabled`.
pub async fn collect(
    client: &GithubClient,
    name_with_owner: &str,
) -> Result<DependabotReport, GithubError> {
    let path = format!("/repos/{name_with_owner}/dependabot/alerts?state=open");

    match client.get_paginated::<RawAlert>(&path).await {
        Ok(crudas) => Ok(DependabotReport {
            state: DependabotState::Enabled,
            alerts: crudas.into_iter().map(RawAlert::normalize).collect(),
        }),
        Err(GithubError::Disabled) => Ok(DependabotReport {
            state: DependabotState::Disabled,
            alerts: Vec::new(),
        }),
        Err(otro) => Err(otro),
    }
}

/// Forma cruda de una alerta de Dependabot.
#[derive(Debug, Deserialize)]
struct RawAlert {
    /// Número de la alerta dentro del repositorio.
    number: u64,
    /// Instante de creación de la alerta tal y como lo publica GitHub.
    ///
    /// Se guarda **crudo** y se normaliza aparte: una fecha presente pero
    /// ilegible no debe hacer fallar la deserialización de toda la respuesta.
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    dependency: RawDependency,
    #[serde(default)]
    security_advisory: RawAdvisory,
    #[serde(default)]
    security_vulnerability: RawVulnerability,
}

impl RawAlert {
    /// Normaliza la alerta al modelo único.
    fn normalize(self) -> Alert {
        // El paquete puede figurar tanto en la dependencia como en la
        // vulnerabilidad; se prefiere el de la vulnerabilidad, pero **campo a
        // campo**: un `name` (o `ecosystem`) ausente en la vulnerabilidad no
        // debe ocultar el de la dependencia.
        let package = self
            .security_vulnerability
            .package
            .as_ref()
            .and_then(|paquete| paquete.name.clone())
            .or_else(|| {
                self.dependency
                    .package
                    .as_ref()
                    .and_then(|paquete| paquete.name.clone())
            });
        let ecosystem = self
            .security_vulnerability
            .package
            .as_ref()
            .and_then(|paquete| paquete.ecosystem.clone())
            .or_else(|| {
                self.dependency
                    .package
                    .as_ref()
                    .and_then(|paquete| paquete.ecosystem.clone())
            });

        Alert {
            source: AlertSource::Dependabot,
            external_id: self.number.to_string(),
            ghsa_id: self.security_advisory.ghsa_id,
            cve_id: self.security_advisory.cve_id,
            severity: self
                .security_advisory
                .severity
                .as_deref()
                .map(Severity::from_github),
            cvss: self.security_advisory.cvss.and_then(|cvss| cvss.score),
            summary: self.security_advisory.summary,
            package,
            ecosystem,
            manifest_path: self.dependency.manifest_path,
            scope: self.dependency.scope,
            vulnerable_range: self.security_vulnerability.vulnerable_version_range,
            first_patched: self
                .security_vulnerability
                .first_patched_version
                .and_then(|parche| parche.identifier),
            secret_type: None,
            created_at: parse_rfc3339(self.created_at.as_deref()),
        }
    }
}

/// Dependencia afectada por la alerta.
#[derive(Debug, Default, Deserialize)]
struct RawDependency {
    #[serde(default)]
    package: Option<RawPackage>,
    #[serde(default)]
    manifest_path: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

/// Paquete afectado.
#[derive(Debug, Default, Deserialize)]
struct RawPackage {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    ecosystem: Option<String>,
}

/// Aviso de seguridad asociado.
#[derive(Debug, Default, Deserialize)]
struct RawAdvisory {
    #[serde(default)]
    ghsa_id: Option<String>,
    #[serde(default)]
    cve_id: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    cvss: Option<RawCvss>,
}

/// Puntuación CVSS del aviso.
#[derive(Debug, Default, Deserialize)]
struct RawCvss {
    #[serde(default)]
    score: Option<f64>,
}

/// Vulnerabilidad concreta de la dependencia.
#[derive(Debug, Default, Deserialize)]
struct RawVulnerability {
    #[serde(default)]
    package: Option<RawPackage>,
    #[serde(default)]
    vulnerable_version_range: Option<String>,
    #[serde(default)]
    first_patched_version: Option<RawPatch>,
}

/// Primera versión que corrige la vulnerabilidad.
#[derive(Debug, Default, Deserialize)]
struct RawPatch {
    #[serde(default)]
    identifier: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Alerta cruda con la dependencia y la vulnerabilidad indicadas.
    fn crudo(vulnerabilidad: RawPackage, dependencia: RawPackage) -> RawAlert {
        RawAlert {
            number: 1,
            created_at: None,
            dependency: RawDependency {
                package: Some(dependencia),
                manifest_path: None,
                scope: None,
            },
            security_advisory: RawAdvisory::default(),
            security_vulnerability: RawVulnerability {
                package: Some(vulnerabilidad),
                vulnerable_version_range: None,
                first_patched_version: None,
            },
        }
    }

    /// Un `name` ausente en la vulnerabilidad **no** oculta el `name` de la
    /// dependencia: el fallback es campo a campo.
    #[test]
    fn package_se_resuelve_campo_a_campo() {
        let alerta = crudo(
            RawPackage {
                name: None,
                ecosystem: None,
            },
            RawPackage {
                name: Some("django".to_string()),
                ecosystem: Some("pip".to_string()),
            },
        )
        .normalize();

        assert_eq!(alerta.package.as_deref(), Some("django"));
        assert_eq!(alerta.ecosystem.as_deref(), Some("pip"));
    }

    /// Cuando la vulnerabilidad sí aporta el paquete, ese manda sobre el de la
    /// dependencia.
    #[test]
    fn package_prefiere_el_de_la_vulnerabilidad() {
        let alerta = crudo(
            RawPackage {
                name: Some("django".to_string()),
                ecosystem: Some("pip".to_string()),
            },
            RawPackage {
                name: Some("otro".to_string()),
                ecosystem: Some("npm".to_string()),
            },
        )
        .normalize();

        assert_eq!(alerta.package.as_deref(), Some("django"));
        assert_eq!(alerta.ecosystem.as_deref(), Some("pip"));
    }
}
