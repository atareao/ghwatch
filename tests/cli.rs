//! Tests de integración del CLI.
//!
//! Cubren la superficie de `cli` y `config`: los seis subcomandos, los códigos
//! de salida, el flujo completo de `scan` (GitHub + OSV + Valet simulados con
//! `wiremock`), la idempotencia, los subcomandos de consulta sin credenciales, la
//! inicialización y la no filtración de tokens.
//!
//! Todo el IO se inyecta: entorno, reloj, salida y resolución de credenciales.
//! La suite no hace red real ni depende de `$HOME`. Los tests viven en el módulo
//! `cli` para que `cargo test cli` los seleccione por nombre.

mod cli {
    use std::collections::HashMap;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    use chrono::{DateTime, TimeZone, Utc};
    use clap::Parser;
    use clap::error::ErrorKind;
    use ghwatch::auth::{AuthError, Token};
    use ghwatch::cli::{Cli, EXIT_AUTH, EXIT_GITHUB, EXIT_OK, EXIT_STORE, Env, run_with};
    use ghwatch::report::ReportDocument;
    use ghwatch::scan::ProgressReporter;
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Token de GitHub de los tests (falso).
    const GITHUB_TOKEN: &str = "ghp_token_de_prueba";
    /// Token de ingesta de Valet de los tests (falso).
    const VALET_TOKEN: &str = "val_token_de_prueba";
    /// Instante determinista usado por todos los tests.
    const AHORA: i64 = 1_700_000_000;

    /// Instante determinista.
    fn instante(segundos: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(segundos, 0)
            .single()
            .expect("instante válido")
    }

    /// Nombre del informe para el instante fijado.
    fn fecha(now: DateTime<Utc>) -> String {
        now.format("%Y-%m-%d").to_string()
    }

    /// Escritor compartido que captura la salida del CLI.
    #[derive(Clone, Default)]
    struct Salida(Arc<Mutex<Vec<u8>>>);

    impl Salida {
        fn texto(&self) -> String {
            String::from_utf8(self.0.lock().expect("cerrojo de salida").clone())
                .expect("la salida es UTF-8")
        }
    }

    impl Write for Salida {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("cerrojo de salida")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Constructor de [`Env`] para los tests.
    struct EnvPrueba {
        vars: HashMap<String, String>,
        now: DateTime<Utc>,
        out: Salida,
        reporter: Option<Arc<dyn ProgressReporter>>,
        auth_falla: bool,
    }

    impl EnvPrueba {
        fn nuevo() -> Self {
            Self {
                vars: HashMap::new(),
                now: instante(AHORA),
                out: Salida::default(),
                reporter: None,
                auth_falla: false,
            }
        }

        fn var(mut self, clave: &str, valor: &str) -> Self {
            self.vars.insert(clave.to_string(), valor.to_string());
            self
        }

        fn reporter(mut self, reporter: Arc<dyn ProgressReporter>) -> Self {
            self.reporter = Some(reporter);
            self
        }

        fn auth_falla(mut self) -> Self {
            self.auth_falla = true;
            self
        }

        fn salida(&self) -> Salida {
            self.out.clone()
        }

        fn construir(self) -> Env {
            let mapa = self.vars;
            let falla = self.auth_falla;
            Env {
                get_env: Box::new(move |clave| mapa.get(clave).cloned()),
                now: self.now,
                out: Box::new(self.out),
                reporter: self.reporter,
                resolver: Box::new(move || {
                    let falla = falla;
                    Box::pin(async move {
                        if falla {
                            Err(AuthError::NoCredentials {
                                cause: Box::new(AuthError::GhNotInstalled),
                            })
                        } else {
                            Token::new(GITHUB_TOKEN)
                        }
                    })
                }),
            }
        }
    }

    /// Construye un `Cli` a partir de los argumentos (sin el nombre del
    /// binario).
    fn cli_de(args: &[&str]) -> Cli {
        let mut argv: Vec<&str> = vec!["ghwatch"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).expect("argumentos válidos")
    }

    /// JSON de un repositorio del inventario.
    fn repo_json(nombre: &str, fork: bool, archived: bool) -> Value {
        json!({
            "full_name": nombre,
            "private": false,
            "archived": archived,
            "fork": fork,
            "language": "Rust",
            "pushed_at": "2026-11-03T09:00:00Z",
        })
    }

