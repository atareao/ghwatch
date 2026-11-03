//! Cliente HTTP de la API de GitHub.
//!
//! El cliente recibe un [`Token`] **ya resuelto**: no resuelve credenciales ni
//! hace IO en su constructor, y quien lo construye decide cuándo y cómo obtener
//! la credencial (por ejemplo con [`crate::auth::resolve_token_async`]).
//!
//! Expone dos métodos genéricos pensados para reutilizarse:
//!
//! - [`GithubClient::get_json`], para un único recurso.
//! - [`GithubClient::get_paginated`], que recorre todas las páginas siguiendo
//!   la cabecera `Link` sin truncar el resultado.
//!
//! Sobre ambos aplica el control del límite de peticiones: lee
//! `X-RateLimit-*`, pausa de forma preventiva antes de agotar la cuota,
//! reintenta los `403` de límite secundario respetando `Retry-After` con
//! retroceso exponencial y acota la concurrencia con un semáforo.
//!
//! El tiempo no se duerme directamente: se delega en un [`Sleeper`] inyectable,
//! de modo que los tests pueden comprobar las esperas solicitadas sin esperar.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;
use tokio::task::{Id, JoinError, JoinSet};

use crate::auth::Token;

use super::error::{GithubError, classify_response};
use super::pagination::next_link;
use super::ratelimit::{
    RateLimitState, backoff_delay, parse_retry_after, rate_limit_state, should_pause_before_request,
};

/// URL base por defecto de la API de GitHub.
pub const DEFAULT_BASE_URL: &str = "https://api.github.com";

/// `User-Agent` por defecto de las peticiones.
pub const DEFAULT_USER_AGENT: &str = "ghwatch";

/// Concurrencia por defecto al recorrer colecciones de recursos.
pub const DEFAULT_CONCURRENCY: usize = 6;

/// Número máximo de páginas por colección antes de abortar por seguridad.
///
/// Acota un `Link` mal formado o cíclico: 1000 páginas × 100 elementos cubren
/// de sobra cualquier repositorio real (Dependabot no llega a ese orden de
/// magnitud), así que agotarlo indica un bucle, no un caso legítimo.
pub const DEFAULT_MAX_PAGES: usize = 1000;

/// Media type exigido por la API de GitHub.
const ACCEPT_JSON: &str = "application/vnd.github+json";

/// Versión de la API de GitHub que se solicita.
const API_VERSION: &str = "2022-11-28";

/// Cabecera que fija la versión de la API.
const HEADER_API_VERSION: HeaderName = HeaderName::from_static("x-github-api-version");

/// Cabecera de paginación.
const HEADER_LINK: &str = "link";

/// Cabecera con las peticiones restantes.
const HEADER_REMAINING: &str = "x-ratelimit-remaining";

/// Cabecera con el instante de restablecimiento de la cuota.
const HEADER_RESET: &str = "x-ratelimit-reset";

/// Cabecera con la espera sugerida tras un límite secundario.
const HEADER_RETRY_AFTER: &str = "retry-after";

/// Dormidor inyectable.
///
/// Abstrae la espera para poder sustituirla en los tests por un doble que
/// registra las duraciones solicitadas y devuelve el control al instante. En
/// producción se usa [`TokioSleeper`].
pub trait Sleeper: Send + Sync {
    /// Espera durante `duration`.
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Dormidor de producción, basado en `tokio::time::sleep`.
pub struct TokioSleeper;

impl Sleeper for TokioSleeper {
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// Configuración del cliente de GitHub.
///
/// Todos los campos tienen un valor por defecto sensato en [`GithubConfig::default`];
/// `base_url` es sobrescribible para apuntar a un servidor de pruebas.
#[derive(Debug, Clone)]
pub struct GithubConfig {
    /// URL base de la API (por defecto, la API pública de GitHub).
    pub base_url: String,
    /// Tamaño de página solicitado (`per_page`).
    pub per_page: u32,
    /// Número máximo de reintentos tras un límite secundario.
    pub max_retries: u32,
    /// Umbral de cuota restante por debajo del cual se pausa preventivamente.
    pub rate_limit_pause_threshold: u64,
    /// Tiempo máximo por petición.
    pub timeout: Duration,
    /// Valor de la cabecera `User-Agent`.
    pub user_agent: String,
    /// Máximo de peticiones simultáneas.
    pub concurrency: usize,
    /// Número máximo de páginas que se recorren en una colección paginada.
    ///
    /// Corta un `Link` mal formado o cíclico con
    /// [`GithubError::PaginationLimit`] en lugar de colgar el escaneo.
    pub max_pages: usize,
}

impl Default for GithubConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            per_page: 100,
            max_retries: 3,
            rate_limit_pause_threshold: 100,
            timeout: Duration::from_secs(30),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            concurrency: DEFAULT_CONCURRENCY,
            max_pages: DEFAULT_MAX_PAGES,
        }
    }
}

