//! Configuración del notificador de Valet.
//!
//! La configuración se resuelve del entorno (`VALET_BASE_URL` y
//! `VALET_INGEST_TOKEN`) **o** del fichero `~/.config/ghwatch/config.toml`, por
//! ese orden de precedencia: el entorno gana y el fichero solo completa lo que
//! falte. Ese fichero contiene el token en claro, así que sus permisos se
//! verifican con [`check_permissions`]: uno más laxo que `600` se rechaza en
//! lugar de usarse en silencio.
//!
//! La carga del fichero es best-effort: un fichero ausente no es un error y uno
//! ilegible, con TOML inválido o con permisos laxos se descarta con un aviso,
//! sin que su token llegue a usarse nunca.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{IngestToken, NotifyError};

/// Variable de entorno con la URL base de Valet.
pub const ENV_BASE_URL: &str = "VALET_BASE_URL";
/// Variable de entorno con el token de ingesta de Valet.
pub const ENV_INGEST_TOKEN: &str = "VALET_INGEST_TOKEN";
/// URL base por defecto de Valet.
pub const DEFAULT_BASE_URL: &str = "http://localhost:3000";

/// Configuración resuelta del notificador.
#[derive(Debug, Clone)]
pub struct ValetSettings {
    /// URL base de Valet (sin barra final garantizada por el cliente).
    pub base_url: String,
    /// Token de ingesta, si está configurado.
    pub token: Option<IngestToken>,
}

impl ValetSettings {
    /// Resuelve la configuración a partir de un acceso al entorno inyectable.
    ///
    /// La lectura del entorno se inyecta como clausura para poder probarla sin
    /// tocar el entorno del proceso. Un valor vacío o en blanco se trata como
    /// ausente: la URL cae a su valor por defecto y el token queda sin
    /// configurar.
    #[must_use]
    pub fn from_env<F>(get: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let base_url = get(ENV_BASE_URL)
            .filter(|valor| !valor.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let token = get(ENV_INGEST_TOKEN).and_then(|valor| IngestToken::new(valor).ok());
        Self { base_url, token }
    }

    /// Resuelve la configuración con precedencia **entorno > fichero**.
    ///
    /// Primero usa el entorno inyectado (`get`). Lo que falte se completa desde
    /// `config_path` (TOML con `valet_base_url`/`valet_ingest_token`, o sus alias
    /// `base_url`/`ingest_token`), siempre que el fichero exista y sus permisos
    /// sean exactamente `600`. Un fichero ausente
    /// no es un error; uno con permisos más laxos se rechaza con un `warn!` y su
    /// token **no** se usa.
    ///
    /// La ruta se recibe como parámetro (no se lee `HOME` aquí) para poder
    /// probarla sin depender del entorno real; [`default_config_path`] aporta la
    /// ruta convencional y [`ValetSettings::from_process_env`] la usa.
    #[must_use]
    pub fn from_env_or_file<F>(get: F, config_path: &Path) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let base_entorno = get(ENV_BASE_URL).filter(|valor| !valor.trim().is_empty());
        let token_entorno = get(ENV_INGEST_TOKEN).filter(|valor| !valor.trim().is_empty());

        // Solo se lee el fichero si el entorno no cubre ya ambos valores.
        let fichero = if base_entorno.is_none() || token_entorno.is_none() {
            cargar_fichero(config_path)
        } else {
            None
        };

        let base_url = base_entorno
            .or_else(|| {
                fichero
                    .as_ref()
                    .and_then(|config| config.base_url.clone())
                    .filter(|valor| !valor.trim().is_empty())
            })
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let token = token_entorno
            .or_else(|| {
                fichero
                    .as_ref()
                    .and_then(|config| config.ingest_token.clone())
            })
            .and_then(|valor| IngestToken::new(valor).ok());

        Self { base_url, token }
    }

    /// Resuelve la configuración desde el entorno real del proceso y el fichero
    /// convencional del usuario ([`default_config_path`]).
    ///
    /// Es el punto de entrada de producción; si no hay `HOME` disponible cae
    /// directamente al entorno.
    #[must_use]
    pub fn from_process_env() -> Self {
        let get = |clave: &str| std::env::var(clave).ok();
        match default_config_path() {
            Some(ruta) => Self::from_env_or_file(get, &ruta),
            None => Self::from_env(get),
        }
    }
}