    /// JSON de una alerta de Dependabot.
    fn alerta_dependabot(numero: u64) -> Value {
        json!({
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

    /// JSON de una alerta de secret scanning.
    fn alerta_secreto(numero: u64) -> Value {
        json!({
            "number": numero,
            "secret_type": "adafruit_io_key",
            "secret_type_display_name": "Adafruit IO Key"
        })
    }

    /// JSON de una alerta de code scanning.
    fn alerta_code(numero: u64) -> Value {
        json!({
            "number": numero,
            "rule": {
                "description": "Uso de función insegura",
                "security_severity_level": "high"
            }
        })
    }

    /// Monta el inventario en una única página.
    async fn montar_inventario(server: &MockServer, repos: &[Value]) {
        Mock::given(method("GET"))
            .and(path("/user/repos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(repos))
            .mount(server)
            .await;
    }

    /// Monta las respuestas de las tres fuentes para todos los repositorios.
    async fn montar_fuentes(
        server: &MockServer,
        dependabot: &[Value],
        secret: &[Value],
        code: &[Value],
    ) {
        Mock::given(method("GET"))
            .and(path_regex(r"^/repos/[^/]+/[^/]+/dependabot/alerts$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(dependabot))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/repos/[^/]+/[^/]+/secret-scanning/alerts$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(secret))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/repos/[^/]+/[^/]+/code-scanning/alerts$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(code))
            .mount(server)
            .await;
    }

    /// Monta una respuesta de lote vacía de OSV.
    async fn montar_osv(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/v1/querybatch"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
            .mount(server)
            .await;
    }

    /// Monta un Valet que acepta la notificación.
    async fn montar_valet(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "n-1" })))
            .mount(server)
            .await;
    }

    /// Cuenta las peticiones recibidas en una ruta.
    async fn peticiones_a(server: &MockServer, ruta: &str) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|peticion| peticion.url.path() == ruta)
            .count()
    }

    /// Lee y deserializa el informe JSON.
    fn leer_informe(json: &Path) -> ReportDocument {
        let contenido = std::fs::read_to_string(json).expect("leer informe JSON");
        serde_json::from_str(&contenido).expect("deserializar informe")
    }

    /// JSON mínimo y válido de un informe, para los tests de `report`.
    fn informe_json(run_id: i64) -> Value {
        json!({
            "schema_version": 1,
            "generated_at": "2026-11-04T00:00:00Z",
            "run_id": run_id,
            "scanned": 1,
            "failed": 0,
            "total": 1,
            "by_severity": {
                "critical": 0,
                "high": 1,
                "medium": 0,
                "low": 0,
                "unknown": 0
            },
            "by_ecosystem": [],
            "worst_repos": [],
            "new": 1,
            "resolved": 0,
            "actions": [],
            "history": []
        })
    }

    /// Escribe un `config.toml` con el modo indicado y devuelve su ruta.
    #[cfg(unix)]
    fn escribir_config(directorio: &Path, nombre: &str, modo: u32, contenido: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let ruta = directorio.join(nombre);
        std::fs::write(&ruta, contenido).expect("escribir config.toml");
        std::fs::set_permissions(&ruta, std::fs::Permissions::from_mode(modo))
            .expect("fijar permisos");
        ruta
    }

    /// Sumidero en memoria para capturar lo que se registra con `tracing`.
    #[derive(Clone, Default)]
    struct Captura(Arc<Mutex<Vec<u8>>>);

    impl Captura {
        fn texto(&self) -> String {
            String::from_utf8(self.0.lock().expect("cerrojo de registro").clone())
                .expect("el registro es UTF-8")
        }
    }

    impl Write for Captura {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("cerrojo de registro")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captura {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Suscriptor que registra hasta `TRACE` en el sumidero dado.
    fn suscriptor(captura: Captura) -> impl tracing::Subscriber + Send + Sync {
        tracing_subscriber::fmt()
            .with_writer(captura)
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::TRACE)
            .finish()
    }

    /// Instala una única vez un suscriptor **global** que captura los eventos.
    ///
    /// Un suscriptor global —no `set_default`, que es thread-local— garantiza
    /// que los callsites compartidos con el resto de tests del binario no
    /// queden cacheados como inertes en otro hilo: el interés se evalúa contra
    /// el suscriptor global. `rebuild_interest_cache` re-evalúa todos los
    /// callsites ya registrados, incluidos los que corrieron antes.
    fn logs_globales() -> &'static Captura {
        static LOGS: OnceLock<Captura> = OnceLock::new();
        LOGS.get_or_init(|| {
            let captura = Captura::default();
            let _ = tracing::subscriber::set_global_default(suscriptor(captura.clone()));
            tracing::callsite::rebuild_interest_cache();
            captura
        })
    }

    /// Reporter de progreso que solo cuenta las llamadas.
    #[derive(Default)]
    struct ReporterContado {
        arranques: AtomicUsize,
        avances: AtomicUsize,
        cierres: AtomicUsize,
    }

    impl ProgressReporter for ReporterContado {
        fn start(&self, _total: u64) {
            self.arranques.fetch_add(1, Ordering::SeqCst);
        }

        fn advance(&self, _delta: u64) {
            self.avances.fetch_add(1, Ordering::SeqCst);
        }

        fn finish(&self) {
            self.cierres.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Ejecuta un `scan` completo con las URLs indicadas y devuelve su código.
    async fn escanear(
        db: &Path,
        reports: &Path,
        github: &MockServer,
        valet: &MockServer,
        extra: &[&str],
    ) -> u8 {
        let db = db.to_str().expect("ruta UTF-8");
        let reports = reports.to_str().expect("ruta UTF-8");
        let github_uri = github.uri();
        let valet_uri = valet.uri();

        let mut args = vec![
            "scan",
            "--db",
            db,
            "--report-dir",
            reports,
            "--github-base-url",
            github_uri.as_str(),
            "--valet-base-url",
            valet_uri.as_str(),
            "--no-enrich",
        ];
        args.extend_from_slice(extra);

        let env = EnvPrueba::nuevo()
            .var("GITHUB_TOKEN", GITHUB_TOKEN)
            .var("VALET_INGEST_TOKEN", VALET_TOKEN)
            .construir();
        run_with(cli_de(&args), env).await
    }

    // --- 7.1 Superficie del CLI ---

    /// La ayuda nombra los seis subcomandos y termina con éxito.
    #[test]
    fn la_ayuda_nombra_los_seis_subcomandos() {
        let error = Cli::try_parse_from(["ghwatch", "--help"]).expect_err("--help no es válido");
        assert_eq!(error.kind(), ErrorKind::DisplayHelp);
        assert_eq!(error.exit_code(), 0);

        let texto = error.to_string();
        for subcomando in ["scan", "report", "diff", "history", "notify", "init"] {
            assert!(
                texto.contains(subcomando),
                "la ayuda debe nombrar `{subcomando}`:\n{texto}"
            );
        }
    }

    /// Un subcomando desconocido es un error de uso con código 2.
    #[test]
    fn un_subcomando_desconocido_es_error_de_uso() {
        let error = Cli::try_parse_from(["ghwatch", "volar"]).expect_err("`volar` no existe");
        assert_eq!(error.exit_code(), 2);
    }

    // --- 7.4 `scan` de extremo a extremo ---

    /// Un escaneo con GitHub, OSV y Valet simulados escribe el informe, notifica
    /// y termina con código 0.
    #[tokio::test]
    async fn el_escaneo_extremo_a_extremo_escribe_informe_y_notifica() {
        let github = MockServer::start().await;
        let osv = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(
            &github,
            &[alerta_dependabot(1)],
            &[alerta_secreto(7)],
            &[alerta_code(3)],
        )
        .await;
        montar_osv(&osv).await;
        montar_valet(&valet).await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        let env = EnvPrueba::nuevo()
            .var("GITHUB_TOKEN", GITHUB_TOKEN)
            .var("VALET_INGEST_TOKEN", VALET_TOKEN)
            .construir();

        let osv_uri = osv.uri();
        let codigo = run_with(
            cli_de(&[
                "scan",
                "--db",
                db.to_str().expect("ruta"),
                "--report-dir",
                reports.to_str().expect("ruta"),
                "--github-base-url",
                github.uri().as_str(),
                "--osv-base-url",
                osv_uri.as_str(),
                "--valet-base-url",
                valet.uri().as_str(),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);

        let nombre = fecha(instante(AHORA));
        let markdown = reports.join(format!("ghwatch-{nombre}.md"));
        let json = reports.join(format!("ghwatch-{nombre}.json"));
        assert!(markdown.exists(), "el informe Markdown debe existir");
        assert!(json.exists(), "el informe JSON debe existir");

        let documento = leer_informe(&json);
        assert_eq!(documento.scanned, 1);
        assert_eq!(documento.total, 3, "una alerta por fuente");
        assert!(documento.new > 0, "el primer escaneo aporta novedades");

        assert_eq!(
            peticiones_a(&valet, "/api/notifications").await,
            1,
            "Valet debe recibir una notificación"
        );
    }

    // --- Valet caído ---

    /// Un Valet que falla no impide escribir el informe ni cambia el éxito.
    #[tokio::test]
    async fn valet_caido_no_impide_el_informe_ni_cambia_el_exito() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[alerta_dependabot(1)], &[], &[]).await;
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&valet)
            .await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        let codigo = escanear(&db, &reports, &github, &valet, &[]).await;

        assert_eq!(codigo, EXIT_OK, "Valet caído no altera el código de salida");
        let nombre = fecha(instante(AHORA));
        assert!(reports.join(format!("ghwatch-{nombre}.md")).exists());
        assert!(reports.join(format!("ghwatch-{nombre}.json")).exists());
    }

    // --- 7.5 Idempotencia ---

    /// Repetir un escaneo idéntico informa de 0 nuevas y 0 resueltas.
    #[tokio::test]
    async fn el_segundo_escaneo_identico_no_aporta_novedades() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[alerta_dependabot(1)], &[], &[]).await;
        montar_valet(&valet).await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");

        assert_eq!(escanear(&db, &reports, &github, &valet, &[]).await, EXIT_OK);
        let primero =
            leer_informe(&reports.join(format!("ghwatch-{}.json", fecha(instante(AHORA)))));
        assert_eq!(primero.new, 1);

        assert_eq!(escanear(&db, &reports, &github, &valet, &[]).await, EXIT_OK);
        let segundo =
            leer_informe(&reports.join(format!("ghwatch-{}.json", fecha(instante(AHORA)))));
        assert_eq!(segundo.new, 0, "nada nuevo");
        assert_eq!(segundo.resolved, 0, "nada resuelto");
    }

