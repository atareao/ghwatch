//! Fusión de los datos de OSV sobre las alertas.
//!
//! El enriquecimiento **completa sin sobreescribir**: un dato que GitHub ya
//! aporta (CVSS, alias CVE o versión corregida) no se toca. Solo se rellenan los
//! huecos.
//!
//! Es **best-effort**: si OSV falla, [`OsvClient::enrich`] devuelve el error y
//! las alertas quedan tal cual; quien orquesta decide si avisar (el escaneo usa
//! [`crate::scan::scan_with_enrichment`], que lo registra como aviso y continúa).

use crate::domain::alert::Alert;

use super::client::{OsvClient, OsvError};
use super::ecosystem::map_ecosystem;
use super::model::{OsvAdvisory, OsvQuery};

/// Resultado de enriquecer un conjunto de alertas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnrichmentOutcome {
    /// Consultas enviadas a OSV.
    pub queried: usize,
    /// Alertas a las que se les completó algún dato.
    pub enriched: usize,
    /// Alertas omitidas (ecosistema sin equivalencia o sin paquete).
    pub skipped: usize,
}

/// Construye la consulta OSV de una alerta, si es posible.
///
/// Devuelve `None` cuando la alerta no tiene paquete o su ecosistema no tiene
/// equivalencia en la tabla cerrada. GitHub no publica la versión instalada, así
/// que la consulta va solo por paquete.
#[must_use]
pub fn query_for_alert(alert: &Alert) -> Option<OsvQuery> {
    let ecosystem = map_ecosystem(alert.ecosystem.as_deref()?)?;
    let package = alert.package.clone()?;

    Some(OsvQuery {
        ecosystem: ecosystem.to_string(),
        package,
        version: None,
        advisory_id: alert.ghsa_id.clone(),
    })
}

/// Aplica un aviso de OSV sobre una alerta **sin sobreescribir** lo de GitHub.
///
/// Un `cvss` a cero cuenta como ausente: una puntuación nula no es un dato útil
/// y se completa con la de OSV. En sentido inverso, un aviso cuyo único dato sea
/// un `cvss` nulo **no** se aplica: no aporta información. Devuelve `true` si la
/// alerta cambió.
pub fn apply_advisory(alert: &mut Alert, advisory: &OsvAdvisory) -> bool {
    let mut cambio = false;

    if alert.cve_id.is_none()
        && let Some(cve) = &advisory.cve_id
    {
        alert.cve_id = Some(cve.clone());
        cambio = true;
    }

    if alert.cvss.is_none_or(|score| score == 0.0)
        && let Some(cvss) = advisory.cvss
        && cvss > 0.0
    {
        alert.cvss = Some(cvss);
        cambio = true;
    }

    if alert.first_patched.is_none()
        && let Some(fixed) = &advisory.fixed_version
    {
        alert.first_patched = Some(fixed.clone());
        cambio = true;
    }

    cambio
}

impl OsvClient {
    /// Enriquece las alertas **in situ** con los datos de OSV.
    ///
    /// Construye una consulta por alerta con ecosistema equivalente, las envía en
    /// lotes y aplica los avisos sin sobreescribir. Las alertas cuyo ecosistema
    /// no tiene equivalencia se omiten sin error y se registran en modo
    /// depuración.
    ///
    /// # Errores
    ///
    /// Si la consulta a OSV falla, devuelve [`OsvError`] **sin haber tocado**
    /// ninguna alerta: la fusión solo se aplica cuando la respuesta llega
    /// completa.
    pub async fn enrich<'a, I>(&self, alerts: I) -> Result<EnrichmentOutcome, OsvError>
    where
        I: IntoIterator<Item = &'a mut Alert>,
    {
        let mut alerts: Vec<&mut Alert> = alerts.into_iter().collect();
        let mut outcome = EnrichmentOutcome::default();

        let mut indices: Vec<usize> = Vec::new();
        let mut queries: Vec<OsvQuery> = Vec::new();

        for (indice, alerta) in alerts.iter().enumerate() {
            match query_for_alert(alerta) {
                Some(consulta) => {
                    queries.push(consulta);
                    indices.push(indice);
                }
                None => {
                    outcome.skipped += 1;
                    tracing::debug!(
                        alerta = %alerta.external_id,
                        ecosistema = ?alerta.ecosystem,
                        "sin equivalencia OSV: la alerta se omite del enriquecimiento"
                    );
                }
            }
        }

        outcome.queried = queries.len();
        if queries.is_empty() {
            return Ok(outcome);
        }

        let advisories = self.query_batch(&queries).await?;

        for (advisory, indice) in advisories.iter().zip(indices) {
            if apply_advisory(alerts[indice], advisory) {
                outcome.enriched += 1;
            }
        }

        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::alert::{AlertSource, Severity};

    /// Alerta mínima con el ecosistema y el paquete indicados.
    fn alerta(ecosystem: Option<&str>, package: Option<&str>) -> Alert {
        Alert {
            source: AlertSource::Dependabot,
            external_id: "1".to_string(),
            ghsa_id: None,
            cve_id: None,
            severity: Some(Severity::High),
            cvss: None,
            summary: None,
            package: package.map(str::to_string),
            ecosystem: ecosystem.map(str::to_string),
            manifest_path: None,
            scope: None,
            vulnerable_range: None,
            first_patched: None,
            secret_type: None,
            created_at: None,
        }
    }

    /// Un ecosistema conocido y un paquete producen consulta; uno desconocido no.
    #[test]
    fn la_consulta_depende_del_mapeo() {
        assert_eq!(
            query_for_alert(&alerta(Some("pip"), Some("django"))),
            Some(OsvQuery {
                ecosystem: "PyPI".to_string(),
                package: "django".to_string(),
                version: None,
                advisory_id: None,
            })
        );
        assert_eq!(query_for_alert(&alerta(Some("swift"), Some("x"))), None);
        assert_eq!(query_for_alert(&alerta(Some("pip"), None)), None);
    }

    /// La fusión no sobreescribe lo que ya consta y solo rellena los huecos.
    #[test]
    fn la_fusion_completa_sin_sobreescribir() {
        let mut alerta = alerta(Some("pip"), Some("django"));
        alerta.cvss = Some(7.5);
        alerta.first_patched = Some("3.0.1".to_string());

        let advisory = OsvAdvisory {
            cve_id: Some("CVE-2021-1234".to_string()),
            cvss: Some(9.8),
            fixed_version: Some("3.0.2".to_string()),
        };

        assert!(apply_advisory(&mut alerta, &advisory));
        assert_eq!(alerta.cve_id.as_deref(), Some("CVE-2021-1234"));
        assert_eq!(alerta.cvss, Some(7.5));
        assert_eq!(alerta.first_patched.as_deref(), Some("3.0.1"));
    }

    /// Un CVSS a cero cuenta como ausente y se completa.
    #[test]
    fn un_cvss_cero_se_completa() {
        let mut alerta = alerta(Some("pip"), Some("django"));
        alerta.cvss = Some(0.0);

        let advisory = OsvAdvisory {
            cvss: Some(9.8),
            ..OsvAdvisory::default()
        };

        assert!(apply_advisory(&mut alerta, &advisory));
        assert_eq!(alerta.cvss, Some(9.8));
    }

    /// Un aviso vacío no cambia nada.
    #[test]
    fn un_aviso_vacio_no_cambia_la_alerta() {
        let mut alerta = alerta(Some("pip"), Some("django"));
        assert!(!apply_advisory(&mut alerta, &OsvAdvisory::default()));
    }
}
