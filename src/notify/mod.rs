//! Notificación del resumen del escaneo a Valet.
//!
//! Este es el único módulo que habla con Valet. Usa su API de ingesta ya
//! existente —`POST {VALET_BASE_URL}/api/notifications` con un token de ingesta
//! (`Authorization: Bearer val_…`)— y respeta sus límites: `message` de como
//! máximo [`MESSAGE_LIMIT`] bytes y `metadata` de como máximo [`METADATA_LIMIT`].
//!
//! # Best-effort
//!
//! La notificación nunca aborta el escaneo: un fallo se traduce a un aviso y el
//! informe en disco ya está escrito. El único caso en que se decide no
//! reintentar es el `401` (token revocado), que se propaga como
//! [`NotifyError::Unauthorized`] para que el llamante lo registre sin el token.
//!
//! # El token nunca se registra
//!
//! [`IngestToken`] redacta su `Debug`/`Display` y solo expone el valor por
//! [`IngestToken::expose_secret`]. Ningún error de [`NotifyError`] transporta el
//! valor del token.

pub mod client;
pub mod config;
pub mod emit;
pub mod message;

pub use client::{ValetClient, ValetConfig};
pub use config::{
    DEFAULT_BASE_URL, ENV_BASE_URL, ENV_INGEST_TOKEN, ValetSettings, check_permissions,
    default_config_path,
};
pub use emit::{EmitOutcome, NotifyOutcome, emit_and_notify, send_document};
pub use message::{
    MESSAGE_LIMIT, METADATA_LIMIT, NotifyLevel, ValetNotification, build_notification, level_for,
    should_notify, truncate_utf8,
};

/// Error de la integración con Valet.
///
/// Ninguna variante incluye el valor del token: el mensaje puede registrarse sin
/// riesgo de filtrarlo.
#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    /// Fallo de red, de tiempo de espera agotado o de decodificación.
    #[error("fallo al enviar la notificación a Valet: {0}")]
    Request(#[from] reqwest::Error),

    /// Valet respondió con un estado que no es de éxito.
    #[error("Valet respondió con el estado {status}")]
    Status {
        /// Código de estado HTTP devuelto por Valet.
        status: u16,
    },

    /// El token de ingesta fue rechazado (revocado o desconocido): se aborta sin
    /// reintentar.
    #[error("el token de ingesta fue rechazado (401): se aborta sin reintentar")]
    Unauthorized,

    /// Se intentó construir un [`IngestToken`] con un valor vacío.
    #[error("no se puede construir un token de ingesta vacío o compuesto solo de espacios")]
    EmptyToken,

    /// El fichero de configuración tiene permisos más laxos que `600`.
    #[error("el fichero de configuración {path:?} tiene permisos {mode:o}, más laxos que 600")]
    InsecurePermissions {
        /// Ruta del fichero de configuración.
        path: std::path::PathBuf,
        /// Modo observado (permisos unix, sin los bits de tipo).
        mode: u32,
    },

    /// No se pudo leer el fichero de configuración.
    #[error("no se pudo leer el fichero de configuración {path:?}: {source}")]
    ConfigIo {
        /// Ruta del fichero de configuración.
        path: std::path::PathBuf,
        /// Causa del fallo de lectura.
        #[source]
        source: std::io::Error,
    },
}

/// Token de ingesta de Valet.
///
/// Envuelve el token en claro pero redacta su contenido al formatearlo con
/// `Debug` o `Display`. El valor solo se obtiene a través de
/// [`IngestToken::expose_secret`], de modo que el punto de fuga queda explícito
/// y acotado.
#[derive(Clone)]
pub struct IngestToken(String);

impl IngestToken {
    /// Construye un token a partir de un valor, validando que no esté vacío.
    ///
    /// El valor se recorta por delante y por detrás: un token nunca debería
    /// tener espacios y un valor en blanco no es una credencial válida.
    ///
    /// # Errores
    ///
    /// Devuelve [`NotifyError::EmptyToken`] si, tras recortar, el valor queda
    /// vacío.
    pub fn new(valor: impl Into<String>) -> Result<Self, NotifyError> {
        let valor = valor.into();
        let recortado = valor.trim();
        if recortado.is_empty() {
            return Err(NotifyError::EmptyToken);
        }
        Ok(Self(recortado.to_string()))
    }

    /// Devuelve el valor en claro del token.
    ///
    /// Es el único punto de fuga deliberado. Quien lo use asume la
    /// responsabilidad de no registrarlo ni incluirlo en la salida.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for IngestToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IngestToken(***)")
    }
}

impl std::fmt::Display for IngestToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un token vacío o en blanco se rechaza; uno válido se recorta.
    #[test]
    fn el_token_valida_y_recorta() {
        assert!(matches!(
            IngestToken::new("   "),
            Err(NotifyError::EmptyToken)
        ));
        let token = IngestToken::new("  val_abc  ").expect("token válido");
        assert_eq!(token.expose_secret(), "val_abc");
    }

    /// El token nunca aparece al formatearlo.
    #[test]
    fn el_token_se_redacta() {
        let token = IngestToken::new("val_super_secreto").expect("token válido");
        let debug = format!("{token:?}");
        let display = format!("{token}");
        assert!(!debug.contains("val_super_secreto"), "{debug}");
        assert!(!display.contains("val_super_secreto"), "{display}");
    }
}