    /// `--include-forks` y `--include-archived` reincorporan al inventario los
    /// repositorios que el filtro por defecto excluye.
    #[tokio::test]
    async fn include_forks_e_include_archived_reincorporan_repositorios() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(
            &github,
            &[
                repo_json("atareao/normal", false, false),
                repo_json("atareao/fork", true, false),
                repo_json("atareao/archivado", false, true),
            ],
        )
        .await;
        montar_fuentes(&github, &[], &[], &[]).await;
        montar_valet(&valet).await;

        let nombre = fecha(instante(AHORA));

        let db1 = dir.path().join("por-defecto.db");
        let reps1 = dir.path().join("reports-1");
        assert_eq!(escanear(&db1, &reps1, &github, &valet, &[]).await, EXIT_OK);
        let por_defecto = leer_informe(&reps1.join(format!("ghwatch-{nombre}.json")));
        assert_eq!(por_defecto.scanned, 1, "solo el repositorio normal");

        let db2 = dir.path().join("con-todos.db");
        let reps2 = dir.path().join("reports-2");
        assert_eq!(
            escanear(
                &db2,
                &reps2,
                &github,
                &valet,
                &["--include-forks", "--include-archived"]
            )
            .await,
            EXIT_OK
        );
        let con_todos = leer_informe(&reps2.join(format!("ghwatch-{nombre}.json")));
        assert_eq!(con_todos.scanned, 3, "forks y archivados incluidos");
    }

