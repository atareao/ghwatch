//! Tests de integración del cliente HTTP de GitHub.
//!
//! Se usa `wiremock` como servidor falso y un dormidor de doble que registra las
//! esperas solicitadas sin dormir de verdad: el tiempo no se duerme, se
//! comprueba. Los tests viven en el módulo `github` para que el filtro
//! `cargo test github` los seleccione por nombre.

mod github {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use ghwatch::auth::Token;
    use ghwatch::github::{GithubClient, GithubConfig, GithubError, Sleeper};
    use serde::Deserialize;
    use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Elemento mínimo devuelto por los endpoints paginados.
    #[derive(Debug, Deserialize)]
    struct Item {
        id: u64,
    }

    /// Dormidor de doble: registra las duraciones solicitadas y no espera.
    struct RecordingSleeper {
        requested: Mutex<Vec<Duration>>,
    }

    impl RecordingSleeper {
        fn new() -> Self {
            Self {
                requested: Mutex::new(Vec::new()),
            }
        }

        /// Duraciones solicitadas, en orden.
        fn requested(&self) -> Vec<Duration> {
            self.requested.lock().expect("mutex no envenenado").clone()
        }
    }

    impl Sleeper for RecordingSleeper {
        fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.requested
                .lock()
                .expect("mutex no envenenado")
                .push(duration);
            Box::pin(async {})
        }
    }

    /// Construye un cliente apuntando al servidor falso, con dormidor de doble.
    fn cliente(server: &MockServer) -> (GithubClient, Arc<RecordingSleeper>) {
        let sleeper = Arc::new(RecordingSleeper::new());
        let client = GithubClient::with_sleeper(
            Token::new(TOKEN).expect("token válido"),
            GithubConfig {
                base_url: server.uri(),
                ..GithubConfig::default()
            },
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
        )
        .expect("el cliente debería construirse sin resolver credenciales");
        (client, sleeper)
    }

    /// Token reconocible que los tests usan para comprobar que no se filtra.
    const TOKEN: &str = "ghp_token_de_prueba";

    /// Instante actual en segundos desde la época UNIX.
    fn epoca_actual() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("el reloj debe ir después de la época UNIX")
            .as_secs()
    }

    // --- 2.1 Paginación por `Link` ---

    /// Dos páginas encadenadas por un `Link` **con forma real de cursor** (otro
    /// prefijo de ruta y `after=`) deben devolver los elementos de ambas, y la
    /// segunda petición debe ir **exactamente** a la URL del `Link`.
    ///
    /// La página 1 se monta en la ruta original y la página 2 **solo** bajo la
    /// ruta del cursor: una implementación que recompusiera la URL en lugar de
    /// seguir el `Link` crudo no casaría ningún mock y fallaría. La página 1 se
    /// sirve una sola vez (`up_to_n_times(1)`) para que esa recomposición acabe
    /// en un `404` acotado en vez de colgar el test en un bucle infinito.
    #[tokio::test]
    async fn paginacion_sigue_la_url_del_link_con_forma_de_cursor() {
        let server = MockServer::start().await;
        // El prefijo del `Link` es distinto del de la petición original.
        let siguiente_url = format!(
            "{uri}/repositories/459293568/dependabot/alerts?per_page=100&after=Y3Vyc29yOnYyOpPPAAAB",
            uri = server.uri()
        );
        let link = format!("<{siguiente_url}>; rel=\"next\"");

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/dependabot/alerts"))
            .and(query_param("state", "open"))
            .and(query_param_is_missing("after"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", link.as_str())
                    .set_body_json(serde_json::json!([{ "id": 1 }, { "id": 2 }])),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repositories/459293568/dependabot/alerts"))
            .and(query_param("after", "Y3Vyc29yOnYyOpPPAAAB"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([{ "id": 3 }])),
            )
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let items: Vec<Item> = client
            .get_paginated("/repos/atareao/reto-python/dependabot/alerts?state=open")
            .await
            .expect("la paginación debería recorrer las dos páginas siguiendo el `Link`");

        assert_eq!(
            items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        let peticiones = server
            .received_requests()
            .await
            .expect("peticiones recibidas");
        assert_eq!(peticiones.len(), 2, "deben ser exactamente dos peticiones");
        // wiremock reconstruye la URL con host `localhost` (el URI de origen no
        // lleva autoridad), así que la comparación exacta se hace sobre la ruta
        // y la consulta, que es lo que distingue seguir el `Link` de recomponerlo.
        assert_eq!(
            peticiones[1].url.path(),
            "/repositories/459293568/dependabot/alerts",
            "la segunda petición debe usar el prefijo del `Link`"
        );
        assert_eq!(
            peticiones[1].url.query(),
            Some("per_page=100&after=Y3Vyc29yOnYyOpPPAAAB"),
            "la segunda petición debe arrastrar la cadena de consulta del `Link` tal cual"
        );
    }

    /// Una página sin `rel="next"` no debe provocar ninguna petición adicional.
    #[tokio::test]
    async fn pagina_sin_rel_next_no_provoca_peticion_adicional() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/items"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([{ "id": 1 }])),
            )
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let items: Vec<Item> = client
            .get_paginated("/items")
            .await
            .expect("la paginación de una sola página debería funcionar");

        assert_eq!(items.len(), 1);
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("peticiones recibidas")
                .len(),
            1
        );
    }

    // --- 2.2 Cabeceras exigidas ---

    /// La petición solo casa si lleva todas las cabeceras exigidas; además se
    /// asevera sobre las peticiones realmente recibidas.
    #[tokio::test]
    async fn envia_las_cabeceras_exigidas() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/ping"))
            .and(header("user-agent", "ghwatch"))
            .and(header("accept", "application/vnd.github+json"))
            .and(header("x-github-api-version", "2022-11-28"))
            .and(header("authorization", "Bearer ghp_token_de_prueba"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })),
            )
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let valor: serde_json::Value = client
            .get_json("/ping")
            .await
            .expect("si faltara alguna cabecera, el mock no casaría y la API respondería 404");

        assert_eq!(valor["ok"], serde_json::json!(true));

        let peticiones = server
            .received_requests()
            .await
            .expect("peticiones recibidas");
        assert_eq!(peticiones.len(), 1);
        let cabeceras = &peticiones[0].headers;

        let valor_cabecera = |nombre: &str| {
            cabeceras
                .get(nombre)
                .unwrap_or_else(|| panic!("falta la cabecera {nombre}"))
                .to_str()
                .unwrap_or_else(|_| panic!("la cabecera {nombre} no es ASCII"))
                .to_string()
        };

        assert_eq!(valor_cabecera("user-agent"), "ghwatch");
        assert_eq!(valor_cabecera("accept"), "application/vnd.github+json");
        assert_eq!(valor_cabecera("x-github-api-version"), "2022-11-28");
        assert_eq!(
            valor_cabecera("authorization"),
            "Bearer ghp_token_de_prueba"
        );
    }

    // --- 2.3 Autenticación rechazada ---

    /// Un `401` se traduce a `Unauthorized`.
    #[tokio::test]
    async fn respuesta_401_es_unauthorized() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(serde_json::json!({ "message": "Bad credentials" })),
            )
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let err = client
            .get_json::<serde_json::Value>("/user")
            .await
            .expect_err("un 401 debe producir error");

        assert!(
            matches!(err, GithubError::Unauthorized),
            "se esperaba Unauthorized, no: {err:?}"
        );
    }

    // --- 2.4 Límite secundario y reintentos ---

    /// Un `403` con `Retry-After: 30` servido una sola vez se reintenta con
    /// éxito y el dormidor registra la espera de 30 s.
    #[tokio::test]
    async fn reintenta_tras_403_con_retry_after_y_solicita_la_espera() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/recurso"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("Retry-After", "30")
                    .set_body_json(
                        serde_json::json!({ "message": "You have exceeded a secondary rate limit" }),
                    ),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/recurso"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })),
            )
            .with_priority(2)
            .mount(&server)
            .await;

        let (client, sleeper) = cliente(&server);
        let valor: serde_json::Value = client
            .get_json("/recurso")
            .await
            .expect("debería reintentar y tener éxito");

        assert_eq!(valor["ok"], serde_json::json!(true));
        let esperas = sleeper.requested();
        assert!(
            esperas.contains(&Duration::from_secs(30)),
            "no se solicitó la espera de 30 s: {esperas:?}"
        );
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("peticiones recibidas")
                .len(),
            2
        );
    }

    /// Un `403` con `Retry-After` persistente devuelve `RateLimited` tras
    /// agotar `max_retries`.
    #[tokio::test]
    async fn rate_limited_persistente_tras_agotar_reintentos() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/recurso"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("Retry-After", "30")
                    .set_body_json(serde_json::json!({ "message": "secondary rate limit" })),
            )
            .mount(&server)
            .await;

        let (client, sleeper) = cliente(&server);
        let err = client
            .get_json::<serde_json::Value>("/recurso")
            .await
            .expect_err("un 403 persistente debe acabar en error");

        match err {
            GithubError::RateLimited { retry_after } => {
                assert_eq!(retry_after, Some(Duration::from_secs(30)));
            }
            otro => panic!("se esperaba RateLimited, no: {otro:?}"),
        }

        // Intento inicial + 3 reintentos (el máximo por defecto).
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("peticiones recibidas")
                .len(),
            4
        );
        assert_eq!(sleeper.requested().len(), 3);
    }

    // --- 2.5 Tolerancia a capacidades no habilitadas ---

    /// Un `404` con `no analysis found` indica que la capacidad no está
    /// configurada.
    #[tokio::test]
    async fn no_analysis_found_es_not_configured() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/acme/demo/code-scanning/alerts"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({ "message": "no analysis found" })),
            )
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let err = client
            .get_json::<serde_json::Value>("/repos/acme/demo/code-scanning/alerts")
            .await
            .expect_err("un 404 debe producir error");

        assert!(
            matches!(err, GithubError::NotConfigured),
            "se esperaba NotConfigured, no: {err:?}"
        );
    }

    /// Un `404` con otro cuerpo es un `NotFound` genérico.
    #[tokio::test]
    async fn not_found_con_otro_cuerpo_es_not_found() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/acme/inexistente"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({ "message": "Not Found" })),
            )
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let err = client
            .get_json::<serde_json::Value>("/repos/acme/inexistente")
            .await
            .expect_err("un 404 debe producir error");

        assert!(
            matches!(err, GithubError::NotFound),
            "se esperaba NotFound, no: {err:?}"
        );
    }

    /// Un `403` con el cuerpo literal de Dependabot deshabilitado se traduce a
    /// `Disabled` (capacidad deshabilitada), no a un `Http` genérico.
    #[tokio::test]
    async fn respuesta_403_con_cuerpo_deshabilitado_es_disabled() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/atareao/gitignore/dependabot/alerts"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "message": "Dependabot alerts are disabled for this repository.",
                "documentation_url": "https://docs.github.com/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository",
                "status": "403"
            })))
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let err = client
            .get_json::<serde_json::Value>("/repos/atareao/gitignore/dependabot/alerts")
            .await
            .expect_err("un 403 debe producir error");

        assert!(
            matches!(err, GithubError::Disabled),
            "se esperaba Disabled, no: {err:?}"
        );
    }

    /// Un `403` con otro cuerpo no es «deshabilitado»: es un `Http` con estado
    /// 403.
    #[tokio::test]
    async fn respuesta_403_con_otro_cuerpo_es_http() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/acme/demo/otra-cosa"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(serde_json::json!({ "message": "Resource not accessible" })),
            )
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let err = client
            .get_json::<serde_json::Value>("/repos/acme/demo/otra-cosa")
            .await
            .expect_err("un 403 debe producir error");

        match err {
            GithubError::Http { status, .. } => assert_eq!(status, 403),
            otro => panic!("se esperaba Http, no: {otro:?}"),
        }
    }

    // --- S2: corte de seguridad de la paginación ---

    /// Si el servidor repite el mismo `Link`, la paginación aborta con un error
    /// acotado en lugar de entrar en un bucle infinito.
    #[tokio::test]
    async fn paginacion_con_link_repetido_aborta_sin_colgar() {
        let server = MockServer::start().await;
        let repetido = format!("<{}/items?per_page=100>; rel=\"next\"", server.uri());

        Mock::given(method("GET"))
            .and(path("/items"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", repetido.as_str())
                    .set_body_json(serde_json::json!([{ "id": 1 }])),
            )
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let err = client
            .get_paginated::<Item>("/items")
            .await
            .expect_err("un `Link` repetido debe abortar");

        assert!(
            matches!(err, GithubError::PaginationLimit { .. }),
            "se esperaba PaginationLimit, no: {err:?}"
        );
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("peticiones recibidas")
                .len(),
            1,
            "no debe repetirse la petición a la misma URL"
        );
    }

    /// `max_pages` acota el número de páginas: al alcanzarlo, error acotado en
    /// lugar de seguir pidiendo.
    #[tokio::test]
    async fn paginacion_respeta_max_pages() {
        let server = MockServer::start().await;
        let p2 = format!("<{}/items?after=2>; rel=\"next\"", server.uri());
        let p3 = format!("<{}/items?after=3>; rel=\"next\"", server.uri());
        let p4 = format!("<{}/items?after=4>; rel=\"next\"", server.uri());

        Mock::given(method("GET"))
            .and(path("/items"))
            .and(query_param_is_missing("after"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", p2.as_str())
                    .set_body_json(serde_json::json!([{ "id": 1 }])),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/items"))
            .and(query_param("after", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", p3.as_str())
                    .set_body_json(serde_json::json!([{ "id": 2 }])),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/items"))
            .and(query_param("after", "3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", p4.as_str())
                    .set_body_json(serde_json::json!([{ "id": 3 }])),
            )
            .mount(&server)
            .await;

        let config = GithubConfig {
            base_url: server.uri(),
            max_pages: 2,
            ..GithubConfig::default()
        };
        let sleeper = Arc::new(RecordingSleeper::new());
        let client = GithubClient::with_sleeper(
            Token::new("ghp_token_de_prueba").expect("token válido"),
            config,
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
        )
        .expect("el cliente debería construirse");

        let err = client
            .get_paginated::<Item>("/items")
            .await
            .expect_err("superar `max_pages` debe abortar");

        assert!(
            matches!(err, GithubError::PaginationLimit { pages } if pages == 2),
            "se esperaba PaginationLimit con 2 páginas, no: {err:?}"
        );
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("peticiones recibidas")
                .len(),
            2,
            "no debe pedirse una página más allá del máximo"
        );
    }

    // --- 2.6 Pausa preventiva por cuota baja ---
    /// Con `X-RateLimit-Remaining` por debajo del umbral, antes de la siguiente
    /// petición se solicita al dormidor una espera coherente con `Reset`.
    #[tokio::test]
    async fn pausa_preventiva_cuando_la_cuota_baja_del_umbral() {
        let server = MockServer::start().await;
        let siguiente = format!("<{}/items?page=2>; rel=\"next\"", server.uri());
        let reset = epoca_actual() + 30;
        let reset_valor = reset.to_string();

        Mock::given(method("GET"))
            .and(path("/items"))
            .and(query_param("per_page", "100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("X-RateLimit-Remaining", "5")
                    .insert_header("X-RateLimit-Reset", reset_valor.as_str())
                    .insert_header("Link", siguiente.as_str())
                    .set_body_json(serde_json::json!([{ "id": 1 }])),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/items"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([{ "id": 2 }])),
            )
            .mount(&server)
            .await;

        let (client, sleeper) = cliente(&server);
        let items: Vec<Item> = client.get_paginated("/items").await.expect("paginación");

        assert_eq!(items.len(), 2);
        let esperas = sleeper.requested();
        assert!(
            esperas
                .iter()
                .any(|espera| *espera >= Duration::from_secs(25)
                    && *espera <= Duration::from_secs(30)),
            "no se solicitó una espera coherente con el reset: {esperas:?}"
        );
    }

    // --- Error genérico del servidor ---

    /// Un `500` se traduce a `Http` conservando el estado y el cuerpo.
    #[tokio::test]
    async fn respuesta_500_es_http_con_estado_y_cuerpo() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/boom"))
            .respond_with(
                ResponseTemplate::new(500).set_body_json(serde_json::json!({ "message": "boom" })),
            )
            .mount(&server)
            .await;

        let (client, _sleeper) = cliente(&server);
        let err = client
            .get_json::<serde_json::Value>("/boom")
            .await
            .expect_err("un 500 debe producir error");

        match err {
            GithubError::Http { status, body } => {
                assert_eq!(status, 500);
                assert!(
                    body.contains("boom"),
                    "el cuerpo debería conservarse: {body}"
                );
            }
            otro => panic!("se esperaba Http, no: {otro:?}"),
        }
    }

    // --- El token nunca se filtra ---

    /// Ni el `Debug` del cliente ni ningún mensaje de error exponen el token.
    #[tokio::test]
    async fn el_token_no_aparece_en_los_errores() {
        const SECRETO: &str = "ghp_SECRETO_IRRECONOCIBLE_1234567890";
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(serde_json::json!({ "message": "Bad credentials" })),
            )
            .mount(&server)
            .await;

        let sleeper = Arc::new(RecordingSleeper::new());
        let client = GithubClient::with_sleeper(
            Token::new(SECRETO).expect("token válido"),
            GithubConfig {
                base_url: server.uri(),
                ..GithubConfig::default()
            },
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
        )
        .expect("el cliente debería construirse");

        let err = client
            .get_json::<serde_json::Value>("/user")
            .await
            .expect_err("un 401 debe producir error");

        assert!(
            !err.to_string().contains(SECRETO),
            "el Display del error filtra el token: {err}"
        );
        assert!(
            !format!("{err:?}").contains(SECRETO),
            "el Debug del error filtra el token: {err:?}"
        );
        assert!(
            !format!("{client:?}").contains(SECRETO),
            "el Debug del cliente filtra el token"
        );
    }
}
