//! Cliente HTTP de OSV.dev.
//!
//! Consulta `POST {base_url}/v1/querybatch` **sin credenciales**, repartiendo las
//! consultas en lotes de como máximo [`OsvConfig::batch_size`] (1000 por
//! defecto). La URL base es configurable para poder apuntar a un servidor falso
//! en los tests; en producción es `https://api.osv.dev`.
//!
//! La respuesta de cada lote trae un resultado por consulta, en el mismo orden
//! que la petición. Si OSV devuelve menos resultados de los esperados, los que
//! faltan se rellenan con un aviso vacío: el enriquecimiento nunca indexa fuera
//! de rango.
//!
//! # Alcance
//!
//! El enriquecimiento lee los avisos que acompañan a cada resultado del lote. El
//! contrato que se prueba aquí es el endpoint de lotes (una petición por cada
//! 1000 consultas); la consulta de detalle por identificador queda fuera de esta
//! fase.
//!
//! # Limitaciones conocidas
//!
//! - **`next_page_token`.** La respuesta de OSV puede incluir
//!   `next_page_token` para indicar que hay más resultados; se **ignora** a
//!   propósito. El enriquecimiento es complementario y best-effort: agotar la
//!   paginación de OSV multiplicaría las peticiones sin garantizar más datos
//!   útiles para la alerta.
//! - **Correlación sin versión.** GitHub no publica la versión instalada, así
//!   que la consulta va solo por paquete y OSV puede devolver varias
//!   vulnerabilidades. Cuando la alerta aporta `ghsa_id`, se prefiere la
//!   vulnerabilidad cuyo `id` coincide (ver
//!   [`QueryResult::aggregate`](QueryResult)); si ninguna coincide, el agregado
//!   mezcla los datos de todas ellas, que es la mejor aproximación disponible.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::cvss;
use super::model::{OsvAdvisory, OsvQuery};

/// Configuración del cliente OSV.
#[derive(Debug, Clone)]
pub struct OsvConfig {
    /// URL base del servicio (sin barra final).
    pub base_url: String,
    /// Número máximo de consultas por petición (OSV admite hasta 1000).
    pub batch_size: usize,
    /// Tiempo máximo de espera de cada petición.
    pub timeout: Duration,
}

impl Default for OsvConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.osv.dev".to_string(),
            batch_size: 1000,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Error tipado del cliente OSV.
#[derive(Debug, thiserror::Error)]
pub enum OsvError {
    /// Fallo de red, de tiempo de espera agotado o de decodificación.
    #[error("fallo al consultar OSV: {0}")]
    Request(#[from] reqwest::Error),
    /// OSV respondió con un estado que no es de éxito.
    #[error("OSV respondió con el estado {status}")]
    Status {
        /// Código de estado HTTP devuelto por OSV.
        status: u16,
    },
}

/// Cliente de OSV.dev.
#[derive(Debug, Clone)]
pub struct OsvClient {
    /// Cliente HTTP subyacente.
    http: reqwest::Client,
    /// Configuración del cliente.
    config: OsvConfig,
}

impl OsvClient {
    /// Construye el cliente a partir de su configuración.
    ///
    /// # Errores
    ///
    /// Devuelve [`OsvError`] si no se puede construir el cliente HTTP.
    pub fn new(config: OsvConfig) -> Result<Self, OsvError> {
        let http = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self { http, config })
    }

    /// Consulta OSV en lotes y devuelve un aviso por consulta, en orden.
    ///
    /// Las consultas se reparten en lotes de [`OsvConfig::batch_size`]: 1500
    /// consultas producen exactamente dos peticiones. La lista devuelta tiene
    /// siempre la misma longitud que `queries`.
    ///
    /// # Errores
    ///
    /// Devuelve [`OsvError`] si una petición falla o si OSV responde con un
    /// estado que no es de éxito. No hay reintentos: el enriquecimiento es
    /// complementario y su fallo no debe retrasar el escaneo.
    pub async fn query_batch(&self, queries: &[OsvQuery]) -> Result<Vec<OsvAdvisory>, OsvError> {
        let mut advisories = Vec::with_capacity(queries.len());
        let tamano = self.config.batch_size.max(1);
        let url = format!(
            "{}/v1/querybatch",
            self.config.base_url.trim_end_matches('/')
        );

        for lote in queries.chunks(tamano) {
            let peticion = BatchRequest {
                queries: lote.iter().map(QueryItem::from).collect(),
            };

            let respuesta = self.http.post(&url).json(&peticion).send().await?;
            let estado = respuesta.status();
            if !estado.is_success() {
                return Err(OsvError::Status {
                    status: estado.as_u16(),
                });
            }

            let decodificada: BatchResponse = respuesta.json().await?;
            let mut del_lote: Vec<OsvAdvisory> = Vec::with_capacity(decodificada.results.len());
            for (indice, resultado) in decodificada.results.into_iter().enumerate() {
                let esperado = lote
                    .get(indice)
                    .and_then(|consulta| consulta.advisory_id.as_deref());
                del_lote.push(resultado.aggregate(esperado));
            }
            del_lote.resize(lote.len(), OsvAdvisory::default());
            advisories.extend(del_lote);
        }

        advisories.truncate(queries.len());
        Ok(advisories)
    }
}