    // --- 7.3 Códigos de salida ---

    /// Sin token de GitHub el escaneo termina con 3 y no hace ninguna petición.
    #[tokio::test]
    async fn sin_token_devuelve_tres_y_no_hace_peticiones() {
        let github = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");
        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        let env = EnvPrueba::nuevo().auth_falla().construir();

        let codigo = run_with(
            cli_de(&[
                "scan",
                "--db",
                db.to_str().expect("ruta"),
                "--report-dir",
                reports.to_str().expect("ruta"),
                "--github-base-url",
                github.uri().as_str(),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_AUTH);
        assert_eq!(
            github.received_requests().await.unwrap_or_default().len(),
            0,
            "sin token no se contacta con GitHub"
        );
    }

    /// Un fallo fatal en el descubrimiento de GitHub termina con 4.
    #[tokio::test]
    async fn un_fallo_fatal_de_github_devuelve_cuatro() {
        let github = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");
        Mock::given(method("GET"))
            .and(path("/user/repos"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&github)
            .await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        let env = EnvPrueba::nuevo()
            .var("GITHUB_TOKEN", GITHUB_TOKEN)
            .construir();

        let codigo = run_with(
            cli_de(&[
                "scan",
                "--db",
                db.to_str().expect("ruta"),
                "--report-dir",
                reports.to_str().expect("ruta"),
                "--github-base-url",
                github.uri().as_str(),
                "--no-enrich",
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_GITHUB);
    }

    /// Un fallo fatal de la base de datos termina con 5.
    #[tokio::test]
    async fn un_fallo_de_la_base_de_datos_devuelve_cinco() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[], &[], &[]).await;
        montar_valet(&valet).await;

        // La ruta de la base de datos es un directorio: sqlite no puede abrirla.
        let db = dir.path().to_path_buf();
        let reports = dir.path().join("informes");
        let codigo = escanear(&db, &reports, &github, &valet, &[]).await;

        assert_eq!(codigo, EXIT_STORE);
    }

    // --- 7.6 Subcomandos de consulta sin credenciales ---

    /// `report`, `diff`, `history` y `notify` funcionan sin token de GitHub y sin
    /// contactar con la red.
    #[tokio::test]
    async fn las_consultas_funcionan_sin_credenciales() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[alerta_dependabot(1)], &[], &[]).await;
        montar_valet(&valet).await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        assert_eq!(escanear(&db, &reports, &github, &valet, &[]).await, EXIT_OK);

        let peticiones_github = github.received_requests().await.unwrap_or_default().len();
        let db = db.to_str().expect("ruta");
        let reports = reports.to_str().expect("ruta");

        for args in [
            vec!["report", "--report-dir", reports],
            vec!["diff", "--db", db],
            vec!["history", "--db", db],
        ] {
            let env = EnvPrueba::nuevo().auth_falla().construir();
            let codigo = run_with(cli_de(&args), env).await;
            assert_eq!(codigo, EXIT_OK, "consulta sin credenciales: {args:?}");
        }
        assert_eq!(
            github.received_requests().await.unwrap_or_default().len(),
            peticiones_github,
            "las consultas no contactan con GitHub"
        );

        // `notify --force` con token de ingesta envía sin GitHub.
        let valet_uri = valet.uri();
        let antes = peticiones_a(&valet, "/api/notifications").await;
        let env = EnvPrueba::nuevo()
            .auth_falla()
            .var("VALET_INGEST_TOKEN", VALET_TOKEN)
            .construir();
        let codigo = run_with(
            cli_de(&[
                "notify",
                "--report-dir",
                reports,
                "--force",
                "--valet-base-url",
                valet_uri.as_str(),
            ]),
            env,
        )
        .await;
        assert_eq!(codigo, EXIT_OK);
        assert_eq!(
            peticiones_a(&valet, "/api/notifications").await,
            antes + 1,
            "notify envía una notificación adicional"
        );
    }

    /// `notify --force` envía el último informe sin volver a escanear.
    #[tokio::test]
    async fn notify_force_envia_sin_github() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[alerta_dependabot(1)], &[], &[]).await;
        montar_valet(&valet).await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        assert_eq!(escanear(&db, &reports, &github, &valet, &[]).await, EXIT_OK);
        let antes = peticiones_a(&valet, "/api/notifications").await;
        let peticiones_github = github.received_requests().await.unwrap_or_default().len();

