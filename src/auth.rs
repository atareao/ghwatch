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
    /// Construye un token a partir de un valor, validando que no esté vacío.
    ///
    /// El valor se recorta por delante y por detrás: un token nunca debería
    /// tener espacios, y un valor vacío o en blanco no es una credencial
    /// válida. Al centralizar aquí la validación, el tipo `Token` garantiza su
    /// invariante y no existe una vía pública para crear uno vacío.
    ///
    /// # Errores
    ///
    /// Devuelve [`AuthError::EmptyToken`] si, tras recortar, el valor queda
    /// vacío.
    // Nota: no se marca `#[must_use]` porque `Result` ya lo es y sería redundante.
    pub fn new(valor: impl Into<String>) -> Result<Self, AuthError> {
        let valor = valor.into();
        let recortado = valor.trim();
        if recortado.is_empty() {
            return Err(AuthError::EmptyToken);
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

    /// Se intentó construir un [`Token`] con un valor vacío o en blanco.
    #[error("no se puede construir un Token vacío o compuesto solo de espacios")]
    EmptyToken,

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

    /// `gh auth token` devolvió bytes que no son UTF-8 válido.
    ///
    /// Se trata como error en lugar de sustituir los bytes inválidos por el
    /// carácter de reemplazo (U+FFFD), que produciría un token corrupto sin
    /// avisar.
    #[error("`gh auth token` devolvió una salida que no es UTF-8 válido")]
    GhOutputNotUtf8,

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

/// Devuelve el token de la variable de entorno si es utilizable.
///
/// Un valor ausente, vacío o compuesto solo de espacios no es una credencial:
/// en ese caso devuelve `None` para que se intente la vía de `gh`.
fn token_from_env(env_value: Option<String>) -> Option<Token> {
    env_value.and_then(|valor| Token::new(valor).ok())
}

/// Traduce el resultado del runner de `gh` a una credencial o a un error.
///
/// Una salida vacía (o en blanco) se traduce a [`AuthError::NoCredentials`] con
/// causa [`AuthError::GhEmpty`]; cualquier otro fallo del runner se propaga
/// envuelto en [`AuthError::NoCredentials`] para que el mensaje final explique
/// las dos vías de autenticación.
fn token_from_gh(gh_result: Result<String, AuthError>) -> Result<Token, AuthError> {
    match gh_result {
        Ok(salida) => Token::new(salida).map_err(|_| AuthError::NoCredentials {
            cause: Box::new(AuthError::GhEmpty),
        }),
        Err(causa) => Err(AuthError::NoCredentials {
            cause: Box::new(causa),
        }),
    }
}

/// Resuelve la credencial usando el entorno y el runner de `gh` inyectados.
///
/// Es una función pura respecto al entorno del proceso: no lee variables de
/// entorno ni lanza procesos por sí misma. Eso la hace determinista y
/// comprobable con dobles. Su equivalente con IO real es [`resolve_token`]
/// (síncrono) y [`resolve_token_async`] (asíncrono).
///
/// # Orden de resolución
///
/// 1. `env(ENV_VAR)`: si devuelve un valor no vacío (ignorando espacios), se usa.
/// 2. `run_gh()`: si tiene éxito y devuelve un valor no vacío, se usa.
/// 3. En otro caso, se devuelve [`AuthError::NoCredentials`] con la causa de `gh`.
///
/// El runner de `gh` se evalúa de forma **perezosa**: si la variable de entorno
/// ya aporta un token, `run_gh` no llega a ejecutarse.
///
/// # Errores
///
/// Devuelve [`AuthError::NoCredentials`] cuando ninguna de las dos vías aporta
/// una credencial. El mensaje menciona ambas vías y no contiene ningún token.
pub fn resolve_token_from(
    env: impl FnOnce(&str) -> Option<String>,
    run_gh: impl FnOnce() -> Result<String, AuthError>,
) -> Result<Token, AuthError> {
    match token_from_env(env(ENV_VAR)) {
        Some(token) => Ok(token),
        None => token_from_gh(run_gh()),
    }
}

/// Resuelve la credencial contra el entorno real del proceso y la CLI `gh`
/// de forma **síncrona**.
///
/// Envoltura fina de [`resolve_token_from`] que conecta las implementaciones
/// reales: `std::env::var` y `gh auth token`. Es la vía adecuada para el
/// arranque del CLI, donde aún no hay runtime de Tokio o donde bloquear un hilo
/// no tiene coste. Desde código asíncrono, usa [`resolve_token_async`].
///
/// # Errores
///
/// Devuelve [`AuthError::NoCredentials`] si no hay credencial disponible por
/// ninguna de las dos vías.
pub fn resolve_token() -> Result<Token, AuthError> {
    // `VarError::NotUnicode` se trata igual que «no definida»: un valor que no
    // es texto UTF-8 no puede ser un token utilizable, así que se cae a `gh`.
    resolve_token_from(|clave| std::env::var(clave).ok(), run_gh_auth_token)
}

/// Núcleo asíncrono de la resolución: comparte la decisión con la vía síncrona.
///
/// Recibe el valor del entorno ya leído y un futuro que produce el resultado de
/// `gh`. El futuro se resuelve de forma perezosa: si el entorno ya aporta un
/// token utilizable, no se espera a `gh`.
async fn resolve_from_async<F>(env_value: Option<String>, gh: F) -> Result<Token, AuthError>
where
    F: std::future::Future<Output = Result<String, AuthError>>,
{
    match token_from_env(env_value) {
        Some(token) => Ok(token),
        None => token_from_gh(gh.await),
    }
}

/// Resuelve la credencial contra el entorno real y la CLI `gh` de forma
/// **asíncrona**.
///
/// Usa `tokio::process::Command` en lugar de `std::process::Command`, de modo
/// que esperar a `gh` no bloquea el hilo del executor. Es la vía adecuada
/// cuando el cliente HTTP asíncrono (`reqwest`) va a necesitar el token desde
/// una tarea del runtime. Comparte con [`resolve_token`] tanto la decisión
/// (`token_from_env` / `token_from_gh`) como la interpretación de la salida
/// ([`interpret_gh_exit`]).
///
/// # Errores
///
/// Devuelve [`AuthError::NoCredentials`] si no hay credencial disponible por
/// ninguna de las dos vías.
pub async fn resolve_token_async() -> Result<Token, AuthError> {
    resolve_from_async(std::env::var(ENV_VAR).ok(), run_gh_auth_token_async()).await
}

/// Interpreta el resultado de ejecutar `gh auth token`.
///
/// Es una función **pura**: no lanza procesos ni toca el entorno. Recibe el
/// código de salida y la salida estándar ya capturados y decide si hay
/// credencial o qué error devolver. Concentrar aquí la interpretación permite
/// probar todas las ramas sin IO.
///
/// El caso «`gh` no está instalado» **no** pasa por aquí: solo puede detectarse
/// durante el intento de lanzar el proceso (IO), así que lo resuelve el runner
/// al observar [`std::io::ErrorKind::NotFound`].
///
/// # Errores
///
/// - [`AuthError::GhFailed`] si el código de salida no es `Some(0)`.
/// - [`AuthError::GhOutputNotUtf8`] si la salida no es UTF-8 válido.
/// - [`AuthError::GhEmpty`] si la salida, una vez recortada, está vacía.
///
/// No se incluye la salida de `gh` en ningún error: así el mensaje jamás puede
/// contener el token.
fn interpret_gh_exit(code: Option<i32>, stdout: &[u8]) -> Result<String, AuthError> {
    if code != Some(0) {
        return Err(AuthError::GhFailed { code });
    }

    // Conversión estricta: unos bytes no UTF-8 son un error, no un token
    // silenciosamente corrompido.
    let texto = String::from_utf8(stdout.to_vec()).map_err(|_| AuthError::GhOutputNotUtf8)?;

    let texto = texto.trim();
    if texto.is_empty() {
        return Err(AuthError::GhEmpty);
    }

    Ok(texto.to_string())
}

/// Ejecuta `gh auth token` y devuelve su salida estándar.
///
/// # Errores
///
/// - [`AuthError::GhNotInstalled`] si el binario `gh` no existe.
/// - [`AuthError::GhFailed`] si no se pudo lanzar el proceso o si `gh` terminó
///   con un código distinto de cero.
/// - [`AuthError::GhEmpty`] si `gh` terminó bien pero sin salida.
/// - [`AuthError::GhOutputNotUtf8`] si la salida no es UTF-8 válido.
///
/// La decisión a partir del resultado del proceso la toma
/// [`interpret_gh_exit`]; aquí solo queda el IO (lanzar y capturar).
fn run_gh_auth_token() -> Result<String, AuthError> {
    let salida = match Command::new("gh").args(["auth", "token"]).output() {
        Ok(salida) => salida,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AuthError::GhNotInstalled);
        }
        Err(_) => return Err(AuthError::GhFailed { code: None }),
    };

    interpret_gh_exit(salida.status.code(), &salida.stdout)
}