/// Cliente HTTP de la API de GitHub.
pub struct GithubClient {
    /// Cliente `reqwest` subyacente.
    http: reqwest::Client,
    /// Credencial ya resuelta.
    token: Token,
    /// Configuración.
    config: GithubConfig,
    /// Dormidor inyectable.
    sleeper: Arc<dyn Sleeper>,
    /// Semáforo que acota la concurrencia de peticiones.
    limiter: Arc<Semaphore>,
    /// Último estado de cuota observado, para la pausa preventiva.
    rate_limit: Mutex<RateLimitState>,
}

impl std::fmt::Debug for GithubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // El `Debug` se construye a mano para no exponer el dormidor y para que
        // el token salga redactado (lo redacta el propio `Token`).
        f.debug_struct("GithubClient")
            .field("base_url", &self.config.base_url)
            .field("token", &self.token)
            .field("per_page", &self.config.per_page)
            .field("concurrency", &self.config.concurrency)
            .finish_non_exhaustive()
    }
}

impl GithubClient {
    /// Construye el cliente con el dormidor de producción.
    ///
    /// # Errores
    ///
    /// Devuelve [`GithubError::Transport`] si no se puede inicializar el
    /// cliente HTTP subyacente.
    pub fn new(token: Token, config: GithubConfig) -> Result<Self, GithubError> {
        Self::with_sleeper(token, config, Arc::new(TokioSleeper))
    }