        let valet_uri = valet.uri();
        let env = EnvPrueba::nuevo()
            .auth_falla()
            .var("VALET_INGEST_TOKEN", VALET_TOKEN)
            .construir();
        let codigo = run_with(
            cli_de(&[
                "notify",
                "--report-dir",
                reports.to_str().expect("ruta"),
                "--force",
                "--valet-base-url",
                valet_uri.as_str(),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);
        assert_eq!(
            peticiones_a(&valet, "/api/notifications").await,
            antes + 1,
            "notify envía una notificación adicional"
        );
        assert_eq!(
            github.received_requests().await.unwrap_or_default().len(),
            peticiones_github,
            "notify no contacta con GitHub"
        );
    }

    // --- 7.6 `diff` histórico ---

    /// `diff` lista las alertas nuevas y las resueltas de la última ejecución.
    #[tokio::test]
    async fn el_diff_lista_nuevas_y_resueltas() {
        let github1 = MockServer::start().await;
        let github2 = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(&github1, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(
            &github1,
            &[alerta_dependabot(1), alerta_dependabot(2)],
            &[],
            &[],
        )
        .await;

        montar_inventario(&github2, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(
            &github2,
            &[alerta_dependabot(2), alerta_dependabot(3)],
            &[],
            &[],
        )
        .await;

        montar_valet(&valet).await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        assert_eq!(
            escanear(&db, &reports, &github1, &valet, &[]).await,
            EXIT_OK
        );
        assert_eq!(
            escanear(&db, &reports, &github2, &valet, &[]).await,
            EXIT_OK
        );

        let builder = EnvPrueba::nuevo().auth_falla();
        let salida = builder.salida();
        let env = builder.construir();
        let codigo = run_with(cli_de(&["diff", "--db", db.to_str().expect("ruta")]), env).await;

        assert_eq!(codigo, EXIT_OK);
        let texto = salida.texto();
        assert!(texto.contains("#3"), "la alerta nueva aparece: {texto}");
        assert!(texto.contains("#1"), "la alerta resuelta aparece: {texto}");
    }

    // --- 7.8 `init` ---

    /// `init` crea la base de datos y la plantilla de configuración con permisos
    /// `600`, sin sobrescribir un fichero existente.
    #[tokio::test]
    async fn init_crea_base_y_configuracion_sin_sobrescribir() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("crear HOME");

        let env = EnvPrueba::nuevo()
            .var("HOME", home.to_str().expect("ruta"))
            .construir();
        assert_eq!(run_with(cli_de(&["init"]), env).await, EXIT_OK);

        let db = home.join(".local/share/ghwatch/ghwatch.db");
        let config = home.join(".config/ghwatch/config.toml");
        assert!(db.exists(), "la base de datos debe existir");
        assert!(config.exists(), "la configuración debe existir");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let modo = std::fs::metadata(&config)
                .expect("metadatos")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(modo, 0o600, "la plantilla se escribe con permisos 600");
        }

        let contenido = std::fs::read_to_string(&config).expect("leer config");
        let env = EnvPrueba::nuevo()
            .var("HOME", home.to_str().expect("ruta"))
            .construir();
        assert_eq!(run_with(cli_de(&["init"]), env).await, EXIT_OK);
        assert_eq!(
            std::fs::read_to_string(&config).expect("releer config"),
            contenido,
            "un config.toml existente no se sobrescribe"
        );
    }

    // --- 7.6 `--quiet` y `-v` ---

    /// `--quiet` no dibuja barra de progreso y no altera el resultado; sin
    /// `--quiet`, el reporter inyectado sí se usa.
    #[tokio::test]
    async fn quiet_no_dibuja_barra_ni_cambia_el_resultado() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[alerta_dependabot(1)], &[], &[]).await;
        montar_valet(&valet).await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        let reporter = Arc::new(ReporterContado::default());
        let reporter_dyn: Arc<dyn ProgressReporter> = reporter.clone();

        let db_str = db.to_str().expect("ruta");
        let reports_str = reports.to_str().expect("ruta");
        let github_uri = github.uri();
        let valet_uri = valet.uri();
        let env = EnvPrueba::nuevo()
            .var("GITHUB_TOKEN", GITHUB_TOKEN)
            .var("VALET_INGEST_TOKEN", VALET_TOKEN)
            .reporter(reporter_dyn)
            .construir();