/// Ejecuta `gh auth token` de forma asíncrona con `tokio::process::Command`.
///
/// Equivalente a [`run_gh_auth_token`] pero sin bloquear el hilo del executor.
/// La decisión a partir del resultado la toma [`interpret_gh_exit`]; el futuro
/// devuelto no lanza el proceso hasta que se le hace `await`, así que un token
/// presente en el entorno evita por completo el `spawn`.
///
/// # Errores
///
/// Los mismos que [`run_gh_auth_token`].
async fn run_gh_auth_token_async() -> Result<String, AuthError> {
    let salida = match tokio::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .await
    {
        Ok(salida) => salida,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AuthError::GhNotInstalled);
        }
        Err(_) => return Err(AuthError::GhFailed { code: None }),
    };

    interpret_gh_exit(salida.status.code(), &salida.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- interpret_gh_exit: interpretación pura de la salida de `gh` ---

    /// Un código `Some(0)` con salida válida devuelve el token ya recortado.
    #[test]
    fn interpret_gh_exit_exito_devuelve_token_recortado() {
        let token = interpret_gh_exit(Some(0), b"ghp_abc123\n").expect("salida válida");
        assert_eq!(token, "ghp_abc123");
    }

    /// Un código distinto de cero produce `GhFailed` conservando el código.
    #[test]
    fn interpret_gh_exit_codigo_distinto_de_cero_devuelve_gh_failed() {
        let err = interpret_gh_exit(Some(1), b"").expect_err("código 1 debe fallar");
        assert!(matches!(err, AuthError::GhFailed { code: Some(1) }));
    }

    /// Un código ausente (proceso terminado por señal) produce `GhFailed`.
    #[test]
    fn interpret_gh_exit_codigo_ausente_devuelve_gh_failed() {
        let err = interpret_gh_exit(None, b"").expect_err("código ausente debe fallar");
        assert!(matches!(err, AuthError::GhFailed { code: None }));
    }

    /// Salida vacía con éxito produce `GhEmpty`.
    #[test]
    fn interpret_gh_exit_vacio_devuelve_gh_empty() {
        let err = interpret_gh_exit(Some(0), b"").expect_err("salida vacía debe fallar");
        assert!(matches!(err, AuthError::GhEmpty));
    }

    /// Salida compuesta solo de espacios produce `GhEmpty`.
    #[test]
    fn interpret_gh_exit_solo_espacios_devuelve_gh_empty() {
        let err = interpret_gh_exit(Some(0), b"  \n\t ").expect_err("solo espacios debe fallar");
        assert!(matches!(err, AuthError::GhEmpty));
    }

    /// Bytes no UTF-8 producen `GhOutputNotUtf8` en lugar de corromper el token.
    #[test]
    fn interpret_gh_exit_no_utf8_devuelve_gh_output_not_utf8() {
        let err =
            interpret_gh_exit(Some(0), &[0xff, 0xfe, 0x00]).expect_err("no UTF-8 debe fallar");
        assert!(matches!(err, AuthError::GhOutputNotUtf8));
    }

    // --- Token::new: el invariante «token no vacío» lo protege el tipo ---

    /// Un valor vacío se rechaza.
    #[test]
    fn token_new_rechaza_vacio() {
        assert!(matches!(Token::new(""), Err(AuthError::EmptyToken)));
    }

    /// Un valor compuesto solo de espacios se rechaza.
    #[test]
    fn token_new_rechaza_solo_espacios() {
        assert!(matches!(Token::new("   \n\t"), Err(AuthError::EmptyToken)));
    }

    /// Un valor válido se acepta y se recorta.
    #[test]
    fn token_new_recorta_y_acepta_valor_valido() {
        let token = Token::new("  ghp_abc  ").expect("valor válido");
        assert_eq!(token.expose_secret(), "ghp_abc");
    }

    // --- núcleo asíncrono: comparte decisión con la vía síncrona ---

    /// El núcleo asíncrono prefiere el token del entorno.
    #[tokio::test]
    async fn resolve_from_async_prefiere_el_entorno() {
        let token = resolve_from_async(
            Some("ghp_env".to_string()),
            std::future::ready(Ok("ghp_gh".to_string())),
        )
        .await
        .expect("el entorno debe bastar");
        assert_eq!(token.expose_secret(), "ghp_env");
    }

    /// El núcleo asíncrono cae al resultado de `gh` cuando no hay entorno.
    #[tokio::test]
    async fn resolve_from_async_cae_a_gh_sin_entorno() {
        let token = resolve_from_async(None, std::future::ready(Ok("ghp_gh".to_string())))
            .await
            .expect("`gh` debe proporcionar el token");
        assert_eq!(token.expose_secret(), "ghp_gh");
    }

    /// El núcleo asíncrono envuelve el error del runner en `NoCredentials`.
    #[tokio::test]
    async fn resolve_from_async_propaga_error_de_gh() {
        let err = resolve_from_async(None, std::future::ready(Err(AuthError::GhNotInstalled)))
            .await
            .expect_err("sin credenciales debe fallar");
        assert!(matches!(err, AuthError::NoCredentials { .. }));
    }
}