    /// Construye el cliente con un dormidor inyectado.
    ///
    /// Pensado para los tests, que inyectan un doble que registra las esperas.
    ///
    /// # Errores
    ///
    /// Devuelve [`GithubError::Transport`] si no se puede inicializar el
    /// cliente HTTP subyacente.
    pub fn with_sleeper(
        token: Token,
        config: GithubConfig,
        sleeper: Arc<dyn Sleeper>,
    ) -> Result<Self, GithubError> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| GithubError::Transport(error.to_string()))?;
        let limiter = Arc::new(Semaphore::new(config.concurrency.max(1)));

        Ok(Self {
            http,
            token,
            config,
            sleeper,
            limiter,
            rate_limit: Mutex::new(RateLimitState::default()),
        })
    }

    /// Obtiene un recurso y lo deserializa.
    ///
    /// # Errores
    ///
    /// Devuelve el [`GithubError`] correspondiente al estado de la respuesta o
    /// [`GithubError::Transport`] si falla el transporte o la decodificación.
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, GithubError> {
        let url = self.absolute_url(path);
        let response = self.send_with_retries(&url).await?;
        response
            .json::<T>()
            .await
            .map_err(|error| GithubError::Transport(error.to_string()))
    }

    /// Obtiene una colección recorriendo **todas** las páginas por `Link`.
    ///
    /// Añade `per_page` a la ruta si no está ya presente y sigue la relación
    /// `next` hasta agotar las páginas: nunca trunca el resultado al tamaño de
    /// la primera página.
    ///
    /// La paginación de GitHub es por **cursor**: el `Link` puede cambiar el
    /// prefijo de la ruta (por ejemplo, de `/repos/{o}/{r}/dependabot/alerts` a
    /// `/repositories/{id}/dependabot/alerts`) y arrastrar un `after=`. Por eso
    /// se sigue la URL del `Link` **tal cual**, sin recomponerla.
    ///
    /// Por seguridad, se aborta con [`GithubError::PaginationLimit`] si se
    /// supera [`GithubConfig::max_pages`] o si el servidor vuelve a ofrecer una
    /// URL ya visitada.
    ///
    /// # Errores
    ///
    /// Los mismos que [`GithubClient::get_json`], aplicados a cualquier página,
    /// más [`GithubError::PaginationLimit`] si la cadena de `Link` no es finita.
    pub async fn get_paginated<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<Vec<T>, GithubError> {
        let mut url = self.absolute_url(&with_per_page(path, self.config.per_page));
        let mut elementos: Vec<T> = Vec::new();
        let mut visitadas: HashSet<String> = HashSet::new();
        let mut paginas: usize = 0;

        loop {
            if paginas >= self.config.max_pages {
                return Err(GithubError::PaginationLimit { pages: paginas });
            }

            let response = self.send_with_retries(&url).await?;
            visitadas.insert(url.clone());
            paginas += 1;

            let siguiente = next_link(header_str(&response, HEADER_LINK).as_deref());

            let pagina: Vec<T> = response
                .json()
                .await
                .map_err(|error| GithubError::Transport(error.to_string()))?;
            elementos.extend(pagina);

            match siguiente {
                Some(siguiente) => {
                    if visitadas.contains(&siguiente) {
                        return Err(GithubError::PaginationLimit { pages: paginas });
                    }
                    url = siguiente;
                }
                None => break,
            }
        }

        Ok(elementos)
    }

    /// Devuelve el semáforo que acota la concurrencia, para orquestar barridos
    /// de varias colecciones.
    #[must_use]
    pub fn limiter(&self) -> Arc<Semaphore> {
        Arc::clone(&self.limiter)
    }

    /// Aplica `f` a cada elemento con concurrencia acotada por el semáforo.
    ///
    /// Conserva el orden de `items`: la salida en la posición `i` corresponde al
    /// elemento `i`. Cada posición es un `Result`: `Ok(salida)` si la tarea se
    /// completó, `Err(JoinError)` si la tarea falló a nivel de runtime (pánico o
    /// cancelación). Los fallos **no** se descartan: el llamante decide qué hacer
    /// con ellos.
    pub async fn for_each_concurrent<T, F, Fut, O>(
        &self,
        items: Vec<T>,
        f: F,
    ) -> Vec<Result<O, JoinError>>
    where
        T: Send + 'static,
        F: Fn(T) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: Send + 'static,
    {
        let tarea = Arc::new(f);
        let mut conjunto: JoinSet<(usize, O)> = JoinSet::new();
        let mut indices: HashMap<Id, usize> = HashMap::new();

        let total = items.len();
        for (indice, item) in items.into_iter().enumerate() {
            let tarea = Arc::clone(&tarea);
            let limiter = Arc::clone(&self.limiter);
            let handle = conjunto.spawn(async move {
                // `acquire` solo falla si el semáforo se cierra, algo que aquí no
                // ocurre: en ese caso se ejecuta sin permiso para no perder trabajo.
                let _permiso = limiter.acquire().await.ok();
                (indice, tarea(item).await)
            });
            indices.insert(handle.id(), indice);
        }

        let mut pares: Vec<(usize, Result<O, JoinError>)> = Vec::with_capacity(total);
        while let Some(resultado) = conjunto.join_next_with_id().await {
            match resultado {
                Ok((_, (indice, salida))) => pares.push((indice, Ok(salida))),
                Err(error) => {
                    // Un `JoinError` no dice a qué elemento correspondía, pero el
                    // `Id` de la tarea sí: se guardó al lanzarla.
                    let indice = indices.get(&error.id()).copied().unwrap_or(usize::MAX);
                    pares.push((indice, Err(error)));
                }
            }
        }
        pares.sort_by_key(|(indice, _)| *indice);

        pares.into_iter().map(|(_, salida)| salida).collect()
    }

    /// Envía una petición `GET` aplicando pausa preventiva y reintentos.
    ///
    /// Devuelve la respuesta solo si es de éxito (`status < 400`); en otro caso
    /// devuelve el error tipado correspondiente.
    async fn send_with_retries(&self, url: &str) -> Result<reqwest::Response, GithubError> {
        let mut intento: u32 = 0;

        loop {
            self.pause_if_needed().await;

            let response = self
                .http
                .get(url)
                .headers(self.request_headers()?)
                .send()
                .await
                .map_err(|error| GithubError::Transport(error.to_string()))?;

            self.record_rate_limit(&response);

            let status = response.status().as_u16();
            if status < 400 {
                return Ok(response);
            }

            let retry_after =
                parse_retry_after(header_str(&response, HEADER_RETRY_AFTER).as_deref());
            let body = response
                .text()
                .await
                .map_err(|error| GithubError::Transport(error.to_string()))?;

            match classify_response(status, &body, retry_after) {
                Err(GithubError::RateLimited { retry_after })
                    if intento < self.config.max_retries =>
                {
                    let espera = retry_after.unwrap_or_else(|| backoff_delay(intento));
                    self.sleeper.sleep(espera).await;
                    intento += 1;
                }
                Err(error) => return Err(error),
                // `classify_response` nunca acepta un estado >= 400; defensivo.
                Ok(()) => return Err(GithubError::Http { status, body }),
            }
        }
    }

    /// Pausa si la última cuota observada está por debajo del umbral.
    async fn pause_if_needed(&self) {
        let estado = self.current_rate_limit();
        if let Some(espera) =
            should_pause_before_request(&estado, self.config.rate_limit_pause_threshold)
        {
            self.sleeper.sleep(espera).await;
        }
    }

    /// Guarda el estado de cuota leído de una respuesta.
    ///
    /// Si la respuesta no trae ninguna de las cabeceras de cuota, conserva el
    /// estado anterior en lugar de olvidarlo.
    fn record_rate_limit(&self, response: &reqwest::Response) {
        let remaining = header_str(response, HEADER_REMAINING);
        let reset = header_str(response, HEADER_RESET);
        if remaining.is_none() && reset.is_none() {
            return;
        }

        let estado = rate_limit_state(remaining.as_deref(), reset.as_deref(), now_epoch());
        self.store_rate_limit(estado);
    }

    /// Sustituye el estado de cuota almacenado.
    fn store_rate_limit(&self, estado: RateLimitState) {
        match self.rate_limit.lock() {
            Ok(mut actual) => *actual = estado,
            // Un `Mutex` envenenado no debe impedir seguir: se recupera el dato.
            Err(poisoned) => *poisoned.into_inner() = estado,
        }
    }

    /// Lee el estado de cuota almacenado.
    fn current_rate_limit(&self) -> RateLimitState {
        match self.rate_limit.lock() {
            Ok(actual) => *actual,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    /// Construye las cabeceras exigidas por la API de GitHub.
    ///
    /// Es el **único** punto donde se expone el token, copiándolo a la cabecera
    /// `Authorization`.
    fn request_headers(&self) -> Result<HeaderMap, GithubError> {
        let mut cabeceras = HeaderMap::new();
        cabeceras.insert(header::ACCEPT, HeaderValue::from_static(ACCEPT_JSON));
        cabeceras.insert(HEADER_API_VERSION, HeaderValue::from_static(API_VERSION));
        cabeceras.insert(header::USER_AGENT, header_value(&self.config.user_agent)?);

        // Única exposición deliberada del secreto.
        let autorizacion = format!("Bearer {}", self.token.expose_secret());
        cabeceras.insert(header::AUTHORIZATION, header_value(&autorizacion)?);

        Ok(cabeceras)
    }

    /// Compone la URL absoluta a partir de la ruta o de una URL ya absoluta
    /// (como las que devuelve la cabecera `Link`).
    fn absolute_url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            return path.to_string();
        }

        let base = self.config.base_url.trim_end_matches('/');
        if path.starts_with('/') {
            format!("{base}{path}")
        } else {
            format!("{base}/{path}")
        }
    }
}