        let codigo = run_with(
            cli_de(&[
                "scan",
                "--quiet",
                "--no-enrich",
                "--db",
                db_str,
                "--report-dir",
                reports_str,
                "--github-base-url",
                github_uri.as_str(),
                "--valet-base-url",
                valet_uri.as_str(),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);
        assert_eq!(
            reporter.arranques.load(Ordering::SeqCst),
            0,
            "`--quiet` no dibuja barra"
        );

        // Sin `--quiet`, el reporter inyectado recibe el progreso.
        let db2 = dir.path().join("sin-quiet.db");
        let reps2 = dir.path().join("reports-sin-quiet");
        let reporter2 = Arc::new(ReporterContado::default());
        let reporter2_dyn: Arc<dyn ProgressReporter> = reporter2.clone();
        let env = EnvPrueba::nuevo()
            .var("GITHUB_TOKEN", GITHUB_TOKEN)
            .var("VALET_INGEST_TOKEN", VALET_TOKEN)
            .reporter(reporter2_dyn)
            .construir();

        let codigo = run_with(
            cli_de(&[
                "scan",
                "--no-enrich",
                "--db",
                db2.to_str().expect("ruta"),
                "--report-dir",
                reps2.to_str().expect("ruta"),
                "--github-base-url",
                github_uri.as_str(),
                "--valet-base-url",
                valet_uri.as_str(),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);
        assert_eq!(
            reporter2.arranques.load(Ordering::SeqCst),
            1,
            "sin `--quiet` se usa el reporter"
        );
    }

    // --- 7.10 Ubicaciones por defecto ---

    /// Sin `--db` ni `--report-dir`, la base y los informes cuelgan de
    /// `$HOME/.local/share/ghwatch/`.
    #[tokio::test]
    async fn las_ubicaciones_por_defecto_cuelgan_de_home() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("crear HOME");

        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[], &[], &[]).await;
        montar_valet(&valet).await;

        let github_uri = github.uri();
        let valet_uri = valet.uri();
        let env = EnvPrueba::nuevo()
            .var("HOME", home.to_str().expect("ruta"))
            .var("GITHUB_TOKEN", GITHUB_TOKEN)
            .var("VALET_INGEST_TOKEN", VALET_TOKEN)
            .construir();

        let codigo = run_with(
            cli_de(&[
                "scan",
                "--no-enrich",
                "--github-base-url",
                github_uri.as_str(),
                "--valet-base-url",
                valet_uri.as_str(),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);
        assert!(
            home.join(".local/share/ghwatch/ghwatch.db").exists(),
            "la base por defecto cuelga de $HOME"
        );
        let nombre = fecha(instante(AHORA));
        assert!(
            home.join(format!(
                ".local/share/ghwatch/reports/ghwatch-{nombre}.json"
            ))
            .exists(),
            "los informes por defecto cuelgan de $HOME"
        );
    }

    // --- Anti-fuga de tokens ---

    /// Ni el token de GitHub ni el de ingesta aparecen en la salida inyectada,
    /// en el registro de `tracing` ni en los informes escritos.
    ///
    /// El registro se captura con un suscriptor global: un evento `info!` que
    /// filtrase el token (como el que se usa para verificar el test) queda en
    /// esta captura, no solo en el proceso.
    #[tokio::test]
    async fn ningun_token_aparece_en_la_salida() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");

        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[alerta_dependabot(1)], &[], &[]).await;
        montar_valet(&valet).await;

        let secreto_github = "ghp_SECRETO_github_123";
        let secreto_valet = "val_SECRETO_ingesta_456";

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        let builder = EnvPrueba::nuevo()
            .var("GITHUB_TOKEN", secreto_github)
            .var("VALET_INGEST_TOKEN", secreto_valet);
        let salida = builder.salida();
        let env = builder.construir();
        let logs = logs_globales();

        let github_uri = github.uri();
        let valet_uri = valet.uri();
        let codigo = run_with(
            cli_de(&[
                "scan",
                "-vv",
                "--no-enrich",
                "--db",
                db.to_str().expect("ruta"),
                "--report-dir",
                reports.to_str().expect("ruta"),
                "--github-base-url",
                github_uri.as_str(),
                "--valet-base-url",
                valet_uri.as_str(),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);

        let nombre = fecha(instante(AHORA));
        let markdown =
            std::fs::read_to_string(reports.join(format!("ghwatch-{nombre}.md"))).expect("md");
        let json =
            std::fs::read_to_string(reports.join(format!("ghwatch-{nombre}.json"))).expect("json");

        for (donde, texto) in [
            ("salida", salida.texto()),
            ("registro", logs.texto()),
            ("informe Markdown", markdown),
            ("informe JSON", json),
        ] {
            assert!(
                !texto.contains(secreto_github),
                "el token de GitHub no puede aparecer en {donde}: {texto}"
            );
            assert!(
                !texto.contains(secreto_valet),
                "el token de ingesta no puede aparecer en {donde}: {texto}"
            );
        }
    }

    // --- Endurecimiento tras la auditoría ---

    /// B1: el aviso de un `config.toml` laxo va a `stderr` y no contamina el
    /// JSON que `report --format json` escribe en `stdout`.
    #[cfg(unix)]
    #[test]
    fn el_aviso_de_config_laxa_no_contamina_el_json_de_stdout() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("crear HOME");
        let reports = dir.path().join("informes");
        std::fs::create_dir_all(&reports).expect("crear informes");
        std::fs::write(
            reports.join("ghwatch-2020-01-01.json"),
            serde_json::to_vec(&informe_json(42)).expect("serializar informe"),
        )
        .expect("escribir informe");
        let config = escribir_config(dir.path(), "config.toml", 0o644, "concurrency = 3\n");

        let salida = Command::new(env!("CARGO_BIN_EXE_ghwatch"))
            .args([
                "report",
                "--format",
                "json",
                "--report-dir",
                reports.to_str().expect("ruta"),
                "--config",
                config.to_str().expect("ruta"),
            ])
            .env("HOME", &home)
            .env_remove("GITHUB_TOKEN")
            .env_remove("GHWATCH_DB")
            .output()
            .expect("ejecutar ghwatch");

        assert!(
            salida.status.success(),
            "report debe terminar con éxito: {}",
            String::from_utf8_lossy(&salida.stderr)
        );
        let stdout = String::from_utf8(salida.stdout).expect("stdout UTF-8");
        let valor: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
            panic!(
                "stdout debe ser solo JSON válido ({error}):\n{stdout}\nstderr: {}",
                String::from_utf8_lossy(&salida.stderr)
            )
        });
        assert_eq!(valor["run_id"], 42);
        let stderr = String::from_utf8_lossy(&salida.stderr);
        assert!(
            stderr.to_lowercase().contains("permisos"),
            "el aviso de permisos va a stderr: {stderr}"
        );
    }

