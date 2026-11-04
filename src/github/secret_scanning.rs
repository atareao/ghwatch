//! Recolección y normalización de las alertas de secret scanning.
//!
//! Recorre todas las páginas con `per_page=100` y normaliza cada alerta al
//! [`Alert`] único, con `source = secret`. Una alerta de secreto **no** aporta
//! `cvss`, `package` ni identificadores GHSA/CVE: esos campos quedan a `None`
//! sin que ello sea un error. Un `404` (repo público o capacidad no disponible),
//! un `404` con `no analysis found` y un `403` de capacidad deshabilitada se
//! toleran como lista vacía: no son fallos del escaneo, pero **sí** marcan la
//! fuente como **no observable** (el informe [`SecretScanningReport`] lleva esa
//! marca) para que la persistencia no declare resueltos secretos que solo
//! dejaron de poder observarse.

use serde::Deserialize;

use crate::domain::alert::{Alert, AlertSource};

use super::client::GithubClient;
use super::error::GithubError;

/// Resultado de consultar las alertas de secret scanning de un repositorio.
#[derive(Debug, Clone, PartialEq)]
pub struct SecretScanningReport {
    /// Si la fuente estaba disponible para observarse en el repositorio.
    ///
    /// `false` cuando la API respondió `404`/`403`/`no analysis found`: la lista
    /// vacía entonces significa «no se pudo mirar», no «no hay secretos».
    pub observable: bool,
    /// Alertas normalizadas; vacío si la fuente no estaba observable.
    pub alerts: Vec<Alert>,
}

/// Recolecta y normaliza las alertas abiertas de secret scanning.
///
/// # Errores
///
/// Propaga el [`GithubError`] de cualquier página salvo cuando la capacidad no
/// está disponible en el repositorio: [`GithubError::NotFound`] (un `404`
/// genérico, p. ej. un repo público), [`GithubError::NotConfigured`] y
/// [`GithubError::Disabled`] se traducen a un informe con `observable = false` y
/// lista vacía. GitHub documenta el `404` para
/// `GET /repos/{owner}/{repo}/secret-scanning/alerts` cuando el repositorio es
/// público o la capacidad está desactivada, así que no debe contabilizarse como
/// fallo.
pub async fn collect(
    client: &GithubClient,
    name_with_owner: &str,
) -> Result<SecretScanningReport, GithubError> {
    let path = format!("/repos/{name_with_owner}/secret-scanning/alerts?state=open");

    match client.get_paginated::<RawAlert>(&path).await {
        Ok(crudas) => Ok(SecretScanningReport {
            observable: true,
            alerts: crudas.into_iter().map(RawAlert::normalize).collect(),
        }),
        Err(GithubError::NotFound | GithubError::NotConfigured | GithubError::Disabled) => {
            Ok(SecretScanningReport {
                observable: false,
                alerts: Vec::new(),
            })
        }
        Err(otro) => Err(otro),
    }
}

/// Forma cruda de una alerta de secret scanning.
#[derive(Debug, Deserialize)]
struct RawAlert {
    /// Número de la alerta dentro del repositorio.
    number: u64,
    #[serde(default)]
    secret_type: Option<String>,
    #[serde(default)]
    secret_type_display_name: Option<String>,
}

impl RawAlert {
    /// Normaliza la alerta al modelo único.
    fn normalize(self) -> Alert {
        Alert {
            source: AlertSource::Secret,
            external_id: self.number.to_string(),
            ghsa_id: None,
            cve_id: None,
            severity: None,
            cvss: None,
            summary: self.secret_type_display_name,
            package: None,
            ecosystem: None,
            manifest_path: None,
            scope: None,
            vulnerable_range: None,
            first_patched: None,
            secret_type: self.secret_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La normalización de un secreto conserva el tipo y deja el `cvss` vacío.
    #[test]
    fn normaliza_un_secreto_sin_cvss() {
        let crudo = RawAlert {
            number: 7,
            secret_type: Some("adafruit_io_key".to_string()),
            secret_type_display_name: Some("Adafruit IO Key".to_string()),
        };
        let alerta = crudo.normalize();

        assert_eq!(alerta.source, AlertSource::Secret);
        assert_eq!(alerta.external_id, "7");
        assert_eq!(alerta.secret_type.as_deref(), Some("adafruit_io_key"));
        assert!(alerta.cvss.is_none());
        assert!(alerta.package.is_none());
    }
}