/// Añade `per_page` a la ruta si no está ya presente.
///
/// La consulta se analiza como pares `clave=valor` separados por `&`; solo se
/// considera presente un parámetro cuya **clave** sea exactamente `per_page`.
/// Así no se confunde con un valor como `per_page=3` dentro de otro parámetro
/// (`otro=per_page=3`) ni con una clave que contenga la subcadena
/// (`not_per_page`).
fn with_per_page(path: &str, per_page: u32) -> String {
    let ya_presente = path.split_once('?').is_some_and(|(_, consulta)| {
        consulta.split('&').any(|par| match par.split_once('=') {
            Some((clave, _)) => clave == "per_page",
            None => par == "per_page",
        })
    });

    if ya_presente {
        path.to_string()
    } else if path.contains('?') {
        format!("{path}&per_page={per_page}")
    } else {
        format!("{path}?per_page={per_page}")
    }
}

/// Convierte una cadena en valor de cabecera.
fn header_value(valor: &str) -> Result<HeaderValue, GithubError> {
    HeaderValue::from_str(valor)
        .map_err(|_| GithubError::Transport("valor de cabecera no válido".to_string()))
}

/// Lee una cabecera de la respuesta como texto.
fn header_str(response: &reqwest::Response, nombre: &str) -> Option<String> {
    response
        .headers()
        .get(nombre)
        .and_then(|valor| valor.to_str().ok())
        .map(str::to_string)
}