    /// I2: `notify` no escribe informes y envía el documento del `.json` más
    /// reciente, aunque exista un `.md` de otra fecha.
    #[tokio::test]
    async fn notify_no_reescribe_el_markdown_y_envia_el_documento_del_json() {
        let valet = MockServer::start().await;
        montar_valet(&valet).await;
        let dir = tempfile::tempdir().expect("directorio temporal");
        let reports = dir.path().join("informes");
        std::fs::create_dir_all(&reports).expect("crear informes");

        std::fs::write(
            reports.join("ghwatch-2026-11-04.json"),
            serde_json::to_vec(&informe_json(9)).expect("serializar"),
        )
        .expect("escribir json");
        let centinela = "# centinela de otra fecha\n";
        std::fs::write(reports.join("ghwatch-2020-01-01.md"), centinela).expect("escribir md");

        let env = EnvPrueba::nuevo()
            .auth_falla()
            .var("VALET_INGEST_TOKEN", VALET_TOKEN)
            .construir();
        let valet_uri = valet.uri();
        let codigo = run_with(
            cli_de(&[
                "notify",
                "--report-dir",
                reports.to_str().expect("ruta"),
                "--force",
                "--valet-base-url",
                valet_uri.as_str(),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);
        assert_eq!(
            std::fs::read_to_string(reports.join("ghwatch-2020-01-01.md")).expect("leer md"),
            centinela,
            "notify no debe reescribir un Markdown existente"
        );
        assert!(
            !reports.join("ghwatch-2020-01-01.json").exists(),
            "notify no debe fabricar informes"
        );

        let peticiones = valet.received_requests().await.unwrap_or_default();
        assert_eq!(peticiones.len(), 1, "notify envía una notificación");
        let cuerpo = String::from_utf8_lossy(&peticiones[0].body);
        assert!(
            cuerpo.contains("ghwatch-2026-11-04.md"),
            "la notificación referencia el informe del JSON: {cuerpo}"
        );
    }

    /// M6: `--limit 0` no muestra ninguna ejecución.
    #[tokio::test]
    async fn history_con_limit_cero_no_muestra_ejecuciones() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");
        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[alerta_dependabot(1)], &[], &[]).await;
        montar_valet(&valet).await;

        let db = dir.path().join("ghwatch.db");
        let reports = dir.path().join("informes");
        assert_eq!(escanear(&db, &reports, &github, &valet, &[]).await, EXIT_OK);

        let builder = EnvPrueba::nuevo().auth_falla();
        let salida = builder.salida();
        let env = builder.construir();
        let codigo = run_with(
            cli_de(&[
                "history",
                "--db",
                db.to_str().expect("ruta"),
                "--limit",
                "0",
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);
        assert!(
            !salida.texto().contains('#'),
            "con --limit 0 no se lista ninguna ejecución: {:?}",
            salida.texto()
        );
    }

    /// M7: un informe con una `schema_version` desconocida es error de almacén.
    #[tokio::test]
    async fn un_informe_con_schema_version_desconocida_es_error_de_almacen() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let reports = dir.path().join("informes");
        std::fs::create_dir_all(&reports).expect("crear informes");
        let mut documento = informe_json(1);
        documento["schema_version"] = json!(999);
        std::fs::write(
            reports.join("ghwatch-2020-01-01.json"),
            serde_json::to_vec(&documento).expect("serializar"),
        )
        .expect("escribir informe");

        let env = EnvPrueba::nuevo().auth_falla().construir();
        let codigo = run_with(
            cli_de(&[
                "report",
                "--format",
                "json",
                "--report-dir",
                reports.to_str().expect("ruta"),
            ]),
            env,
        )
        .await;
        assert_eq!(codigo, EXIT_STORE);
    }

    /// M7/M13: un informe que no es JSON es un error de almacén.
    #[tokio::test]
    async fn un_informe_json_corrupto_es_error_de_almacen() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let reports = dir.path().join("informes");
        std::fs::create_dir_all(&reports).expect("crear informes");
        std::fs::write(reports.join("ghwatch-2020-01-01.json"), "{ esto no es json")
            .expect("escribir informe");

        let env = EnvPrueba::nuevo().auth_falla().construir();
        let codigo = run_with(
            cli_de(&[
                "report",
                "--format",
                "json",
                "--report-dir",
                reports.to_str().expect("ruta"),
            ]),
            env,
        )
        .await;
        assert_eq!(codigo, EXIT_STORE);
    }

    /// M13: sin informes, `report` termina con éxito y sin salida.
    #[tokio::test]
    async fn sin_informes_report_termina_con_exito_y_sin_salida() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let reports = dir.path().join("vacio");
        std::fs::create_dir_all(&reports).expect("crear informes");