/// Cuerpo de la petición de lote.
#[derive(Debug, Serialize)]
struct BatchRequest<'a> {
    /// Consultas del lote.
    queries: Vec<QueryItem<'a>>,
}

/// Consulta individual dentro del lote.
#[derive(Debug, Serialize)]
struct QueryItem<'a> {
    /// Paquete consultado.
    package: PackageRef<'a>,
    /// Versión consultada; se omite si no consta.
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<&'a str>,
}

impl<'a> From<&'a OsvQuery> for QueryItem<'a> {
    fn from(consulta: &'a OsvQuery) -> Self {
        Self {
            package: PackageRef {
                name: &consulta.package,
                ecosystem: &consulta.ecosystem,
            },
            version: consulta.version.as_deref(),
        }
    }
}

/// Referencia a un paquete en la petición.
#[derive(Debug, Serialize)]
struct PackageRef<'a> {
    /// Nombre del paquete.
    name: &'a str,
    /// Ecosistema en la nomenclatura de OSV.
    ecosystem: &'a str,
}

/// Respuesta de un lote.
#[derive(Debug, Deserialize)]
struct BatchResponse {
    /// Resultados, uno por consulta y en el mismo orden.
    #[serde(default)]
    results: Vec<QueryResult>,
}

/// Resultado de una consulta.
#[derive(Debug, Default, Deserialize)]
struct QueryResult {
    /// Vulnerabilidades que afectan a la consulta.
    #[serde(default)]
    vulns: Vec<RawVuln>,
}

impl QueryResult {
    /// Agrega los avisos del resultado en un único [`OsvAdvisory`].
    ///
    /// Cuando se conoce el identificador del aviso esperado (el `ghsa_id` de la
    /// alerta), se **prefiere** la vulnerabilidad cuyo `id` coincide: así no se
    /// mezclan el CVSS, el alias CVE o la versión corregida de otra
    /// vulnerabilidad del mismo paquete. Si ninguna coincide —o no hay pista—,
    /// el agregado es determinista sobre el conjunto: el primer alias CVE
    /// encontrado, la puntuación CVSS más alta y la primera versión corregida
    /// que aparece. Esa mezcla es una **limitación conocida** cuando OSV
    /// devuelve varias vulnerabilidades sin correlación posible.
    fn aggregate(self, expected_id: Option<&str>) -> OsvAdvisory {
        let mut vulns = self.vulns;

        if let Some(esperado) = expected_id
            && vulns
                .iter()
                .any(|vuln| vuln.id.as_deref() == Some(esperado))
        {
            vulns.retain(|vuln| vuln.id.as_deref() == Some(esperado));
        }

        let mut advisory = OsvAdvisory::default();

        for vuln in vulns {
            if advisory.cve_id.is_none() {
                advisory.cve_id = vuln
                    .aliases
                    .into_iter()
                    .find(|alias| alias.to_ascii_uppercase().starts_with("CVE-"));
            }

            for severidad in &vuln.severity {
                if let Some(score) = severidad.score.as_deref().and_then(cvss::parse_score) {
                    advisory.cvss = Some(advisory.cvss.map_or(score, |actual| actual.max(score)));
                }
            }

            if advisory.fixed_version.is_none() {
                advisory.fixed_version = vuln
                    .affected
                    .into_iter()
                    .flat_map(|afectado| afectado.ranges)
                    .flat_map(|rango| rango.events)
                    .find_map(|evento| evento.fixed);
            }
        }

        advisory
    }
}

/// Vulnerabilidad cruda devuelta por OSV.
#[derive(Debug, Default, Deserialize)]
struct RawVuln {
    /// Identificador de la vulnerabilidad (habitualmente el propio GHSA).
    #[serde(default)]
    id: Option<String>,
    /// Alias de la vulnerabilidad (incluye los CVE).
    #[serde(default)]
    aliases: Vec<String>,
    /// Severidades publicadas (vectores CVSS).
    #[serde(default)]
    severity: Vec<RawSeverity>,
    /// Paquetes y rangos afectados.
    #[serde(default)]
    affected: Vec<RawAffected>,
}

/// Severidad publicada por OSV.
#[derive(Debug, Default, Deserialize)]
struct RawSeverity {
    /// Puntuación: normalmente un vector CVSS, a veces un número.
    #[serde(default)]
    score: Option<String>,
}

/// Paquete afectado por una vulnerabilidad.
#[derive(Debug, Default, Deserialize)]
struct RawAffected {
    /// Rangos de versiones afectadas.
    #[serde(default)]
    ranges: Vec<RawRange>,
}