/// Instante actual en segundos desde la época UNIX.
fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|transcurrido| transcurrido.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construye un cliente sin tocar la red.
    fn cliente_de_prueba() -> GithubClient {
        GithubClient::new(
            Token::new("ghp_token_de_prueba").expect("token válido"),
            GithubConfig::default(),
        )
        .expect("el cliente debería construirse")
    }

    /// `with_per_page` respeta una ruta sin consulta, con consulta y con
    /// `per_page` ya presente; y no confunde otro parámetro que contenga la
    /// subcadena `per_page=` en su valor.
    #[test]
    fn with_per_page_compone_la_consulta() {
        assert_eq!(with_per_page("/items", 100), "/items?per_page=100");
        assert_eq!(
            with_per_page("/items?page=2", 100),
            "/items?page=2&per_page=100"
        );
        assert_eq!(
            with_per_page("/items?per_page=50", 100),
            "/items?per_page=50"
        );
        assert_eq!(
            with_per_page("/items?otro=per_page=3", 100),
            "/items?otro=per_page=3&per_page=100"
        );
        assert_eq!(
            with_per_page("/items?not_per_page=1", 100),
            "/items?not_per_page=1&per_page=100"
        );
    }

    /// `absolute_url` acepta rutas relativas y URLs ya absolutas.
    #[tokio::test]
    async fn absolute_url_compone_y_respeta_urls_absolutas() {
        let cliente = cliente_de_prueba();
        assert_eq!(
            cliente.absolute_url("/items"),
            format!("{DEFAULT_BASE_URL}/items")
        );
        assert_eq!(
            cliente.absolute_url("https://example.com/x"),
            "https://example.com/x"
        );
    }

    /// `for_each_concurrent` aplica la tarea a todos y conserva el orden.
    #[tokio::test]
    async fn for_each_concurrent_conserva_el_orden() {
        let cliente = cliente_de_prueba();
        let salidas: Vec<u32> = cliente
            .for_each_concurrent(vec![1u32, 2, 3, 4], |numero| async move { numero * 10 })
            .await
            .into_iter()
            .map(|salida| salida.expect("la tarea debería completarse"))
            .collect();
        assert_eq!(salidas, vec![10, 20, 30, 40]);
    }

    /// `for_each_concurrent` no se traga los fallos: un pánico en una tarea se
    /// devuelve como `Err` en su posición, sin perderse.
    #[tokio::test]
    async fn for_each_concurrent_expone_los_fallos() {
        let cliente = cliente_de_prueba();
        let salidas = cliente
            .for_each_concurrent(vec![1u32, 2, 3], |numero| async move {
                assert!(numero != 2, "pánico deliberado en la tarea del 2");
                numero * 10
            })
            .await;

        assert_eq!(salidas.len(), 3, "deben devolverse todas las posiciones");
        assert!(salidas[0].is_ok(), "la tarea 1 debería completarse");
        assert!(
            salidas[1].is_err(),
            "el fallo de la tarea 2 debería aflorar como `Err`"
        );
        assert!(salidas[2].is_ok(), "la tarea 3 debería completarse");
    }

    /// El `Debug` del cliente redacta el token.
    #[tokio::test]
    async fn el_debug_del_cliente_redacta_el_token() {
        let cliente = cliente_de_prueba();
        let texto = format!("{cliente:?}");
        assert!(
            !texto.contains("ghp_token_de_prueba"),
            "el Debug filtra el token: {texto}"
        );
        assert!(texto.contains("GithubClient"));
    }
}
