//! Cliente HTTP de la API de ingesta de Valet.
//!
//! Envía `POST {base_url}/api/notifications` con `Authorization: Bearer <token>`
//! y `Content-Type: application/json`, y espera un `201` con el identificador de
//! la notificación. La URL base es configurable para apuntar a un servidor falso
//! en los tests.
//!
//! Un `401` (token revocado o desconocido) se traduce a
//! [`NotifyError::Unauthorized`] y **no** se reintenta: reintentar con la misma
//! credencial no cambiaría el resultado.

use std::time::Duration;

use reqwest::StatusCode;
use serde::Deserialize;

use super::config::DEFAULT_BASE_URL;
use super::{IngestToken, NotifyError, message::ValetNotification};

/// Configuración del cliente Valet.
#[derive(Debug, Clone)]
pub struct ValetConfig {
    /// URL base de Valet (sin barra final).
    pub base_url: String,
    /// Tiempo máximo de espera de la petición.
    pub timeout: Duration,
}

impl Default for ValetConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: Duration::from_secs(10),
        }
    }
}

/// Cliente de la API de ingesta de Valet.
///
/// El `Debug` derivado redacta el token a través de [`IngestToken`], de modo que
/// registrar el cliente nunca filtra la credencial.
#[derive(Debug, Clone)]
pub struct ValetClient {
    /// Cliente HTTP subyacente.
    http: reqwest::Client,
    /// Configuración del cliente.
    config: ValetConfig,
    /// Token de ingesta.
    token: IngestToken,
}

impl ValetClient {
    /// Construye el cliente a partir de su configuración y su token.
    ///
    /// # Errores
    ///
    /// Devuelve [`NotifyError`] si no se puede construir el cliente HTTP.
    pub fn new(config: ValetConfig, token: IngestToken) -> Result<Self, NotifyError> {
        // Las redirecciones no se siguen: un `3xx` es un estado anómalo para la
        // API de ingesta y debe observarse como tal (`Status`), no seguirse en
        // silencio hacia otro destino (coherente con cómo `github` trata los `3xx`).
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            http,
            config,
            token,
        })
    }

    /// Envía una notificación y devuelve el identificador del `201`.
    ///
    /// # Errores
    ///
    /// Devuelve [`NotifyError::Unauthorized`] si Valet responde `401` (sin
    /// reintentar), [`NotifyError::Status`] para cualquier otro estado que no sea
    /// de éxito y [`NotifyError::Request`] si falla la red o la decodificación.
    pub async fn send(&self, notification: &ValetNotification) -> Result<String, NotifyError> {
        let url = format!(
            "{}/api/notifications",
            self.config.base_url.trim_end_matches('/')
        );

        let respuesta = self
            .http
            .post(&url)
            .bearer_auth(self.token.expose_secret())
            .json(notification)
            .send()
            .await?;

        let estado = respuesta.status();
        if estado == StatusCode::UNAUTHORIZED {
            return Err(NotifyError::Unauthorized);
        }
        if !estado.is_success() {
            return Err(NotifyError::Status {
                status: estado.as_u16(),
            });
        }

        let creada: CreatedResponse = respuesta.json().await?;
        Ok(creada.id)
    }
}

/// Respuesta `201` de la API de ingesta.
#[derive(Debug, Deserialize)]
struct CreatedResponse {
    /// Identificador de la notificación creada.
    id: String,
}
