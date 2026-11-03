//! Errores del cliente HTTP de la API de GitHub.
//!
//! Ninguna variante transporta la credencial: el token solo se usa al construir
//! la cabecera `Authorization` y jamás se copia a un mensaje de error. Por eso
//! los errores pueden mostrarse al usuario o registrarse sin riesgo de filtrar
//! el secreto.

use std::time::Duration;

use thiserror::Error;

/// Marca que la API de GitHub devuelve en el cuerpo de un `404` cuando la
/// capacidad consultada no está configurada (por ejemplo, CodeQL sin análisis).
pub const NOT_CONFIGURED_MARKER: &str = "no analysis found";

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

    /// Respuesta de error no contemplada más arriba.
    ///
    /// Cubre, entre otras, el `403` sin `Retry-After` con el que GitHub señala
    /// que una capacidad está deshabilitada; la fase de recolección lo
    /// interpreta a partir de `status`.
    #[error("respuesta inesperada de la API de GitHub: HTTP {status}")]
    Http {
        /// Código de estado HTTP.
        status: u16,
        /// Cuerpo de la respuesta, acotado a [`MAX_BODY_CHARS`].
        body: String,
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
}

/// Clasifica una respuesta de la API de GitHub en un error tipado.
///
/// Devuelve `Ok(())` para respuestas de éxito (`2xx`) y redirecciones (`3xx`).
///
/// `retry_after` es la espera ya extraída de la cabecera `Retry-After`: un
/// `403` **con** esa cabecera es un límite secundario
/// ([`GithubError::RateLimited`]); un `403` **sin** ella no lo es y se devuelve
/// como [`GithubError::Http`] con `status = 403`, de modo que quien recolecta
/// pueda distinguir «capacidad deshabilitada» de «demasiadas peticiones».
///
/// # Errores
///
/// - [`GithubError::Unauthorized`] para `401`.
/// - [`GithubError::RateLimited`] para `403` con `Retry-After`.
/// - [`GithubError::NotConfigured`] para `404` con `no analysis found`.
/// - [`GithubError::NotFound`] para el resto de `404`.
/// - [`GithubError::Http`] para cualquier otro `status >= 400`.
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
            None => Err(GithubError::Http {
                status,
                body: truncate_body(body),
            }),
        },
        404 if body.contains(NOT_CONFIGURED_MARKER) => Err(GithubError::NotConfigured),
        404 => Err(GithubError::NotFound),
        400..=u16::MAX => Err(GithubError::Http {
            status,
            body: truncate_body(body),
        }),
        _ => Ok(()),
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

    /// Un `403` sin `Retry-After` no es un límite: es un `Http` con estado 403
    /// (capacidad deshabilitada).
    #[test]
    fn classify_response_403_sin_retry_after_es_http() {
        let err = classify_response(
            403,
            "{\"message\":\"Dependabot alerts are disabled\"}",
            None,
        )
        .expect_err("403 sin Retry-After debe fallar");
        match err {
            GithubError::Http { status, .. } => assert_eq!(status, 403),
            otro => panic!("se esperaba Http, no: {otro:?}"),
        }
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
}
