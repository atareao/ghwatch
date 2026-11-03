//! Resolución de la credencial de GitHub.
//!
//! El token se busca en este orden:
//!
//! 1. La variable de entorno [`ENV_VAR`] (`GITHUB_TOKEN`).
//! 2. La CLI `gh` (`gh auth token`).
//! 3. Si ninguna de las dos vías funciona, un error explicativo
//!    ([`AuthError::NoCredentials`]).
//!
//! El token **nunca** se expone fuera del proceso: ni en mensajes de error, ni
//! en logs, ni al formatear [`Token`] con `Debug` o `Display`, que lo redactan.
//!
//! La lógica de decisión vive en [`resolve_token_from`], una función pura con
//! el entorno y el runner de `gh` inyectados, lo que permite probarla sin tocar
//! el entorno del proceso ni lanzar procesos externos. [`resolve_token`] es la
//! envoltura fina que conecta esa lógica con las implementaciones reales.

use std::process::Command;

/// Nombre de la variable de entorno consultada en primer lugar.
pub const ENV_VAR: &str = "GITHUB_TOKEN";

/// Credencial de GitHub.
///
/// Envuelve el token en claro pero redacta su contenido al formatearlo con
/// `Debug` o `Display`. El valor solo se obtiene a través de
/// [`Token::expose_secret`], de modo que el punto de fuga queda explícito y
/// acotado.
#[derive(Clone)]
pub struct Token(String);

impl Token {
    /// Devuelve el valor en claro del token.
    ///
    /// Es el único punto de fuga deliberado. Quien lo use asume la
    /// responsabilidad de no registrarlo ni incluirlo en la salida.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl From<String> for Token {
    fn from(valor: String) -> Self {
        Self(valor)
    }
}

impl From<&str> for Token {
    fn from(valor: &str) -> Self {
        Self(valor.to_string())
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(***)")
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

/// Error al resolver la credencial de GitHub.
///
/// Ninguna variante incluye el valor del token: el mensaje resultante puede
/// mostrarse al usuario o registrarse sin riesgo de filtrarlo.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// La variable de entorno `GITHUB_TOKEN` no está definida.
    #[error("la variable de entorno GITHUB_TOKEN no está definida")]
    EnvVarMissing,

    /// La CLI `gh` no está instalada o no está disponible en el `PATH`.
    #[error("`gh` no está instalado o no está en el PATH")]
    GhNotInstalled,

    /// `gh auth token` terminó con un código de salida distinto de cero.
    ///
    /// No se conserva la salida de error de `gh` para no arriesgar filtrar
    /// información sensible.
    #[error("`gh auth token` terminó con error (código de salida {code:?})")]
    GhFailed {
        /// Código de salida del proceso, si el sistema lo proporcionó.
        code: Option<i32>,
    },

    /// `gh auth token` terminó correctamente pero sin devolver credencial.
    #[error("`gh auth token` no devolvió ninguna credencial")]
    GhEmpty,

    /// No fue posible obtener una credencial por ninguna de las dos vías.
    ///
    /// El mensaje detalla ambas alternativas (`GITHUB_TOKEN` y `gh auth token`)
    /// y adjunta la causa concreta por la que falló la vía de `gh`.
    #[error(
        "no se pudo obtener una credencial de GitHub: define la variable de entorno \
         GITHUB_TOKEN o autentícate con `gh auth login` (se intentó `gh auth token`). \
         Causa: {cause}"
    )]
    NoCredentials {
        /// Motivo por el que falló la resolución mediante `gh`.
        cause: Box<AuthError>,
    },
}

/// Resuelve la credencial usando el entorno y el runner de `gh` inyectados.
///
/// Es una función pura respecto al entorno del proceso: no lee variables de
/// entorno ni lanza procesos por sí misma. Eso la hace determinista y
/// comprobable con dobles.
///
/// # Orden de resolución
///
/// 1. `env(ENV_VAR)`: si devuelve un valor no vacío (ignorando espacios), se usa.
/// 2. `run_gh()`: si tiene éxito y devuelve un valor no vacío, se usa.
/// 3. En otro caso, se devuelve [`AuthError::NoCredentials`] con la causa de `gh`.
///
/// # Errores
///
/// Devuelve [`AuthError::NoCredentials`] cuando ninguna de las dos vías aporta
/// una credencial. El mensaje menciona ambas vías y no contiene ningún token.
pub fn resolve_token_from(
    env: impl Fn(&str) -> Option<String>,
    run_gh: impl Fn() -> Result<String, AuthError>,
) -> Result<Token, AuthError> {
    if let Some(valor) = env(ENV_VAR) {
        let valor = valor.trim();
        if !valor.is_empty() {
            return Ok(Token::from(valor));
        }
    }

    match run_gh() {
        Ok(salida) => {
            let salida = salida.trim();
            if salida.is_empty() {
                Err(AuthError::NoCredentials {
                    cause: Box::new(AuthError::GhEmpty),
                })
            } else {
                Ok(Token::from(salida))
            }
        }
        Err(causa) => Err(AuthError::NoCredentials {
            cause: Box::new(causa),
        }),
    }
}

/// Resuelve la credencial contra el entorno real del proceso y la CLI `gh`.
///
/// Envoltura fina de [`resolve_token_from`] que conecta las implementaciones
/// reales: `std::env::var` y `gh auth token`.
///
/// # Errores
///
/// Devuelve [`AuthError::NoCredentials`] si no hay credencial disponible por
/// ninguna de las dos vías.
pub fn resolve_token() -> Result<Token, AuthError> {
    resolve_token_from(|clave| std::env::var(clave).ok(), run_gh_auth_token)
}

/// Ejecuta `gh auth token` y devuelve su salida estándar.
///
/// # Errores
///
/// - [`AuthError::GhNotInstalled`] si el binario `gh` no existe.
/// - [`AuthError::GhFailed`] si no se pudo lanzar el proceso o si `gh` terminó
///   con un código distinto de cero.
/// - [`AuthError::GhEmpty`] si `gh` terminó bien pero sin salida.
///
/// Deliberadamente no se incluye la salida de `gh` en el error: así el mensaje
/// jamás puede contener el token.
fn run_gh_auth_token() -> Result<String, AuthError> {
    let salida = match Command::new("gh").args(["auth", "token"]).output() {
        Ok(salida) => salida,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AuthError::GhNotInstalled);
        }
        Err(_) => return Err(AuthError::GhFailed { code: None }),
    };

    if !salida.status.success() {
        return Err(AuthError::GhFailed {
            code: salida.status.code(),
        });
    }

    if String::from_utf8_lossy(&salida.stdout).trim().is_empty() {
        return Err(AuthError::GhEmpty);
    }

    Ok(String::from_utf8_lossy(&salida.stdout).into_owned())
}
