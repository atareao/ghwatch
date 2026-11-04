//! Resolución de la configuración de ejecución del CLI.
//!
//! Los valores se resuelven con la precedencia **flags > entorno > fichero >
//! defecto**. El fichero es `~/.config/ghwatch/config.toml` (o el indicado con
//! `--config`) y contiene, entre otras claves, los tokens en claro; por eso sus
//! permisos se verifican con [`crate::notify::check_permissions`]: un fichero más
//! laxo que `600` se rechaza con un aviso y **no se usa**.
//!
//! La carga del fichero es best-effort: ausente, ilegible, con TOML inválido o
//! con permisos laxos se ignora sin propagar error. La lectura del entorno se
//! inyecta como clausura para que la resolución sea determinista y comprobable
//! sin tocar el entorno del proceso.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::auth::{self, Token};
use crate::github::client::DEFAULT_BASE_URL as GITHUB_DEFAULT_BASE_URL;
use crate::notify::config::{
    DEFAULT_BASE_URL as VALET_DEFAULT_BASE_URL, ENV_BASE_URL, ENV_INGEST_TOKEN, check_permissions,
};
use crate::notify::{IngestToken, NotifyError};

/// Variable de entorno con la ruta de la base de datos.
pub const ENV_DB: &str = "GHWATCH_DB";
/// Variable de entorno con el directorio de informes.
pub const ENV_REPORT_DIR: &str = "GHWATCH_REPORT_DIR";
/// Variable de entorno con la concurrencia del cliente de GitHub.
pub const ENV_CONCURRENCY: &str = "GHWATCH_CONCURRENCY";
/// Variable de entorno que reincorpora los forks.
pub const ENV_INCLUDE_FORKS: &str = "GHWATCH_INCLUDE_FORKS";
/// Variable de entorno que reincorpora los repositorios archivados.
pub const ENV_INCLUDE_ARCHIVED: &str = "GHWATCH_INCLUDE_ARCHIVED";
/// Variable de entorno con la URL base de la API de GitHub.
pub const ENV_GITHUB_BASE_URL: &str = "GHWATCH_GITHUB_BASE_URL";
/// Variable de entorno con la URL base de OSV.dev.
pub const ENV_OSV_BASE_URL: &str = "GHWATCH_OSV_BASE_URL";

/// Concurrencia por defecto del cliente de GitHub.
pub const DEFAULT_CONCURRENCY: usize = crate::github::client::DEFAULT_CONCURRENCY;

/// URL base por defecto de OSV.dev.
///
/// Se reutiliza la constante del cliente de OSV para no duplicar el literal.
pub const DEFAULT_OSV_BASE_URL: &str = crate::osv::DEFAULT_OSV_BASE_URL;

/// Valores aportados por los flags de línea de comandos.
///
/// Un campo a `None` (o un booleano a `false`) significa «no especificado»: la
/// resolución cae al entorno, al fichero o al valor por defecto.
#[derive(Debug, Clone, Default)]
pub struct ConfigFlags {
    /// Ruta de la base de datos (`--db`).
    pub db_path: Option<PathBuf>,
    /// Directorio de informes (`--report-dir`).
    pub report_dir: Option<PathBuf>,
    /// Concurrencia del cliente (`--concurrency`).
    pub concurrency: Option<usize>,
    /// Reincorpora los forks (`--include-forks`).
    pub include_forks: bool,
    /// Reincorpora los repositorios archivados (`--include-archived`).
    pub include_archived: bool,
    /// URL base de la API de GitHub (`--github-base-url`).
    pub github_base_url: Option<String>,
    /// URL base de OSV.dev (`--osv-base-url`).
    pub osv_base_url: Option<String>,
    /// URL base de Valet (`--valet-base-url`).
    pub valet_base_url: Option<String>,
}

/// Configuración ya resuelta, lista para construir los clientes.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Credencial de GitHub, si está disponible.
    pub github_token: Option<Token>,
    /// URL base de Valet.
    pub valet_base_url: String,
    /// Token de ingesta de Valet, si está configurado.
    pub valet_ingest_token: Option<IngestToken>,
    /// Ruta de la base de datos SQLite.
    pub db_path: PathBuf,
    /// Directorio donde se escriben los informes.
    pub report_dir: PathBuf,
    /// Concurrencia del cliente de GitHub.
    pub concurrency: usize,
    /// Reincorpora los forks al inventario.
    pub include_forks: bool,
    /// Reincorpora los repositorios archivados al inventario.
    pub include_archived: bool,
    /// URL base de la API de GitHub.
    pub github_base_url: String,
    /// URL base de OSV.dev.
    pub osv_base_url: String,
}

