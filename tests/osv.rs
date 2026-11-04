//! Tests de integración del enriquecimiento OSV.
//!
//! OSV se simula con `wiremock`: la suite no hace red real. El cliente apunta a
//! la URL base del servidor falso, de modo que el mismo código de producción se
//! ejercita sin credenciales. Los tests viven en el módulo `osv` para que el
//! filtro `cargo test osv` los seleccione por nombre.

mod osv {
    use std::io::Write;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use chrono::{DateTime, Utc};
    use ghwatch::auth::Token;
    use ghwatch::domain::alert::{Alert, AlertSource, Severity};
    use ghwatch::github::repos::RepoSelection;
    use ghwatch::github::{GithubClient, GithubConfig, Sleeper};
    use ghwatch::osv::{EnrichmentOutcome, OsvClient, OsvConfig, OsvQuery, map_ecosystem};
    use ghwatch::scan::HiddenReporter;
    use serde_json::{Value, json};
    use tracing::Level;
    use wiremock::matchers::{body_partial_json, method, path, path_regex};
    use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

    /// Token reconocible: no debe filtrarse, pero aquí basta con que sea válido.
    const TOKEN: &str = "ghp_token_de_prueba";

    /// Ruta del endpoint de lotes de OSV.
    const RUTA_LOTE: &str = "/v1/querybatch";

    /// Dormidor de doble: no espera nunca.
    struct SinEspera;

    impl Sleeper for SinEspera {
        fn sleep(
            &self,
            _duration: Duration,
        ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            Box::pin(async {})
        }
    }

    /// Cliente OSV apuntando al servidor falso.
    fn osv(server: &MockServer) -> OsvClient {
        OsvClient::new(OsvConfig {
            base_url: server.uri(),
            ..OsvConfig::default()
        })
        .expect("el cliente OSV debería construirse")
    }

    /// Cliente OSV con un tiempo de espera agotado muy corto.
    fn osv_con_timeout(server: &MockServer, timeout: Duration) -> OsvClient {
        OsvClient::new(OsvConfig {
            base_url: server.uri(),
            timeout,
            ..OsvConfig::default()
        })
        .expect("el cliente OSV debería construirse")
    }

    /// Cliente GitHub apuntando al servidor falso.
    fn github(server: &MockServer) -> GithubClient {
        GithubClient::with_sleeper(
            Token::new(TOKEN).expect("token válido"),
            GithubConfig {
                base_url: server.uri(),
                ..GithubConfig::default()
            },
            Arc::new(SinEspera),
        )
        .expect("el cliente GitHub debería construirse")
    }

    /// Alerta de Dependabot de `django` con el identificador indicado.
    fn alerta_dependabot(external_id: &str) -> Alert {
        Alert {
            source: AlertSource::Dependabot,
            external_id: external_id.to_string(),
            ghsa_id: None,
            cve_id: None,
            severity: Some(Severity::High),
            cvss: None,
            summary: None,
            package: Some("django".to_string()),
            ecosystem: Some("pip".to_string()),
            manifest_path: None,
            scope: Some("runtime".to_string()),
            vulnerable_range: None,
            first_patched: None,
            secret_type: None,
            created_at: None,
        }
    }

    /// Aviso de OSV con alias CVE, vector CVSS v3 y versión corregida.
    fn aviso_osv() -> Value {
        json!([{
            "id": "GHSA-aaaa-bbbb-cccc",
            "aliases": ["CVE-2021-1234"],
            "severity": [{
                "type": "CVSS_V3",
                "score": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"
            }],
            "affected": [{
                "package": { "name": "django", "ecosystem": "PyPI" },
                "ranges": [{
                    "type": "ECOSYSTEM",
                    "events": [{ "introduced": "0" }, { "fixed": "3.0.2" }]
                }]
            }]
        }])
    }