        let builder = EnvPrueba::nuevo().auth_falla();
        let salida = builder.salida();
        let env = builder.construir();
        let codigo = run_with(
            cli_de(&[
                "report",
                "--format",
                "json",
                "--report-dir",
                reports.to_str().expect("ruta"),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);
        assert!(salida.texto().trim().is_empty(), "no hay nada que reemitir");
    }

    /// M9: `init` pasa por la resolución de configuración y respeta `GHWATCH_DB`.
    #[tokio::test]
    async fn init_respeta_ghwatch_db() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let db = dir.path().join("personalizada.db");
        let env = EnvPrueba::nuevo()
            .var("GHWATCH_DB", db.to_str().expect("ruta"))
            .construir();
        assert_eq!(run_with(cli_de(&["init"]), env).await, EXIT_OK);
        assert!(db.exists(), "init crea la base indicada por GHWATCH_DB");
    }

    /// M13: `--config` se usa de verdad (el fichero aporta `report_dir`).
    #[cfg(unix)]
    #[tokio::test]
    async fn el_fichero_de_configuracion_aporta_report_dir() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let reports = dir.path().join("informes");
        std::fs::create_dir_all(&reports).expect("crear informes");
        std::fs::write(
            reports.join("ghwatch-2020-01-01.json"),
            serde_json::to_vec(&informe_json(5)).expect("serializar"),
        )
        .expect("escribir informe");
        let config = escribir_config(
            dir.path(),
            "config.toml",
            0o600,
            &format!("report_dir = {:?}\n", reports.to_str().expect("ruta")),
        );

        let builder = EnvPrueba::nuevo().auth_falla();
        let salida = builder.salida();
        let env = builder.construir();
        let codigo = run_with(
            cli_de(&[
                "report",
                "--format",
                "json",
                "--config",
                config.to_str().expect("ruta"),
            ]),
            env,
        )
        .await;

        assert_eq!(codigo, EXIT_OK);
        let valor: Value =
            serde_json::from_str(salida.texto().trim()).expect("la salida es el JSON del informe");
        assert_eq!(valor["run_id"], 5);
    }

    /// M13: `--no-enrich` no consulta OSV; con enriquecimiento sí.
    #[tokio::test]
    async fn el_enriquecimiento_solo_consulta_osv_cuando_esta_activo() {
        let github = MockServer::start().await;
        let valet = MockServer::start().await;
        let osv = MockServer::start().await;
        let dir = tempfile::tempdir().expect("directorio temporal");
        montar_inventario(&github, &[repo_json("atareao/uno", false, false)]).await;
        montar_fuentes(&github, &[alerta_dependabot(1)], &[], &[]).await;
        montar_valet(&valet).await;
        montar_osv(&osv).await;

        let db = dir.path().join("sin.db");
        let reports = dir.path().join("sin");
        assert_eq!(escanear(&db, &reports, &github, &valet, &[]).await, EXIT_OK);
        assert_eq!(
            peticiones_a(&osv, "/v1/querybatch").await,
            0,
            "--no-enrich no consulta OSV"
        );

        let osv_uri = osv.uri();
        let db = dir.path().join("con.db");
        let reports = dir.path().join("con");
        let env = EnvPrueba::nuevo()
            .var("GITHUB_TOKEN", GITHUB_TOKEN)
            .var("VALET_INGEST_TOKEN", VALET_TOKEN)
            .construir();
        let codigo = run_with(
            cli_de(&[
                "scan",
                "--db",
                db.to_str().expect("ruta"),
                "--report-dir",
                reports.to_str().expect("ruta"),
                "--github-base-url",
                github.uri().as_str(),
                "--osv-base-url",
                osv_uri.as_str(),
                "--valet-base-url",
                valet.uri().as_str(),
            ]),
            env,
        )
        .await;
        assert_eq!(codigo, EXIT_OK);
        assert!(
            peticiones_a(&osv, "/v1/querybatch").await >= 1,
            "el enriquecimiento consulta OSV"
        );
    }

    /// M14: los comandos de solo lectura no crean la base de datos.
    #[tokio::test]
    async fn las_consultas_de_solo_lectura_no_crean_la_base_de_datos() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let db = dir.path().join("no-existe.db");

        for args in [
            vec!["diff", "--db", db.to_str().expect("ruta")],
            vec!["history", "--db", db.to_str().expect("ruta")],
        ] {
            let env = EnvPrueba::nuevo().auth_falla().construir();
            let codigo = run_with(cli_de(&args), env).await;
            assert_eq!(codigo, EXIT_OK, "sin base de datos: {args:?}");
            assert!(!db.exists(), "la consulta no debe crear la base: {args:?}");
        }
    }

    /// M15: una URL de GitHub inválida es un error de configuración, no de API.
    #[tokio::test]
    async fn una_url_de_github_invalida_es_error_de_configuracion() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let env = EnvPrueba::nuevo()
            .var("GITHUB_TOKEN", GITHUB_TOKEN)
            .construir();
        let codigo = run_with(
            cli_de(&[
                "scan",
                "--no-enrich",
                "--db",
                dir.path().join("ghwatch.db").to_str().expect("ruta"),
                "--report-dir",
                dir.path().join("informes").to_str().expect("ruta"),
                "--github-base-url",
                "esto no es una url",
            ]),
            env,
        )
        .await;
        assert_eq!(codigo, EXIT_STORE);
    }
}