/// Resuelve la configuración con la precedencia documentada.
///
/// `get_env` es el acceso inyectable al entorno; `config_path` es la ruta del
/// fichero TOML (`None` si no hay uno). La función no lee el entorno del proceso
/// ni toca la red.
#[must_use]
pub fn resolve(
    flags: &ConfigFlags,
    get_env: &dyn Fn(&str) -> Option<String>,
    config_path: Option<&Path>,
) -> ResolvedConfig {
    let fichero = config_path.and_then(cargar_fichero).unwrap_or_default();

    let github_token = primero_no_vacio([get_env(auth::ENV_VAR), fichero.github_token.clone()])
        .and_then(|valor| Token::new(valor).ok());

    let valet_base_url = primero_no_vacio([
        flags.valet_base_url.clone(),
        get_env(ENV_BASE_URL),
        fichero.valet_base_url.clone(),
    ])
    .unwrap_or_else(|| VALET_DEFAULT_BASE_URL.to_string());

    let valet_ingest_token = primero_no_vacio([
        get_env(ENV_INGEST_TOKEN),
        fichero.valet_ingest_token.clone(),
    ])
    .and_then(|valor| IngestToken::new(valor).ok());

    let db_path = ruta(
        flags.db_path.clone(),
        get_env(ENV_DB),
        fichero.db_path.clone(),
        default_db_path(get_env),
        get_env,
    );
    let report_dir = ruta(
        flags.report_dir.clone(),
        get_env(ENV_REPORT_DIR),
        fichero.report_dir.clone(),
        default_report_dir(get_env),
        get_env,
    );

    let concurrency = entero(
        flags.concurrency,
        get_env(ENV_CONCURRENCY),
        fichero.concurrency,
        DEFAULT_CONCURRENCY,
    );

    let include_forks = booleano(
        flags.include_forks,
        get_env(ENV_INCLUDE_FORKS),
        fichero.include_forks,
    );
    let include_archived = booleano(
        flags.include_archived,
        get_env(ENV_INCLUDE_ARCHIVED),
        fichero.include_archived,
    );

    let github_base_url = primero_no_vacio([
        flags.github_base_url.clone(),
        get_env(ENV_GITHUB_BASE_URL),
        fichero.github_base_url.clone(),
    ])
    .unwrap_or_else(|| GITHUB_DEFAULT_BASE_URL.to_string());

    let osv_base_url = primero_no_vacio([
        flags.osv_base_url.clone(),
        get_env(ENV_OSV_BASE_URL),
        fichero.osv_base_url.clone(),
    ])
    .unwrap_or_else(|| DEFAULT_OSV_BASE_URL.to_string());

    ResolvedConfig {
        github_token,
        valet_base_url,
        valet_ingest_token,
        db_path,
        report_dir,
        concurrency,
        include_forks,
        include_archived,
        github_base_url,
        osv_base_url,
    }
}

/// Ruta por defecto de la base de datos (`~/.local/share/ghwatch/ghwatch.db`).
///
/// Si no hay `HOME`, se usa `./ghwatch.db` como último recurso.
#[must_use]
pub fn default_db_path(get_env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    directorio_datos(get_env).join("ghwatch.db")
}

/// Directorio por defecto de los informes (`~/.local/share/ghwatch/reports`).
#[must_use]
pub fn default_report_dir(get_env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    directorio_datos(get_env).join("reports")
}

/// Ruta por defecto del fichero de configuración
/// (`~/.config/ghwatch/config.toml`), o `None` sin `HOME`.
#[must_use]
pub fn default_config_path(get_env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    home(get_env).map(|home| home.join(".config").join("ghwatch").join("config.toml"))
}

/// Directorio de datos de la aplicación bajo `HOME`.
fn directorio_datos(get_env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    home(get_env)
        .map(|home| home.join(".local").join("share").join("ghwatch"))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `HOME` como ruta, si está definido y no está vacío.
fn home(get_env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    get_env("HOME").and_then(texto_no_vacio).map(PathBuf::from)
}

/// Contenido del fichero de configuración.
///
/// Solo se interpretan las claves conocidas; cualquier otra se ignora.
#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    github_token: Option<String>,
    /// URL base de Valet. La grafía canónica es `valet_base_url`; se acepta
    /// también `base_url` (la que documenta el notificador) para no ignorarla en
    /// silencio.
    #[serde(alias = "base_url")]
    valet_base_url: Option<String>,
    /// Token de ingesta de Valet. Canónica `valet_ingest_token`; se acepta
    /// también `ingest_token`.
    #[serde(alias = "ingest_token")]
    valet_ingest_token: Option<String>,
    db_path: Option<PathBuf>,
    report_dir: Option<PathBuf>,
    concurrency: Option<usize>,
    include_forks: Option<bool>,
    include_archived: Option<bool>,
    github_base_url: Option<String>,
    osv_base_url: Option<String>,
}

