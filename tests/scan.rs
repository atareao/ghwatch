//! Tests de integración de la orquestación del escaneo.
//!
//! Se usa `wiremock` como servidor falso: la suite no hace red real y el
//! dormidor se sustituye por un doble que no espera, de modo que los reintentos
//! se agotan al instante. Los tests viven en el módulo `scan` para que el filtro
//! `cargo test scan` los seleccione por nombre.

mod scan {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use ghwatch::auth::Token;
    use ghwatch::domain::alert::AlertSource;
    use ghwatch::domain::repo::DependabotState;
    use ghwatch::github::repos::RepoSelection;
    use ghwatch::github::{GithubClient, GithubConfig, Sleeper};
    use ghwatch::scan::{HiddenReporter, ProgressReporter};
    use serde_json::Value;
    use wiremock::matchers::{method, path, path_regex, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Token reconocible: no debe filtrarse, pero aquí basta con que sea válido.
    const TOKEN: &str = "ghp_token_de_prueba";

    /// Índices de los repositorios que fallan tras agotar los reintentos.
    const FALLIDOS: [usize; 2] = [100, 200];

    /// Índice del repositorio con Dependabot deshabilitado (no es un fallo).
    const DEPENDABOT_DESHABILITADO: usize = 70;

    /// Dormidor de doble: no espera nunca, así los reintentos se agotan al
    /// instante y el test no depende de temporizadores reales.
    struct SinEspera;

    impl Sleeper for SinEspera {
        fn sleep(
            &self,
            _duration: Duration,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            Box::pin(async {})
        }
    }

    /// Doble de progreso que no dibuja nada y contabiliza las llamadas.
    #[derive(Default)]
    struct ProgresoContado {
        arranques: AtomicU64,
        total: AtomicU64,
        avances: AtomicU64,
        cierres: AtomicU64,
    }

    impl ProgressReporter for ProgresoContado {
        fn start(&self, total: u64) {
            self.arranques.fetch_add(1, Ordering::SeqCst);
            self.total.store(total, Ordering::SeqCst);
        }

        fn advance(&self, delta: u64) {
            self.avances.fetch_add(delta, Ordering::SeqCst);
        }

        fn finish(&self) {
            self.cierres.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Construye un cliente apuntando al servidor falso y sin esperas reales.
    fn cliente(server: &MockServer) -> GithubClient {
        GithubClient::with_sleeper(
            Token::new(TOKEN).expect("token válido"),
            GithubConfig {
                base_url: server.uri(),
                ..GithubConfig::default()
            },
            Arc::new(SinEspera),
        )
        .expect("el cliente debería construirse sin resolver credenciales")
    }

    /// Construye el JSON de un repositorio del inventario.
    fn repo_json(indice: usize, private: bool, fork: bool, archived: bool) -> Value {
        serde_json::json!({
            "full_name": format!("atareao/repo-{indice:03}"),
            "private": private,
            "archived": archived,
            "fork": fork,
            "language": "Rust",
            "pushed_at": "2026-11-03T09:00:00Z",
        })
    }

    /// Inventario de 433 repositorios de los que 66 son forks (0..40) o
    /// archivados (40..66): la selección por defecto deja 367.
    fn inventario_433() -> Vec<Value> {
        (0..433)
            .map(|indice| {
                let fork = indice < 40;
                let archived = (40..66).contains(&indice);
                repo_json(indice, false, fork, archived)
            })
            .collect()
    }

    /// Monta el inventario repartido en páginas de 100, encadenadas por `Link`.
    async fn montar_inventario(server: &MockServer, repos: &[Value]) {
        let paginas: Vec<&[Value]> = repos.chunks(100).collect();
        let total = paginas.len();

        for (indice, pagina) in paginas.iter().enumerate() {
            let numero = indice + 1;
            let mut mock = Mock::given(method("GET")).and(path("/user/repos"));
            mock = if numero == 1 {
                mock.and(query_param_is_missing("page"))
            } else {
                mock.and(query_param("page", numero.to_string()))
            };

            let mut respuesta = ResponseTemplate::new(200);
            if numero < total {
                let siguiente = format!(
                    "<{uri}/user/repos?per_page=100&page={page}>; rel=\"next\"",
                    uri = server.uri(),
                    page = numero + 1
                );
                respuesta = respuesta.insert_header("Link", siguiente.as_str());
            }
            mock.respond_with(respuesta.set_body_json(pagina))
                .mount(server)
                .await;
        }
    }

    /// Alerta cruda de Dependabot con el número indicado.
    fn alerta_dependabot(numero: u64) -> Value {
        serde_json::json!({
            "number": numero,
            "state": "open",
            "dependency": {
                "package": { "ecosystem": "pip", "name": "django" },
                "manifest_path": "requirements.txt",
                "scope": "runtime"
            },
            "security_advisory": {
                "ghsa_id": "GHSA-aaaa-bbbb-cccc",
                "severity": "high",
                "summary": "Vulnerabilidad de prueba"
            },
            "security_vulnerability": {
                "package": { "ecosystem": "pip", "name": "django" },
                "vulnerable_version_range": "< 3.0.1"
            }
        })
    }

    /// Alerta cruda de secret scanning con el número indicado.
    fn alerta_secreto(numero: u64) -> Value {
        serde_json::json!({
            "number": numero,
            "secret_type": "adafruit_io_key",
            "secret_type_display_name": "Adafruit IO Key"
        })
    }

    /// Alerta cruda de code scanning con el número indicado.
    fn alerta_code(numero: u64) -> Value {
        serde_json::json!({
            "number": numero,
            "rule": {
                "description": "Uso de función insegura",
                "security_severity_level": "high"
            }
        })
    }

    /// Monta las respuestas de recolección de las tres fuentes.
    ///
    /// Los mocks específicos (fallos persistentes y Dependabot deshabilitado) se
    /// montan **antes** que los genéricos: ante igual prioridad gana el primero
    /// registrado, de modo que un repositorio concreto no cae en el genérico.
    async fn montar_senales(server: &MockServer) {
        // Fallo transitorio persistente: `403` con `Retry-After` que no cesa.
        // Tras agotar los reintentos, el cliente devuelve `RateLimited`.
        let fallidos = FALLIDOS
            .iter()
            .map(|indice| format!("repo-{indice:03}"))
            .collect::<Vec<_>>()
            .join("|");
        Mock::given(method("GET"))
            .and(path_regex(format!(
                "^/repos/atareao/({fallidos})/dependabot/alerts$"
            )))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("Retry-After", "30")
                    .set_body_json(serde_json::json!({
                        "message": "You have exceeded a secondary rate limit."
                    })),
            )
            .mount(server)
            .await;

        // Dependabot deshabilitado en un repositorio concreto: `403` con el
        // cuerpo de capacidad deshabilitada, sin `Retry-After`.
        Mock::given(method("GET"))
            .and(path_regex(format!(
                "^/repos/atareao/repo-{DEPENDABOT_DESHABILITADO:03}/dependabot/alerts$"
            )))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "message": "Dependabot alerts are disabled for this repository.",
                "status": "403"
            })))
            .mount(server)
            .await;

        // Genéricos: una alerta por repositorio y fuente.
        Mock::given(method("GET"))
            .and(path_regex("^/repos/atareao/repo-[0-9]+/dependabot/alerts$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![alerta_dependabot(1)]))
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex(
                "^/repos/atareao/repo-[0-9]+/secret-scanning/alerts$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![alerta_secreto(7)]))
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex(
                "^/repos/atareao/repo-[0-9]+/code-scanning/alerts$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![alerta_code(3)]))
            .mount(server)
            .await;
    }

    /// Cuenta las peticiones recibidas en una ruta concreta.
    async fn peticiones_a(server: &MockServer, ruta: &str) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|peticion| peticion.url.path() == ruta)
            .count()
    }

    /// Nombres esperados de los repositorios seleccionados, en el orden del
    /// inventario y sin los que fallan.
    fn seleccionados_esperados() -> Vec<String> {
        (66..433)
            .filter(|indice| !FALLIDOS.contains(indice))
            .map(|indice| format!("atareao/repo-{indice:03}"))
            .collect()
    }

    // --- 3.8 Orquestación: fallos transitorios aislados ---

    /// Escenario «Fallos transitorios aislados»: de 367 repositorios, 2 fallan
    /// tras agotar los reintentos; la ejecución reporta 365 correctos y 2
    /// fallidos listados por nombre, sin perder el estado del resto ni el orden.
    #[tokio::test]
    async fn escaneo_reporta_365_correctos_y_2_fallidos_listados() {
        let server = MockServer::start().await;
        montar_inventario(&server, &inventario_433()).await;
        montar_senales(&server).await;

        let client = Arc::new(cliente(&server));
        let progreso = Arc::new(ProgresoContado::default());

        let resumen = ghwatch::scan::scan(
            Arc::clone(&client),
            RepoSelection::default(),
            Arc::clone(&progreso) as Arc<dyn ProgressReporter>,
        )
        .await
        .expect("el escaneo debería completarse pese a los fallos aislados");

        // 365 correctos y 2 fallidos, y los fallidos se listan por nombre.
        assert_eq!(resumen.succeeded_count(), 365, "365 repositorios correctos");
        assert_eq!(resumen.failed_count(), 2, "2 repositorios fallidos");
        assert_eq!(
            resumen.failed_names(),
            vec!["atareao/repo-100", "atareao/repo-200"],
            "los fallidos se listan por nombre y en orden estable"
        );
        assert!(
            resumen
                .failed
                .iter()
                .all(|fallo| fallo.error.is_rate_limited()),
            "el escenario es de límite agotado: cada fallo debe ser un límite"
        );
        assert!(
            resumen.failed.iter().all(|fallo| !fallo.repo.is_empty()),
            "ningún fallo puede salir con nombre vacío"
        );

        // Un fallo individual no pierde el estado del resto, y el orden es el
        // del inventario.
        let nombres: Vec<&str> = resumen
            .scanned
            .iter()
            .map(|repositorio| repositorio.repo.name_with_owner.as_str())
            .collect();
        let esperados_repos = seleccionados_esperados();
        let esperados: Vec<&str> = esperados_repos.iter().map(String::as_str).collect();
        assert_eq!(nombres, esperados, "el orden de los resultados es estable");

        // El fallo se debe a que se agotaron los reintentos (1 + 3 intentos).
        assert_eq!(
            peticiones_a(&server, "/repos/atareao/repo-100/dependabot/alerts").await,
            4,
            "el repo fallido agota los reintentos antes de rendirse"
        );
    }

    /// El escaneo fusiona el estado de Dependabot de cada repositorio correcto:
    /// `enabled` por defecto y `disabled` cuando GitHub responde `403` con el
    /// cuerpo de capacidad deshabilitada (hallazgo M7).
    #[tokio::test]
    async fn escaneo_fusiona_el_estado_de_dependabot() {
        let server = MockServer::start().await;
        montar_inventario(&server, &inventario_433()).await;
        montar_senales(&server).await;

        let client = Arc::new(cliente(&server));
        let resumen = ghwatch::scan::scan(
            Arc::clone(&client),
            RepoSelection::default(),
            Arc::new(HiddenReporter),
        )
        .await
        .expect("el escaneo debería completarse");

        let deshabilitado = resumen
            .scanned
            .iter()
            .find(|repositorio| repositorio.repo.name_with_owner == "atareao/repo-070")
            .expect("el repositorio con Dependabot deshabilitado debe ser correcto");
        assert_eq!(
            deshabilitado.repo.dependabot,
            DependabotState::Disabled,
            "un 403 de capacidad deshabilitada marca `disabled`, no un fallo"
        );
        assert!(
            deshabilitado.dependabot_alerts.is_empty(),
            "sin capacidad no hay alertas"
        );

        assert!(
            resumen
                .scanned
                .iter()
                .filter(|repositorio| repositorio.repo.name_with_owner != "atareao/repo-070")
                .all(|repositorio| repositorio.repo.dependabot == DependabotState::Enabled),
            "el resto de repositorios quedan con Dependabot `enabled`"
        );
    }

    /// El resumen agrega las alertas normalizadas de las tres fuentes.
    #[tokio::test]
    async fn escaneo_agrega_las_alertas_de_las_tres_fuentes() {
        let server = MockServer::start().await;
        montar_inventario(&server, &inventario_433()).await;
        montar_senales(&server).await;

        let client = Arc::new(cliente(&server));
        let resumen = ghwatch::scan::scan(
            Arc::clone(&client),
            RepoSelection::default(),
            Arc::new(HiddenReporter),
        )
        .await
        .expect("el escaneo debería completarse");

        // 364 repos con Dependabot habilitado + 365 con secreto + 365 con code.
        assert_eq!(
            resumen.alert_count(),
            364 + 365 + 365,
            "el resumen agrega las alertas de las tres fuentes"
        );
        assert!(
            resumen
                .alerts()
                .any(|alerta| alerta.source == AlertSource::Dependabot),
            "deben aparecer alertas de Dependabot"
        );
        assert!(
            resumen
                .alerts()
                .any(|alerta| alerta.source == AlertSource::Secret),
            "deben aparecer alertas de secret scanning"
        );
        assert!(
            resumen
                .alerts()
                .any(|alerta| alerta.source == AlertSource::Code),
            "deben aparecer alertas de code scanning"
        );
    }

    /// El escaneo informa del progreso: arranca con el total, avanza una vez por
    /// repositorio y cierra la barra al terminar.
    #[tokio::test]
    async fn escaneo_informa_del_progreso() {
        let server = MockServer::start().await;
        montar_inventario(&server, &inventario_433()).await;
        montar_senales(&server).await;

        let client = Arc::new(cliente(&server));
        let progreso = Arc::new(ProgresoContado::default());
        ghwatch::scan::scan(
            Arc::clone(&client),
            RepoSelection::default(),
            Arc::clone(&progreso) as Arc<dyn ProgressReporter>,
        )
        .await
        .expect("el escaneo debería completarse");

        assert_eq!(progreso.arranques.load(Ordering::SeqCst), 1);
        assert_eq!(progreso.total.load(Ordering::SeqCst), 367);
        assert_eq!(
            progreso.avances.load(Ordering::SeqCst),
            367,
            "avanza una vez por repositorio, incluidos los fallidos"
        );
        assert_eq!(progreso.cierres.load(Ordering::SeqCst), 1);
    }

    /// Descubrimiento vacío (0 repositorios): el escaneo no falla, devuelve un
    /// resumen vacío y arranca el progreso en 0.
    #[tokio::test]
    async fn escaneo_sin_repositorios_devuelve_resumen_vacio() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user/repos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<Value>::new()))
            .mount(&server)
            .await;

        let client = Arc::new(cliente(&server));
        let progreso = Arc::new(ProgresoContado::default());
        let resumen = ghwatch::scan::scan(
            Arc::clone(&client),
            RepoSelection::default(),
            Arc::clone(&progreso) as Arc<dyn ProgressReporter>,
        )
        .await
        .expect("un inventario vacío no es un error");

        assert_eq!(resumen.succeeded_count(), 0);
        assert_eq!(resumen.failed_count(), 0);
        assert_eq!(resumen.alert_count(), 0);
        assert_eq!(progreso.arranques.load(Ordering::SeqCst), 1);
        assert_eq!(progreso.total.load(Ordering::SeqCst), 0);
        assert_eq!(progreso.avances.load(Ordering::SeqCst), 0);
        assert_eq!(progreso.cierres.load(Ordering::SeqCst), 1);
    }

    /// Todos los repositorios fallan: no hay correctos, se listan los tres
    /// fallidos por nombre (ninguno vacío) y cada error es de límite.
    #[tokio::test]
    async fn escaneo_con_todos_los_repositorios_fallidos_no_pierde_ninguno() {
        let server = MockServer::start().await;
        let inventario: Vec<Value> = (0..3)
            .map(|indice| repo_json(indice, false, false, false))
            .collect();
        montar_inventario(&server, &inventario).await;

        Mock::given(method("GET"))
            .and(path_regex("^/repos/atareao/repo-[0-9]+/dependabot/alerts$"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("Retry-After", "30")
                    .set_body_json(serde_json::json!({
                        "message": "You have exceeded a secondary rate limit."
                    })),
            )
            .mount(&server)
            .await;

        let client = Arc::new(cliente(&server));
        let resumen = ghwatch::scan::scan(
            Arc::clone(&client),
            RepoSelection::default(),
            Arc::new(HiddenReporter),
        )
        .await
        .expect("los fallos por repositorio no abortan el escaneo");

        assert_eq!(
            resumen.succeeded_count(),
            0,
            "ningún repositorio es correcto"
        );
        assert_eq!(resumen.failed_count(), 3, "los tres repositorios fallan");
        assert_eq!(
            resumen.failed_names(),
            vec!["atareao/repo-000", "atareao/repo-001", "atareao/repo-002"],
            "los fallidos se listan por nombre y en orden"
        );
        assert!(
            resumen.failed.iter().all(|fallo| !fallo.repo.is_empty()),
            "ningún fallo puede salir con nombre vacío"
        );
        assert!(
            resumen
                .failed
                .iter()
                .all(|fallo| fallo.error.is_rate_limited()),
            "el escenario es de límite agotado"
        );
    }
}