    /// Monta una respuesta de lote de OSV con los avisos indicados.
    async fn montar_osv(server: &MockServer, vulns: Value) {
        Mock::given(method("POST"))
            .and(path(RUTA_LOTE))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "results": [{ "vulns": vulns }] })),
            )
            .mount(server)
            .await;
    }

    /// Peticiones recibidas en el endpoint de lotes de OSV.
    async fn peticiones_de_lote(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|peticion| peticion.url.path() == RUTA_LOTE)
            .count()
    }

    // --- 5.1 Consulta en lotes ---

    /// Escenario «Volumen que requiere dos lotes»: 1500 pares (ecosistema,
    /// paquete, versión) se resuelven con exactamente 2 peticiones a
    /// `POST /v1/querybatch`.
    #[tokio::test]
    async fn mil_quinientos_pares_usan_exactamente_dos_peticiones() {
        let server = MockServer::start().await;
        montar_osv(&server, json!([])).await;

        let cliente = osv(&server);
        let consultas: Vec<OsvQuery> = (0..1500)
            .map(|indice| {
                OsvQuery::new(
                    "PyPI",
                    format!("paquete-{indice}"),
                    Some("1.0.0".to_string()),
                )
            })
            .collect();

        let avisos = cliente
            .query_batch(&consultas)
            .await
            .expect("la consulta debería completarse");

        assert_eq!(avisos.len(), 1500, "una respuesta por consulta");
        assert_eq!(
            peticiones_de_lote(&server).await,
            2,
            "1500 consultas caben en dos lotes de 1000"
        );
    }

    // --- 5.2 Mapeo de ecosistemas ---

    /// Escenario «Traducción de un ecosistema conocido»: la tabla es explícita y
    /// cerrada.
    #[test]
    fn el_mapeo_de_ecosistemas_es_cerrado() {
        for (github, osv) in [
            ("pip", "PyPI"),
            ("npm", "npm"),
            ("cargo", "crates.io"),
            ("go", "Go"),
            ("composer", "Packagist"),
            ("maven", "Maven"),
            ("nuget", "NuGet"),
            ("bundler", "RubyGems"),
            ("github-actions", "GitHub Actions"),
        ] {
            assert_eq!(map_ecosystem(github), Some(osv), "mapeo de `{github}`");
        }
    }

    /// Un ecosistema sin equivalencia declarada no se mapea.
    #[test]
    fn un_ecosistema_sin_equivalencia_no_se_mapea() {
        assert_eq!(map_ecosystem("swift"), None);
        assert_eq!(map_ecosystem(""), None);
        assert_eq!(map_ecosystem("otro"), None);
    }

    /// Escenario «Ecosistema sin equivalencia declarada»: la alerta se omite sin
    /// error y **no** se consulta OSV.
    #[tokio::test]
    async fn un_ecosistema_sin_equivalencia_se_omite_sin_error() {
        let server = MockServer::start().await;
        let cliente = osv(&server);

        let mut alerta = alerta_dependabot("1");
        alerta.ecosystem = Some("swift".to_string());

        let resultado = cliente
            .enrich([&mut alerta])
            .await
            .expect("la omisión de un ecosistema sin equivalencia no es un error");

        assert_eq!(
            resultado,
            EnrichmentOutcome {
                queried: 0,
                enriched: 0,
                skipped: 1,
            }
        );
        assert_eq!(
            peticiones_de_lote(&server).await,
            0,
            "no se consulta OSV si no hay ninguna consulta"
        );
        assert!(alerta.cve_id.is_none(), "la alerta queda sin enriquecer");
    }

    // --- 5.3 Completar sin sobreescribir ---

    /// Escenario «Versión corregida ya informada por GitHub»: el `first_patched`
    /// de GitHub se conserva y el CVSS informado no se sobreescribe; solo se
    /// completa el alias CVE ausente.
    #[tokio::test]
    async fn el_enriquecimiento_no_sobreescribe_lo_que_aporta_github() {
        let server = MockServer::start().await;
        montar_osv(&server, aviso_osv()).await;
        let cliente = osv(&server);

        let mut alerta = alerta_dependabot("1");
        alerta.cvss = Some(7.5);
        alerta.first_patched = Some("3.0.1".to_string());

        let resultado = cliente
            .enrich([&mut alerta])
            .await
            .expect("el enriquecimiento debería completarse");

        assert_eq!(resultado.enriched, 1);
        assert_eq!(
            alerta.cvss,
            Some(7.5),
            "el CVSS de GitHub no se sobreescribe"
        );
        assert_eq!(
            alerta.first_patched.as_deref(),
            Some("3.0.1"),
            "la versión corregida de GitHub se conserva"
        );
        assert_eq!(
            alerta.cve_id.as_deref(),
            Some("CVE-2021-1234"),
            "el alias CVE ausente se completa con el de OSV"
        );
    }

    /// Escenario «CVSS ausente completado por OSV»: un `cvss` a cero cuenta como
    /// ausente y se completa con el de OSV (que se calcula a partir del vector
    /// CVSS v3 del aviso).
    #[tokio::test]
    async fn el_cvss_cero_se_completa_con_el_de_osv() {
        let server = MockServer::start().await;
        montar_osv(&server, aviso_osv()).await;
        let cliente = osv(&server);

        let mut alerta = alerta_dependabot("1");
        alerta.cvss = Some(0.0);

        cliente
            .enrich([&mut alerta])
            .await
            .expect("el enriquecimiento debería completarse");

        assert_eq!(alerta.cvss, Some(9.8), "el CVSS a cero se completa con OSV");
        assert_eq!(
            alerta.first_patched.as_deref(),
            Some("3.0.2"),
            "la versión corregida ausente se completa con OSV"
        );
    }

    // --- 5.4 Fallo de OSV ---

    /// Monta un inventario de un repositorio y sus alertas de las tres fuentes.
    async fn montar_escaneo(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/user/repos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "full_name": "atareao/uno",
                "private": false,
                "archived": false,
                "fork": false,
                "language": "Python",
                "pushed_at": "2026-11-03T09:00:00Z"
            }])))
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex("^/repos/atareao/uno/dependabot/alerts$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "number": 1,
                "state": "open",
                "created_at": "2026-10-01T00:00:00Z",
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
            }])))
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex("^/repos/atareao/uno/secret-scanning/alerts$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex("^/repos/atareao/uno/code-scanning/alerts$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(server)
            .await;
    }

    /// Sumidero en memoria para capturar lo que se registra.
    #[derive(Clone, Default)]
    struct Captura(Arc<Mutex<Vec<u8>>>);

    impl Captura {
        /// Texto registrado hasta el momento.
        fn texto(&self) -> String {
            String::from_utf8(self.0.lock().expect("mutex no envenenado").clone())
                .expect("la salida registrada es UTF-8")
        }
    }

    impl Write for Captura {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("mutex no envenenado")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captura {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Suscriptor que escribe en `captura` los eventos de nivel `WARN` o superior.
    fn suscriptor(captura: Captura) -> impl tracing::Subscriber + Send + Sync {
        tracing_subscriber::fmt()
            .with_writer(captura)
            .with_ansi(false)
            .without_time()
            .with_max_level(Level::WARN)
            .finish()
    }

    /// Suscriptor que escribe en `captura` también los eventos de nivel `DEBUG`.
    fn suscriptor_debug(captura: Captura) -> impl tracing::Subscriber + Send + Sync {
        tracing_subscriber::fmt()
            .with_writer(captura)
            .with_ansi(false)
            .without_time()
            .with_max_level(Level::DEBUG)
            .finish()
    }

    /// Escenario «OSV no disponible»: un error de OSV no aborta el escaneo, las
    /// alertas se reportan sin enriquecer y se registra un aviso.
    #[tokio::test]
    async fn el_fallo_de_osv_no_aborta_el_escaneo_y_avisa() {
        let server = MockServer::start().await;
        montar_escaneo(&server).await;
        Mock::given(method("POST"))
            .and(path(RUTA_LOTE))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let captura = Captura::default();
        let _guarda = tracing::subscriber::set_default(suscriptor(captura.clone()));

        let resumen = ghwatch::scan::scan_with_enrichment(
            Arc::new(github(&server)),
            RepoSelection::default(),
            Arc::new(HiddenReporter),
            Arc::new(osv(&server)),
        )
        .await
        .expect("un fallo de OSV no debe abortar el escaneo");

        assert_eq!(resumen.succeeded_count(), 1, "el escaneo concluye bien");

        let alerta = resumen.alerts().next().expect("debe haber una alerta");
        assert!(alerta.cvss.is_none(), "la alerta se reporta sin enriquecer");
        assert!(
            alerta.cve_id.is_none(),
            "la alerta se reporta sin enriquecer"
        );
        assert_eq!(
            alerta.created_at,
            Some(
                "2026-10-01T00:00:00Z"
                    .parse::<DateTime<Utc>>()
                    .expect("instante válido")
            ),
            "la fecha de creación se puebla desde GitHub"
        );

        let log = captura.texto();
        assert!(log.contains("OSV"), "se registra un aviso de OSV: {log}");
    }

    /// Escenario «OSV no disponible» (variante de tiempo agotado): si OSV
    /// expira, el escaneo concluye bien y las alertas quedan sin enriquecer.
    #[tokio::test]
    async fn el_tiempo_de_espera_agotado_de_osv_no_aborta_el_escaneo() {
        let server = MockServer::start().await;
        montar_escaneo(&server).await;
        Mock::given(method("POST"))
            .and(path(RUTA_LOTE))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(400))
                    .set_body_json(json!({ "results": [] })),
            )
            .mount(&server)
            .await;

        let resumen = ghwatch::scan::scan_with_enrichment(
            Arc::new(github(&server)),
            RepoSelection::default(),
            Arc::new(HiddenReporter),
            Arc::new(osv_con_timeout(&server, Duration::from_millis(50))),
        )
        .await
        .expect("un tiempo agotado en OSV no debe abortar el escaneo");

        assert_eq!(resumen.succeeded_count(), 1, "el escaneo concluye bien");
        let alerta = resumen.alerts().next().expect("debe haber una alerta");
        assert!(alerta.cvss.is_none(), "la alerta se reporta sin enriquecer");
    }

    // --- 5.8 Endurecimiento tras la auditoría de la fase 5 ---

    /// Matcher que exige que la primera consulta **no** declare `version`.
    struct SinVersion;

    impl Match for SinVersion {
        fn matches(&self, peticion: &Request) -> bool {
            let Ok(cuerpo) = serde_json::from_slice::<Value>(&peticion.body) else {
                return false;
            };
            cuerpo
                .get("queries")
                .and_then(Value::as_array)
                .and_then(|queries| queries.first())
                .is_some_and(|query| query.get("version").is_none())
        }
    }

    /// I3 — El contrato de la petición a OSV está verificado: el cuerpo lleva el
    /// paquete con su ecosistema, omite `version` cuando es `None` y **ninguna**
    /// petición viaja con cabecera `Authorization`.
    #[tokio::test]
    async fn la_peticion_a_osv_lleva_el_cuerpo_esperado_y_sin_credenciales() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(RUTA_LOTE))
            .and(body_partial_json(json!({
                "queries": [{ "package": { "name": "django", "ecosystem": "PyPI" } }]
            })))
            .and(SinVersion)
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
            .mount(&server)
            .await;

        let cliente = osv(&server);
        let mut alerta = alerta_dependabot("1");

        cliente
            .enrich([&mut alerta])
            .await
            .expect("la consulta debería completarse");

        let peticiones = server.received_requests().await.unwrap_or_default();
        let de_lote: Vec<&Request> = peticiones
            .iter()
            .filter(|peticion| peticion.url.path() == RUTA_LOTE)
            .collect();
        assert_eq!(de_lote.len(), 1, "se envía una petición de lote");
        assert!(
            de_lote
                .iter()
                .all(|peticion| !peticion.headers.contains_key("authorization")),
            "OSV es público: ninguna petición debe llevar credenciales"
        );
    }

    /// I2 — Con dos vulnerabilidades devueltas para el mismo paquete, se elige la
    /// que comparte el `ghsa_id` de la alerta, en lugar de mezclar datos de la
    /// primera.
    #[tokio::test]
    async fn la_correlacion_por_ghsa_elige_la_vulnerabilidad_correcta() {
        let server = MockServer::start().await;
        montar_osv(
            &server,
            json!([
                {
                    "id": "GHSA-otra-0000-0000",
                    "aliases": ["CVE-2020-0001"],
                    "severity": [{ "type": "CVSS_V3", "score": "9.8" }],
                    "affected": [{
                        "package": { "name": "django", "ecosystem": "PyPI" },
                        "ranges": [{ "type": "ECOSYSTEM", "events": [{ "fixed": "1.0.0" }] }]
                    }]
                },
                {
                    "id": "GHSA-aaaa-bbbb-cccc",
                    "aliases": ["CVE-2020-0002"],
                    "severity": [{ "type": "CVSS_V3", "score": "5.0" }],
                    "affected": [{
                        "package": { "name": "django", "ecosystem": "PyPI" },
                        "ranges": [{ "type": "ECOSYSTEM", "events": [{ "fixed": "2.0.0" }] }]
                    }]
                }
            ]),
        )
        .await;
        let cliente = osv(&server);

        let mut alerta = alerta_dependabot("1");
        alerta.ghsa_id = Some("GHSA-aaaa-bbbb-cccc".to_string());

        cliente
            .enrich([&mut alerta])
            .await
            .expect("el enriquecimiento debería completarse");

        assert_eq!(
            alerta.cve_id.as_deref(),
            Some("CVE-2020-0002"),
            "se toma el CVE de la vulnerabilidad que coincide"
        );
        assert_eq!(
            alerta.cvss,
            Some(5.0),
            "el CVSS es el de la vulnerabilidad que coincide"
        );
        assert_eq!(
            alerta.first_patched.as_deref(),
            Some("2.0.0"),
            "la versión corregida es la de la vulnerabilidad que coincide"
        );
    }

    /// M4 — La omisión de una alerta por ecosistema sin equivalencia se registra
    /// en modo `DEBUG`.
    #[tokio::test]
    async fn la_omision_por_ecosistema_se_registra_en_debug() {
        let server = MockServer::start().await;
        let cliente = osv(&server);

        let captura = Captura::default();
        let _guarda = tracing::subscriber::set_default(suscriptor_debug(captura.clone()));

        let mut alerta = alerta_dependabot("1");
        alerta.ecosystem = Some("swift".to_string());

        cliente
            .enrich([&mut alerta])
            .await
            .expect("la omisión no es un error");

        let log = captura.texto();
        assert!(
            log.contains("sin equivalencia OSV"),
            "se registra la omisión a nivel DEBUG: {log}"
        );
        assert!(
            log.contains("swift"),
            "el evento identifica el ecosistema omitido: {log}"
        );
    }

    /// M7 — Un aviso que solo aporta un CVSS nulo no cuenta como enriquecido:
    /// la fusión no aporta información.
    #[tokio::test]
    async fn un_aviso_sin_informacion_no_cuenta_como_enriquecido() {
        let server = MockServer::start().await;
        montar_osv(
            &server,
            json!([{
                "id": "GHSA-aaaa-bbbb-cccc",
                "severity": [{ "type": "CVSS_V3", "score": "0.0" }]
            }]),
        )
        .await;
        let cliente = osv(&server);

        let mut alerta = alerta_dependabot("1");
        let resultado = cliente
            .enrich([&mut alerta])
            .await
            .expect("el enriquecimiento debería completarse");

        assert_eq!(
            resultado.enriched, 0,
            "un CVSS nulo no aporta información: no cuenta como enriquecido"
        );
        assert!(
            alerta.cvss.is_none(),
            "un CVSS nulo no debe poblar el campo"
        );
    }
}
