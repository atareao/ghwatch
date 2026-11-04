//! Tipos de datos del enriquecimiento OSV.
//!
//! Son tipos puros, sin IO: la consulta ([`OsvQuery`]) y el aviso agregado
//! ([`OsvAdvisory`]) que OSV aporta para completar una alerta.

/// Consulta a OSV de un paquete concreto.
///
/// El `ecosystem` es el nombre que espera OSV (ya traducido con
/// [`map_ecosystem`](super::ecosystem::map_ecosystem)); la `version` es opcional
/// porque GitHub no siempre publica la versión instalada y OSV admite consultas
/// solo por paquete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsvQuery {
    /// Ecosistema en la nomenclatura de OSV (`PyPI`, `npm`, `crates.io`…).
    pub ecosystem: String,
    /// Nombre del paquete afectado.
    pub package: String,
    /// Versión instalada, si consta.
    pub version: Option<String>,
    /// Identificador del aviso esperado (el `ghsa_id` de la alerta), si consta.
    ///
    /// Es una **pista de correlación local**: no se envía a OSV. Cuando la
    /// respuesta trae varias vulnerabilidades para el mismo paquete, permite
    /// elegir la que corresponde a la alerta en lugar de mezclar sus datos.
    pub advisory_id: Option<String>,
}

impl OsvQuery {
    /// Construye una consulta a OSV sin pista de correlación.
    #[must_use]
    pub fn new(
        ecosystem: impl Into<String>,
        package: impl Into<String>,
        version: Option<String>,
    ) -> Self {
        Self {
            ecosystem: ecosystem.into(),
            package: package.into(),
            version,
            advisory_id: None,
        }
    }
}

/// Datos que OSV puede aportar a una alerta.
///
/// El enriquecimiento solo rellena lo que GitHub **no** aporta: un alias CVE, la
/// puntuación CVSS o la versión que corrige la vulnerabilidad.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OsvAdvisory {
    /// Alias CVE del aviso, si lo hay.
    pub cve_id: Option<String>,
    /// Puntuación CVSS base, si se puede interpretar.
    pub cvss: Option<f64>,
    /// Primera versión que corrige la vulnerabilidad, si consta.
    pub fixed_version: Option<String>,
}
