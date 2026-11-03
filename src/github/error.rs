//! Errores del cliente HTTP de la API de GitHub.
//!
//! Ninguna variante transporta la credencial: el token solo se usa al construir
//! la cabecera `Authorization` y jamás se copia a un mensaje de error. Por eso
//! los errores pueden mostrarse al usuario o registrarse sin riesgo de filtrar
//! el secreto.

use std::time::Duration;

use thiserror::Error;

/// Marca que GitHub devuelve en el cuerpo de un `404` cuando la
/// capacidad consultada no está configurada (por ejemplo, CodeQL sin análisis).
pub const NOT_CONFIGURED_MARKER: &str = "no analysis found";

/// Marca que GitHub devuelve en el cuerpo de un `403` cuando los avisos de
/// Dependabot están deshabilitados en el repositorio.
///
/// La redacción literal es
/// `Dependabot alerts are disabled for this repository.`; se compara la parte
/// estable y se acepta cualquier capacidad por el sufijo
/// `are disabled for this repository`.
pub const DISABLED_MARKER: &str = "are disabled for this repository";

/// Longitud máxima del cuerpo que se conserva en [`GithubError::Http`].
///
/// Acota la memoria y evita arrastrar respuestas enormes en un mensaje de
/// error, sin perder la parte útil para diagnosticar.
const MAX_BODY_CHARS: usize = 4096;

/// Error devuelto por el cliente HTTP de GitHub.
#[derive(Debug, Clone, Error)]
pub enum GithubError {
    /// La API rechazó la credencial (`401`).
    #[error("la API de GitHub rechazó la credencial (401)")]
    Unauthorized,

    /// El recurso solicitado no existe (`404` genérico).
    #[error("la API de GitHub devolvió 404: recurso no encontrado")]
    NotFound,

    /// La capacidad consultada no está configurada en el repositorio
    /// (`404` con `no analysis found`). No es un fallo del escaneo.
    #[error(
        "la capacidad consultada no está configurada en el repositorio (404: no analysis found)"
    )]
    NotConfigured,

    /// Se alcanzó el límite de peticiones (`403` de límite secundario).
    ///
    /// El campo `retry_after` recoge la espera indicada por el servidor en la
    /// cabecera `Retry-After`, si estaba presente. No se imprime en el mensaje.
    #[error("se alcanzó el límite de peticiones de la API de GitHub")]
    RateLimited {
        /// Espera indicada por el servidor, si la hubo.
        retry_after: Option<Duration>,
    },

    /// La capacidad consultada está deshabilitada en el repositorio (`403`
    /// con el cuerpo que GitHub usa para `dependabot/alerts`).
    ///
    /// No es un fallo del escaneo: el repositorio simplemente no tiene activada
    /// la capacidad. Se distingue de [`GithubError::Http`] por el cuerpo.
    #[error("la capacidad consultada está deshabilitada en el repositorio (403)")]
    Disabled,

    /// Respuesta de error no contemplada más arriba.
    ///
    /// Cubre, entre otras, el `403` sin `Retry-After` y con un cuerpo que no es
    /// el de «capacidad deshabilitada».
    #[error("respuesta inesperada de la API de GitHub: HTTP {status}")]
    Http {
        /// Código de estado HTTP.
        status: u16,
        /// Cuerpo de la respuesta, acotado a [`MAX_BODY_CHARS`].
        body: String,
    },

    /// La paginación por `Link` no termina de forma sana.
    ///
    /// Se emite al superar [`crate::github::GithubConfig::max_pages`] o cuando
    /// el servidor vuelve a ofrecer una URL ya visitada (bucle). Evita que un
    /// `Link` mal formado o cíclico cuelgue el escaneo.
    #[error("la paginación por `Link` no es finita (posible bucle): {pages} páginas recorridas")]
    PaginationLimit {
        /// Páginas recorridas antes de abortar.
        pages: usize,
    },

    /// Error de red, de transporte o de decodificación del cuerpo.
    ///
    /// El mensaje procede de `reqwest` e incluye la URL, nunca las cabeceras:
    /// no puede contener el token.
    #[error("error de transporte con la API de GitHub: {0}")]
    Transport(String),
}

impl GithubError {
    /// Indica si el error representa un límite de peticiones.
    #[must_use]
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited { .. })
    }

    /// Indica si el error representa una capacidad deshabilitada en el
    /// repositorio.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }

    /// Indica si el error es el corte de seguridad de la paginación.
    #[must_use]
    pub fn is_pagination_limit(&self) -> bool {
        matches!(self, Self::PaginationLimit { .. })
    }
}