/// Carga el fichero si existe y sus permisos son exactamente `600`.
///
/// Es best-effort: cualquier anomalía se registra como aviso y se devuelve
/// `None`, sin propagar error. El token del fichero nunca se registra.
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
                path = %config_path.display(),
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

/// Devuelve el primer valor no vacío, recortado.
fn primero_no_vacio<const N: usize>(valores: [Option<String>; N]) -> Option<String> {
    valores
        .into_iter()
        .flatten()
        .filter_map(texto_no_vacio)
        .next()
}

/// Recorta un valor y lo descarta si queda vacío.
fn texto_no_vacio(valor: String) -> Option<String> {
    let recortado = valor.trim().to_string();
    if recortado.is_empty() {
        None
    } else {
        Some(recortado)
    }
}

/// Resuelve una ruta con precedencia flag > entorno > fichero > defecto.
///
/// La ruta ganadora se expande contra el `HOME` inyectado si empieza por `~`
/// (véase [`expandir_tilde`]), de modo que la plantilla de `init` con
/// `~/.local/share/...` no acaba creando un directorio literal `~` bajo el CWD.
fn ruta(
    flag: Option<PathBuf>,
    entorno: Option<String>,
    fichero: Option<PathBuf>,
    defecto: PathBuf,
    get_env: &dyn Fn(&str) -> Option<String>,
) -> PathBuf {
    let resuelta = flag
        .or_else(|| entorno.and_then(texto_no_vacio).map(PathBuf::from))
        .or(fichero)
        .unwrap_or(defecto);
    expandir_tilde(resuelta, get_env)
}

/// Expande `~` y `~/…` contra el `HOME` inyectado.
///
/// `~` a secas es el `HOME`; `~/resto` cuelga de él. Cualquier otra forma
/// (incluido `~usuario`) se deja intacta: no hay base para resolverla sin tocar
/// el entorno real. Sin `HOME` disponible, la ruta se conserva tal cual.
fn expandir_tilde(ruta: PathBuf, get_env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    let Some(texto) = ruta.to_str() else {
        return ruta;
    };
    if texto == "~" {
        return home(get_env).unwrap_or(ruta);
    }
    match texto.strip_prefix("~/") {
        Some(resto) => match home(get_env) {
            Some(home) => home.join(resto),
            None => ruta,
        },
        None => ruta,
    }
}

/// Resuelve un entero positivo, descartando un entorno no parseable.
fn entero(
    flag: Option<usize>,
    entorno: Option<String>,
    fichero: Option<usize>,
    defecto: usize,
) -> usize {
    flag.or_else(|| {
        entorno
            .and_then(texto_no_vacio)
            .and_then(|valor| valor.parse::<usize>().ok())
    })
    .or(fichero)
    .unwrap_or(defecto)
    .max(1)
}

/// Resuelve un booleano; el entorno inválido cae al fichero.
fn booleano(flag: bool, entorno: Option<String>, fichero: Option<bool>) -> bool {
    flag || entorno
        .and_then(texto_no_vacio)
        .and_then(|valor| interpretar_booleano(&valor))
        .or(fichero)
        .unwrap_or(false)
}

/// Interpreta las grafías habituales de un booleano.
fn interpretar_booleano(valor: &str) -> Option<bool> {
    match valor.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un valor vacío o en blanco no cuenta como definido.
    #[test]
    fn primero_no_vacio_descarta_los_vacios() {
        assert_eq!(
            primero_no_vacio([None, Some("  ".to_string()), Some(" valor ".to_string())]),
            Some("valor".to_string())
        );
        assert_eq!(primero_no_vacio([None, None]), None);
    }

    /// Las grafías habituales de un booleano se reconocen.
    #[test]
    fn interpretar_booleano_acepta_las_grafias_habituales() {
        assert_eq!(interpretar_booleano("true"), Some(true));
        assert_eq!(interpretar_booleano("SI"), None);
        assert_eq!(interpretar_booleano("on"), Some(true));
        assert_eq!(interpretar_booleano("0"), Some(false));
        assert_eq!(interpretar_booleano("off"), Some(false));
    }

    /// La concurrencia nunca baja de 1.
    #[test]
    fn la_concurrencia_nunca_es_cero() {
        assert_eq!(entero(Some(0), None, None, 6), 1);
        assert_eq!(entero(None, Some("0".to_string()), None, 6), 1);
    }
}