/// Rango de versiones afectadas.
#[derive(Debug, Default, Deserialize)]
struct RawRange {
    /// Eventos del rango (`introduced`, `fixed`…).
    #[serde(default)]
    events: Vec<RawEvent>,
}

/// Evento de un rango de versiones.
#[derive(Debug, Default, Deserialize)]
struct RawEvent {
    /// Versión en la que se corrige la vulnerabilidad.
    #[serde(default)]
    fixed: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un resultado con alias, CVSS y versión corregida se agrega entero.
    #[test]
    fn agrega_un_aviso_completo() {
        let crudo = QueryResult {
            vulns: vec![RawVuln {
                id: Some("GHSA-xxxx-yyyy-zzzz".to_string()),
                aliases: vec!["GHSA-xxxx".to_string(), "CVE-2021-1234".to_string()],
                severity: vec![RawSeverity {
                    score: Some("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H".to_string()),
                }],
                affected: vec![RawAffected {
                    ranges: vec![RawRange {
                        events: vec![
                            RawEvent { fixed: None },
                            RawEvent {
                                fixed: Some("3.0.2".to_string()),
                            },
                        ],
                    }],
                }],
            }],
        };

        let advisory = crudo.aggregate(None);
        assert_eq!(advisory.cve_id.as_deref(), Some("CVE-2021-1234"));
        assert_eq!(advisory.cvss, Some(9.8));
        assert_eq!(advisory.fixed_version.as_deref(), Some("3.0.2"));
    }

    /// El agregado toma el CVSS más alto y el primer alias CVE.
    #[test]
    fn agrega_varios_avisos_de_forma_determinista() {
        let crudo = QueryResult {
            vulns: vec![
                RawVuln {
                    id: None,
                    aliases: vec!["CVE-2020-0001".to_string()],
                    severity: vec![RawSeverity {
                        score: Some("5.0".to_string()),
                    }],
                    affected: vec![],
                },
                RawVuln {
                    id: None,
                    aliases: vec!["CVE-2020-0002".to_string()],
                    severity: vec![RawSeverity {
                        score: Some("8.0".to_string()),
                    }],
                    affected: vec![],
                },
            ],
        };

        let advisory = crudo.aggregate(None);
        assert_eq!(advisory.cve_id.as_deref(), Some("CVE-2020-0001"));
        assert_eq!(advisory.cvss, Some(8.0));
    }

    /// I2 — Con una pista de correlación, solo se agrega la vulnerabilidad cuyo
    /// `id` coincide.
    #[test]
    fn la_correlacion_restringe_el_agregado_a_la_vulnerabilidad_que_coincide() {
        let crudo = QueryResult {
            vulns: vec![
                RawVuln {
                    id: Some("GHSA-otra-0000-0000".to_string()),
                    aliases: vec!["CVE-2020-0001".to_string()],
                    severity: vec![RawSeverity {
                        score: Some("9.8".to_string()),
                    }],
                    affected: vec![RawAffected {
                        ranges: vec![RawRange {
                            events: vec![RawEvent {
                                fixed: Some("1.0.0".to_string()),
                            }],
                        }],
                    }],
                },
                RawVuln {
                    id: Some("GHSA-buscada-0000".to_string()),
                    aliases: vec!["CVE-2020-0002".to_string()],
                    severity: vec![RawSeverity {
                        score: Some("5.0".to_string()),
                    }],
                    affected: vec![RawAffected {
                        ranges: vec![RawRange {
                            events: vec![RawEvent {
                                fixed: Some("2.0.0".to_string()),
                            }],
                        }],
                    }],
                },
            ],
        };

        let advisory = crudo.aggregate(Some("GHSA-buscada-0000"));
        assert_eq!(advisory.cve_id.as_deref(), Some("CVE-2020-0002"));
        assert_eq!(advisory.cvss, Some(5.0));
        assert_eq!(advisory.fixed_version.as_deref(), Some("2.0.0"));
    }

    /// I2 — Si ninguna vulnerabilidad coincide con la pista, se mantiene el
    /// agregado determinista sobre el conjunto (limitación conocida).
    #[test]
    fn sin_coincidencia_se_agrega_el_conjunto() {
        let crudo = QueryResult {
            vulns: vec![RawVuln {
                id: Some("GHSA-otra-0000-0000".to_string()),
                aliases: vec!["CVE-2020-0001".to_string()],
                severity: vec![],
                affected: vec![],
            }],
        };

        let advisory = crudo.aggregate(Some("GHSA-buscada-0000"));
        assert_eq!(advisory.cve_id.as_deref(), Some("CVE-2020-0001"));
    }

    /// Un resultado sin vulnerabilidades devuelve un aviso vacío.
    #[test]
    fn un_resultado_vacio_no_aporta_nada() {
        assert_eq!(
            QueryResult::default().aggregate(None),
            OsvAdvisory::default()
        );
    }
}