/// Indica si el cuerpo de un `403` corresponde a «capacidad deshabilitada».
///
/// La comparación es insensible a mayúsculas y reconoce la redacción literal de
/// GitHub (`Dependabot alerts are disabled for this repository.`). Un `403` sin
/// esta marca sigue siendo [`GithubError::Http`], para no tragarse otros `403`.
#[must_use]
pub fn is_disabled_body(body: &str) -> bool {
    body.to_ascii_lowercase().contains(DISABLED_MARKER)
}

/// Clasifica una respuesta de la API de GitHub en un error tipado.
///
/// Devuelve `Ok(())` **solo** para respuestas de éxito (`2xx`). Un `3xx` no es
/// éxito: si `reqwest` no lo siguió (por ejemplo, un `304` sin `Location`), el
/// cuerpo no es el JSON esperado y se devuelve como [`GithubError::Http`] en
/// lugar de intentar decodificarlo.
///
/// `retry_after` es la espera ya extraída de la cabecera `Retry-After`: un
/// `403` **con** esa cabecera es un límite secundario
/// ([`GithubError::RateLimited`]); un `403` **sin** ella es
/// [`GithubError::Disabled`] si su cuerpo lleva la marca de capacidad
/// deshabilitada, y [`GithubError::Http`] con `status = 403` en caso contrario.
///
/// # Errores
///
/// - [`GithubError::Unauthorized`] para `401`.
/// - [`GithubError::RateLimited`] para `403` con `Retry-After`.
/// - [`GithubError::Disabled`] para `403` con el cuerpo de capacidad
///   deshabilitada y sin `Retry-After`.
/// - [`GithubError::NotConfigured`] para `404` con `no analysis found`.
/// - [`GithubError::NotFound`] para el resto de `404`.
/// - [`GithubError::Http`] para cualquier otro `status` que no sea `2xx`.
pub fn classify_response(
    status: u16,
    body: &str,
    retry_after: Option<Duration>,
) -> Result<(), GithubError> {
    match status {
        401 => Err(GithubError::Unauthorized),
        403 => match retry_after {
            Some(retry_after) => Err(GithubError::RateLimited {
                retry_after: Some(retry_after),
            }),
            None if is_disabled_body(body) => Err(GithubError::Disabled),
            None => Err(GithubError::Http {
                status,
                body: truncate_body(body),
            }),
        },
        404 if body.contains(NOT_CONFIGURED_MARKER) => Err(GithubError::NotConfigured),
        404 => Err(GithubError::NotFound),
        200..=299 => Ok(()),
        _ => Err(GithubError::Http {
            status,
            body: truncate_body(body),
        }),
    }
}