/// Contenido del fichero de configuración (`config.toml`).
///
/// Solo las claves que el notificador usa; cualquier otra se ignora. La grafía
/// **canónica** es la del CLI (`valet_base_url`/`valet_ingest_token`), pero se
/// aceptan también `base_url`/`ingest_token` como alias: así el notificador y el
/// `config.rs` del CLI interpretan el **mismo** fichero con las mismas claves.
#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    /// URL base de Valet.
    #[serde(alias = "valet_base_url")]
    base_url: Option<String>,
    /// Token de ingesta de Valet.
    #[serde(alias = "valet_ingest_token")]
    ingest_token: Option<String>,
}

/// Ruta convencional del fichero de configuración (`~/.config/ghwatch/config.toml`).
///
/// Devuelve `None` si no hay `HOME` en el entorno.
#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("ghwatch")
            .join("config.toml")
    })
}

/// Carga el fichero de configuración si existe y sus permisos son `600`.
///
/// Es best-effort: un fichero ausente, ilegible, con permisos laxos o con TOML
/// inválido devuelve `None` (con un aviso en los casos anómalos) en lugar de
/// propagar un error. El token del fichero **nunca** se registra.
fn cargar_fichero(config_path: &Path) -> Option<FileConfig> {
    if !config_path.exists() {
        return None;
    }

    match check_permissions(config_path) {
        Ok(()) => {}
        Err(NotifyError::InsecurePermissions { mode, .. }) => {
            tracing::warn!(
                path = %config_path.display(),
                mode,
                "se rechaza el fichero de configuración por permisos más laxos que 600; no se usa"
            );
            return None;
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "no se pudieron comprobar los permisos del fichero de configuración; no se usa"
            );
            return None;
        }
    }

    let contenido = match std::fs::read_to_string(config_path) {
        Ok(contenido) => contenido,
        Err(error) => {
            tracing::warn!(
                path = %config_path.display(),
                error = %error,
                "no se pudo leer el fichero de configuración; se ignora"
            );
            return None;
        }
    };

    match toml::from_str::<FileConfig>(&contenido) {
        Ok(config) => Some(config),
        Err(error) => {
            tracing::warn!(
                path = %config_path.display(),
                error = %error,
                "el fichero de configuración no es TOML válido; se ignora"
            );
            None
        }
    }
}

/// Comprueba que el fichero de configuración tenga permisos `600`.
///
/// El fichero guarda el token en claro: cualquier permiso del grupo o de otros
/// usuarios lo expone. En plataformas sin el concepto de permisos unix la
/// comprobación no aplica y se acepta el fichero.
///
/// # Errores
///
/// Devuelve [`NotifyError::InsecurePermissions`] si el modo no es exactamente
/// `600`, y [`NotifyError::ConfigIo`] si no se pueden leer sus metadatos.
pub fn check_permissions(path: &Path) -> Result<(), NotifyError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadatos = std::fs::metadata(path).map_err(|source| NotifyError::ConfigIo {
            path: path.to_path_buf(),
            source,
        })?;
        let mode = metadatos.permissions().mode() & 0o777;
        if mode != 0o600 {
            return Err(NotifyError::InsecurePermissions {
                path: path.to_path_buf(),
                mode,
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Las variables vacías caen a los valores por defecto.
    #[test]
    fn el_entorno_vacio_usa_los_valores_por_defecto() {
        let settings = ValetSettings::from_env(|_| None);
        assert_eq!(settings.base_url, DEFAULT_BASE_URL);
        assert!(settings.token.is_none());

        let vacio = ValetSettings::from_env(|_| Some("   ".to_string()));
        assert_eq!(vacio.base_url, DEFAULT_BASE_URL);
        assert!(vacio.token.is_none());
    }

    /// Las variables definidas se respetan.
    #[test]
    fn el_entorno_define_la_configuracion() {
        let settings = ValetSettings::from_env(|clave| match clave {
            ENV_BASE_URL => Some("http://valet.test:9000".to_string()),
            ENV_INGEST_TOKEN => Some("val_abc".to_string()),
            _ => None,
        });
        assert_eq!(settings.base_url, "http://valet.test:9000");
        assert_eq!(
            settings.token.as_ref().map(IngestToken::expose_secret),
            Some("val_abc")
        );
    }
}