/// Recorta el cuerpo a [`MAX_BODY_CHARS`] caracteres, respetando los límites de
/// carácter para no partir un carácter UTF-8 por la mitad.
fn truncate_body(body: &str) -> String {
    if body.chars().count() <= MAX_BODY_CHARS {
        return body.to_string();
    }
    body.chars().take(MAX_BODY_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un estado de éxito no produce error.
    #[test]
    fn classify_response_2xx_es_ok() {
        assert!(classify_response(200, "{}", None).is_ok());
    }

    /// Una redirección que `reqwest` no sigue (por ejemplo, un `304` sin
    /// `Location`) **no** es un éxito: se trata como `Http` para no intentar
    /// decodificar después un cuerpo que no es el JSON esperado.
    #[test]
    fn classify_response_3xx_es_http() {
        let err = classify_response(304, "", None).expect_err("un 304 no es un éxito");
        match err {
            GithubError::Http { status, .. } => assert_eq!(status, 304),
            otro => panic!("se esperaba Http, no: {otro:?}"),
        }
    }

    /// Un `302` tampoco se considera éxito por sí mismo.
    #[test]
    fn classify_response_302_es_http() {
        let err = classify_response(302, "", None).expect_err("un 302 no es un éxito");
        match err {
            GithubError::Http { status, .. } => assert_eq!(status, 302),
            otro => panic!("se esperaba Http, no: {otro:?}"),
        }
    }

    /// Un `401` es `Unauthorized`.
    #[test]
    fn classify_response_401_es_unauthorized() {
        let err = classify_response(401, "", None).expect_err("401 debe fallar");
        assert!(matches!(err, GithubError::Unauthorized));
    }

    /// Un `403` con `Retry-After` es `RateLimited` y conserva la espera.
    ///
    /// El límite tiene prioridad sobre la marca de capacidad deshabilitada: si
    /// GitHub acompaña un `403` con `Retry-After`, es un límite.
    #[test]
    fn classify_response_403_con_retry_after_es_rate_limited() {
        let err = classify_response(403, "{}", Some(Duration::from_secs(30)))
            .expect_err("403 con Retry-After debe fallar");
        match err {
            GithubError::RateLimited { retry_after } => {
                assert_eq!(retry_after, Some(Duration::from_secs(30)));
            }
            otro => panic!("se esperaba RateLimited, no: {otro:?}"),
        }
    }

    /// Un `403` **con** `Retry-After` y el cuerpo de «deshabilitado» sigue siendo
    /// un límite: la cabecera manda.
    #[test]
    fn classify_response_403_con_retry_after_y_cuerpo_disabled_es_rate_limited() {
        let err = classify_response(
            403,
            "{\"message\":\"Dependabot alerts are disabled for this repository.\"}",
            Some(Duration::from_secs(5)),
        )
        .expect_err("403 con Retry-After debe fallar");
        assert!(matches!(err, GithubError::RateLimited { .. }));
    }

    /// Un `403` sin `Retry-After` cuyo cuerpo es el literal de Dependabot es
    /// `Disabled`.
    #[test]
    fn classify_response_403_cuerpo_deshabilitado_es_disabled() {
        let cuerpo = "{\"message\":\"Dependabot alerts are disabled for this repository.\",\"documentation_url\":\"https://docs.github.com/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository\",\"status\":\"403\"}";
        let err = classify_response(403, cuerpo, None).expect_err("403 debe fallar");
        assert!(matches!(err, GithubError::Disabled), "se esperaba Disabled");
    }

    /// Un `403` sin `Retry-After` y con otro cuerpo no es límite ni
    /// «deshabilitado»: es un `Http` con estado 403.
    #[test]
    fn classify_response_403_otro_cuerpo_es_http() {
        let err = classify_response(403, "{\"message\":\"Resource not accessible\"}", None)
            .expect_err("403 sin Retry-After debe fallar");
        match err {
            GithubError::Http { status, .. } => assert_eq!(status, 403),
            otro => panic!("se esperaba Http, no: {otro:?}"),
        }
    }

    /// `is_disabled_body` reconoce el literal de GitHub (insensible a
    /// mayúsculas) y no se dispara con cuerpos ajenos.
    #[test]
    fn is_disabled_body_reconoce_el_literal() {
        assert!(is_disabled_body(
            "{\"message\":\"Dependabot alerts are disabled for this repository.\"}"
        ));
        assert!(is_disabled_body(
            "{\"message\":\"DEPENDABOT ALERTS ARE DISABLED FOR THIS REPOSITORY.\"}"
        ));
        assert!(!is_disabled_body(
            "{\"message\":\"Resource not accessible\"}"
        ));
        assert!(!is_disabled_body(""));
    }

    /// Un `404` con la marca de GitHub es `NotConfigured`.
    #[test]
    fn classify_response_404_no_analysis_found_es_not_configured() {
        let err = classify_response(404, "{\"message\":\"no analysis found\"}", None)
            .expect_err("404 con marca debe fallar");
        assert!(matches!(err, GithubError::NotConfigured));
    }

    /// Un `404` sin la marca es `NotFound`.
    #[test]
    fn classify_response_404_generico_es_not_found() {
        let err = classify_response(404, "{\"message\":\"Not Found\"}", None)
            .expect_err("404 debe fallar");
        assert!(matches!(err, GithubError::NotFound));
    }

    /// Un `5xx` es `Http` y conserva estado y cuerpo.
    #[test]
    fn classify_response_500_es_http() {
        let err = classify_response(500, "boom", None).expect_err("500 debe fallar");
        match err {
            GithubError::Http { status, body } => {
                assert_eq!(status, 500);
                assert_eq!(body, "boom");
            }
            otro => panic!("se esperaba Http, no: {otro:?}"),
        }
    }

    /// El cuerpo largo se recorta sin partir caracteres multibyte.
    #[test]
    fn classify_response_recorta_cuerpos_largos() {
        let cuerpo = "é".repeat(MAX_BODY_CHARS + 100);
        let err = classify_response(500, &cuerpo, None).expect_err("500 debe fallar");
        match err {
            GithubError::Http { body, .. } => {
                assert_eq!(body.chars().count(), MAX_BODY_CHARS);
            }
            otro => panic!("se esperaba Http, no: {otro:?}"),
        }
    }

    /// `is_rate_limited` distingue los límites del resto de errores.
    #[test]
    fn is_rate_limited_solo_para_los_limites() {
        assert!(GithubError::RateLimited { retry_after: None }.is_rate_limited());
        assert!(!GithubError::Unauthorized.is_rate_limited());
    }

    /// `is_pagination_limit` distingue el corte de seguridad de la paginación.
    #[test]
    fn is_pagination_limit_solo_para_el_corte() {
        assert!(GithubError::PaginationLimit { pages: 1000 }.is_pagination_limit());
        assert!(!GithubError::NotFound.is_pagination_limit());
    }
}
