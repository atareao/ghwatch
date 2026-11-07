//! Tests de integración de la persistencia SQLite.
//!
//! Cada test usa una base de datos SQLite **en memoria** (`sqlite::memory:`) y
//! no toca red ni disco. Los tests viven en el módulo `store` para que el filtro
//! `cargo test store` los seleccione por nombre.

mod store {
    use chrono::{DateTime, TimeZone, Utc};
    use ghwatch::domain::alert::{Alert, AlertSource, Severity};
    use ghwatch::domain::repo::{DependabotState, Repo};
    use ghwatch::domain::{AlertState, Drift};
    use ghwatch::github::GithubError;
    use ghwatch::github::code_scanning::CodeScanningReport;
    use ghwatch::github::secret_scanning::SecretScanningReport;
    use ghwatch::scan::{RepoScan, ScanFailure, ScanSummary};
    use ghwatch::store::{RepoQuery, SCHEMA_VERSION, Store};

    /// Instante determinista a partir de un segundo Unix.
    fn instante(segundos: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(segundos, 0)
            .single()
            .expect("instante válido")
    }

    /// Alerta mínima del origen y severidad indicados.
    fn alerta(source: AlertSource, external_id: &str, severity: Option<Severity>) -> Alert {
        Alert {
            source,
            external_id: external_id.to_string(),
            ghsa_id: None,
            cve_id: None,
            severity,
            cvss: None,
            summary: Some(format!("alerta {external_id}")),
            package: Some("paquete".to_string()),
            ecosystem: Some("pip".to_string()),
            manifest_path: None,
            scope: None,
            vulnerable_range: None,
            first_patched: None,
            secret_type: None,
            created_at: None,
        }
    }

    /// Recolección de un repositorio con Dependabot habilitado y las alertas
    /// indicadas, repartidas por su fuente.
    fn repo_scan(nombre: &str, alertas: Vec<Alert>) -> RepoScan {
        repo_scan_con(nombre, DependabotState::Enabled, alertas)
    }

    /// Recolección de un repositorio con el estado de Dependabot indicado.
    fn repo_scan_con(nombre: &str, dependabot: DependabotState, alertas: Vec<Alert>) -> RepoScan {
        let mut dependabot_alerts = Vec::new();
        let mut secrets = Vec::new();
        let mut code = Vec::new();
        for alerta in alertas {
            match alerta.source {
                AlertSource::Dependabot => dependabot_alerts.push(alerta),
                AlertSource::Secret => secrets.push(alerta),
                AlertSource::Code => code.push(alerta),
            }
        }

        RepoScan {
            repo: Repo {
                name_with_owner: nombre.to_string(),
                private: false,
                archived: false,
                fork: false,
                language: Some("Rust".to_string()),
                pushed_at: Some(instante(0)),
                dependabot,
            },
            dependabot_alerts,
            secrets: SecretScanningReport {
                observable: true,
                alerts: secrets,
            },
            code: CodeScanningReport {
                configured: true,
                alerts: code,
            },
        }
    }

    /// Recolección de un repositorio cuyos secretos **no** se pudieron observar
    /// (la fuente respondió `404`/`403`/sin configurar).
    fn repo_scan_secretos_no_observables(nombre: &str) -> RepoScan {
        let mut scan = repo_scan(nombre, Vec::new());
        scan.secrets = SecretScanningReport {
            observable: false,
            alerts: Vec::new(),
        };
        scan
    }

    /// Resumen con los repositorios correctos y `fallidos` fallos ficticios.
    fn resumen(scanned: Vec<RepoScan>, fallidos: usize) -> ScanSummary {
        ScanSummary {
            scanned,
            failed: (0..fallidos)
                .map(|indice| ScanFailure {
                    repo: format!("atareao/fallido-{indice}"),
                    error: GithubError::NotFound,
                })
                .collect(),
        }
    }

    /// Store nuevo con base de datos en memoria y migraciones aplicadas.
    async fn store() -> Store {
        Store::in_memory().await.expect("store en memoria")
    }

    /// Consulta de alertas **abiertas** (el comportamiento histórico del
    /// subcomando `repos`) con el filtro de severidad mínima indicado.
    fn abiertas(min_severity: Option<Severity>) -> RepoQuery {
        RepoQuery {
            state: Some(AlertState::Open),
            min_severity,
            ..RepoQuery::default()
        }
    }

    /// Las migraciones crean las cuatro tablas y la clave única de idempotencia.
    #[tokio::test]
    async fn las_migraciones_crean_las_cuatro_tablas_y_la_clave_unica() {
        let store = store().await;

        let tablas: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table'")
                .fetch_all(store.pool())
                .await
                .expect("consulta de tablas");

        for tabla in ["repos", "scan_runs", "alerts", "alert_events"] {
            assert!(
                tablas.iter().any(|nombre| nombre == tabla),
                "falta la tabla {tabla}; encontradas: {tablas:?}"
            );
        }

        let ddl: String = sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name = 'alerts'")
            .fetch_one(store.pool())
            .await
            .expect("ddl de alerts");
        let normalizado = ddl.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            normalizado.contains("UNIQUE (source, repo, external_id)"),
            "la tabla alerts no declara la clave única: {normalizado}"
        );
    }

    /// Las migraciones crean los índices que sostienen las consultas del
    /// histórico: `idx_alerts_state` (resolución de desaparecidas) y el
    /// compuesto `idx_alert_events_run_change` (diff por run y cambio).
    #[tokio::test]
    async fn las_migraciones_crean_los_indices_del_historico() {
        let store = store().await;

        let indices: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'index'")
                .fetch_all(store.pool())
                .await
                .expect("consulta de índices");

        for indice in ["idx_alerts_state", "idx_alert_events_run_change"] {
            assert!(
                indices.iter().any(|nombre| nombre == indice),
                "falta el índice {indice}; encontrados: {indices:?}"
            );
        }
    }

    /// Repetir un escaneo idéntico no inserta ni resuelve nada y conserva el
    /// `first_seen_run`.
    #[tokio::test]
    async fn un_escaneo_repetido_no_inserta_ni_resuelve_alertas() {
        let store = store().await;
        let escaneo = resumen(
            vec![repo_scan(
                "atareao/uno",
                vec![
                    alerta(AlertSource::Dependabot, "1", Some(Severity::Medium)),
                    alerta(AlertSource::Secret, "2", None),
                ],
            )],
            0,
        );

        let primero = store
            .persist_scan(&escaneo, instante(1), instante(2))
            .await
            .expect("primer escaneo");
        assert_eq!(primero.new, 2);
        assert_eq!(primero.resolved, 0);

        let segundo = store
            .persist_scan(&escaneo, instante(3), instante(4))
            .await
            .expect("segundo escaneo");
        assert_eq!(segundo.new, 0, "no se inserta ninguna alerta nueva");
        assert_eq!(segundo.resolved, 0, "no se resuelve ninguna alerta");
        assert_eq!(segundo.still_open, 2);
        assert_eq!(
            segundo.severity_up, 0,
            "un escaneo idéntico no registra subidas de severidad"
        );

        let alertas = store.alerts().await.expect("alertas almacenadas");
        assert_eq!(alertas.len(), 2, "no se duplican las alertas");
        assert!(
            alertas.iter().all(|a| a.first_seen_run == primero.run_id),
            "first_seen_run se conserva intacto"
        );
        assert!(
            alertas.iter().all(|a| a.last_seen_run == segundo.run_id),
            "last_seen_run avanza a la segunda ejecución"
        );
    }

    /// El ciclo de vida registra `first_seen_run`, `last_seen_run` y
    /// `resolved_at` a lo largo de cuatro ejecuciones.
    #[tokio::test]
    async fn el_ciclo_de_vida_registra_first_last_y_resolved_at() {
        let store = store().await;
        let con_alerta = || {
            resumen(
                vec![repo_scan(
                    "atareao/uno",
                    vec![alerta(AlertSource::Dependabot, "1", Some(Severity::Medium))],
                )],
                0,
            )
        };

        let run1 = store
            .persist_scan(&con_alerta(), instante(1), instante(2))
            .await
            .expect("run 1");
        let run2 = store
            .persist_scan(&con_alerta(), instante(3), instante(4))
            .await
            .expect("run 2");

        let actual = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].first_seen_run, run1.run_id);
        assert_eq!(actual[0].last_seen_run, run2.run_id);
        assert_eq!(actual[0].resolved_at, None);
        assert_eq!(actual[0].state, AlertState::Open);

        // La alerta desaparece: queda resuelta con `resolved_at` de esa ejecución.
        let sin_alerta = resumen(vec![repo_scan("atareao/uno", Vec::new())], 0);
        let run3 = store
            .persist_scan(&sin_alerta, instante(5), instante(6))
            .await
            .expect("run 3");
        assert_eq!(run3.resolved, 1);
        let resuelta = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(resuelta[0].state, AlertState::Resolved);
        assert_eq!(resuelta[0].resolved_at, Some(instante(6)));
        assert_eq!(
            resuelta[0].last_seen_run, run2.run_id,
            "resolver no altera el último avistamiento"
        );

        // Reaparece: `reopened`, nunca `new`, y conserva `first_seen_run`.
        let run4 = store
            .persist_scan(&con_alerta(), instante(7), instante(8))
            .await
            .expect("run 4");
        assert_eq!(run4.reopened, 1);
        assert_eq!(run4.new, 0);
        let reabierta = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(reabierta[0].state, AlertState::Open);
        assert_eq!(reabierta[0].resolved_at, None);
        assert_eq!(reabierta[0].first_seen_run, run1.run_id);
        assert_eq!(reabierta[0].last_seen_run, run4.run_id);
    }

    /// Una fuente que no se pudo observar no declara resueltas sus alertas: si
    /// Dependabot está deshabilitado, su ausencia significa «no se pudo mirar»,
    /// no «ya no está».
    #[tokio::test]
    async fn no_se_resuelven_alertas_de_una_fuente_no_observada() {
        let store = store().await;
        let con_dependabot = resumen(
            vec![repo_scan(
                "atareao/uno",
                vec![alerta(AlertSource::Dependabot, "1", Some(Severity::High))],
            )],
            0,
        );
        let dependabot_deshabilitado = resumen(
            vec![repo_scan_con(
                "atareao/uno",
                DependabotState::Disabled,
                Vec::new(),
            )],
            0,
        );

        store
            .persist_scan(&con_dependabot, instante(1), instante(2))
            .await
            .expect("run 1");
        let run2 = store
            .persist_scan(&dependabot_deshabilitado, instante(3), instante(4))
            .await
            .expect("run 2");

        assert_eq!(
            run2.resolved, 0,
            "no se resuelve lo que no se pudo observar"
        );
        let alertas = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(alertas[0].state, AlertState::Open);
        assert_eq!(alertas[0].resolved_at, None);
    }

    /// Una fuente de secret scanning **no observable** (404/403/sin configurar)
    /// no declara resueltas sus alertas: su ausencia significa «no se pudo
    /// mirar», no «ya no está».
    #[tokio::test]
    async fn no_se_resuelven_secretos_de_una_fuente_no_observable() {
        let store = store().await;
        let observable = resumen(
            vec![repo_scan(
                "atareao/uno",
                vec![alerta(AlertSource::Secret, "7", None)],
            )],
            0,
        );
        let no_observable = resumen(vec![repo_scan_secretos_no_observables("atareao/uno")], 0);

        store
            .persist_scan(&observable, instante(1), instante(2))
            .await
            .expect("run 1");
        let run2 = store
            .persist_scan(&no_observable, instante(3), instante(4))
            .await
            .expect("run 2");

        assert_eq!(
            run2.resolved, 0,
            "no se resuelve lo que no se pudo observar"
        );
        let alertas = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(alertas[0].state, AlertState::Open);
        assert_eq!(alertas[0].resolved_at, None);
    }

    /// La subida de severidad se registra como `severity_up` y actualiza la
    /// severidad almacenada.
    #[tokio::test]
    async fn la_subida_de_severidad_queda_como_severity_up() {
        let store = store().await;
        let media = resumen(
            vec![repo_scan(
                "atareao/uno",
                vec![alerta(AlertSource::Dependabot, "1", Some(Severity::Medium))],
            )],
            0,
        );
        let critica = resumen(
            vec![repo_scan(
                "atareao/uno",
                vec![alerta(
                    AlertSource::Dependabot,
                    "1",
                    Some(Severity::Critical),
                )],
            )],
            0,
        );

        let run1 = store
            .persist_scan(&media, instante(1), instante(2))
            .await
            .expect("run 1");
        assert_eq!(run1.new, 1);

        let run2 = store
            .persist_scan(&critica, instante(3), instante(4))
            .await
            .expect("run 2");
        assert_eq!(run2.severity_up, 1);
        assert_eq!(run2.still_open, 0);

        let almacenada = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(almacenada[0].severity, Some(Severity::Critical));

        let cambios: Vec<Drift> = store
            .events_for_run(run2.run_id)
            .await
            .expect("eventos de la ejecución")
            .into_iter()
            .map(|evento| evento.change)
            .collect();
        assert_eq!(cambios, vec![Drift::SeverityUp]);
    }

    /// Cada ejecución queda registrada como una instantánea comparable.
    #[tokio::test]
    async fn cada_ejecucion_queda_como_instantanea_comparable() {
        let store = store().await;
        let escaneo = resumen(
            vec![repo_scan(
                "atareao/uno",
                vec![alerta(AlertSource::Dependabot, "1", None)],
            )],
            1,
        );

        store
            .persist_scan(&escaneo, instante(1), instante(2))
            .await
            .expect("run 1");
        store
            .persist_scan(&escaneo, instante(3), instante(4))
            .await
            .expect("run 2");

        let runs = store.runs().await.expect("ejecuciones");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].total_repos, 2);
        assert_eq!(runs[0].ok, 1);
        assert_eq!(runs[0].failed, 1);
        assert_eq!(runs[0].alerts_total, 1);
        assert_eq!(runs[0].started_at, instante(1));
        assert_eq!(runs[0].finished_at, instante(2));
        assert_eq!(runs[0].schema_version, SCHEMA_VERSION);
        assert!(
            runs[0].id < runs[1].id,
            "el identificador crece con el tiempo"
        );
    }

    /// El histórico permite consultar la serie por ejecución y el diff respecto
    /// a una ejecución concreta.
    #[tokio::test]
    async fn el_historico_permite_comparar_ejecuciones() {
        let store = store().await;
        let con_alerta = resumen(
            vec![repo_scan(
                "atareao/uno",
                vec![alerta(AlertSource::Dependabot, "1", None)],
            )],
            0,
        );
        let sin_alerta = resumen(vec![repo_scan("atareao/uno", Vec::new())], 0);

        let run1 = store
            .persist_scan(&con_alerta, instante(1), instante(2))
            .await
            .expect("run 1");
        let run2 = store
            .persist_scan(&sin_alerta, instante(3), instante(4))
            .await
            .expect("run 2");
        store
            .persist_scan(&con_alerta, instante(5), instante(6))
            .await
            .expect("run 3");

        let serie = store.history().await.expect("histórico");
        assert_eq!(serie.len(), 3);
        assert_eq!(serie[0].new, 1);
        assert_eq!(serie[0].resolved, 0);
        assert_eq!(serie[0].total, 1);
        assert_eq!(serie[1].new, 0);
        assert_eq!(serie[1].resolved, 1);
        assert_eq!(serie[2].new, 0, "la reaparición no cuenta como nueva");

        let primero = store.diff(run1.run_id).await.expect("diff del run 1");
        assert_eq!(primero.new.len(), 1);
        assert!(primero.resolved.is_empty());

        let segundo = store.diff(run2.run_id).await.expect("diff del run 2");
        assert_eq!(segundo.resolved.len(), 1);
        assert!(segundo.new.is_empty());
    }

    /// Una alerta de secreto persiste y recupera su `secret_type`.
    #[tokio::test]
    async fn una_alerta_de_secreto_persiste_y_recupera_su_tipo() {
        let store = store().await;
        let mut secreto = alerta(AlertSource::Secret, "7", None);
        secreto.secret_type = Some("adafruit_io_key".to_string());
        let escaneo = resumen(vec![repo_scan("atareao/uno", vec![secreto])], 0);

        store
            .persist_scan(&escaneo, instante(1), instante(2))
            .await
            .expect("escaneo");

        let alertas = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(alertas.len(), 1);
        assert_eq!(
            alertas[0].secret_type.as_deref(),
            Some("adafruit_io_key"),
            "el tipo de secreto debe persistirse y recuperarse"
        );
    }

    /// M6 — La fecha de creación de la alerta se persiste y se recupera, sin
    /// dejar deriva entre el modelo y el esquema.
    #[tokio::test]
    async fn la_fecha_de_creacion_persiste_y_se_recupera() {
        let store = store().await;
        let mut con_fecha = alerta(AlertSource::Dependabot, "1", Some(Severity::High));
        con_fecha.created_at = Some(instante(1_700_000_000));
        let escaneo = resumen(vec![repo_scan("atareao/uno", vec![con_fecha])], 0);

        store
            .persist_scan(&escaneo, instante(1), instante(2))
            .await
            .expect("escaneo");

        let alertas = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(
            alertas[0].created_at,
            Some(instante(1_700_000_000)),
            "la fecha de creación debe persistirse y recuperarse"
        );
    }

    /// Un mismo `external_id` en dos fuentes distintas del mismo repositorio son
    /// dos alertas independientes: la clave única incluye `source`.
    #[tokio::test]
    async fn el_mismo_external_id_en_dos_fuentes_son_alertas_distintas() {
        let store = store().await;
        let escaneo = resumen(
            vec![repo_scan(
                "atareao/uno",
                vec![
                    alerta(AlertSource::Dependabot, "1", Some(Severity::High)),
                    alerta(AlertSource::Secret, "1", None),
                ],
            )],
            0,
        );

        let primero = store
            .persist_scan(&escaneo, instante(1), instante(2))
            .await
            .expect("escaneo");
        assert_eq!(
            primero.new, 2,
            "dos fuentes con el mismo id son dos alertas"
        );

        let alertas = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(alertas.len(), 2, "no se pisan entre fuentes");
        assert!(
            alertas.iter().any(|a| a.source == AlertSource::Dependabot),
            "debe conservarse la alerta de Dependabot"
        );
        assert!(
            alertas.iter().any(|a| a.source == AlertSource::Secret),
            "debe conservarse la alerta de secreto"
        );
    }

    /// Una alerta resuelta que sigue ausente en ejecuciones posteriores conserva
    /// su `resolved_at` intacto y no vuelve a registrarse como resuelta.
    #[tokio::test]
    async fn una_alerta_resuelta_ausente_conserva_su_resolved_at() {
        let store = store().await;
        let con_alerta = resumen(
            vec![repo_scan(
                "atareao/uno",
                vec![alerta(AlertSource::Dependabot, "1", Some(Severity::Medium))],
            )],
            0,
        );
        let sin_alerta = resumen(vec![repo_scan("atareao/uno", Vec::new())], 0);

        store
            .persist_scan(&con_alerta, instante(1), instante(2))
            .await
            .expect("run 1");
        let run2 = store
            .persist_scan(&sin_alerta, instante(3), instante(4))
            .await
            .expect("run 2");
        assert_eq!(run2.resolved, 1);

        store
            .persist_scan(&sin_alerta, instante(5), instante(6))
            .await
            .expect("run 3");
        let run4 = store
            .persist_scan(&sin_alerta, instante(7), instante(8))
            .await
            .expect("run 4");
        assert_eq!(
            run4.resolved, 0,
            "ya estaba resuelta: no vuelve a resolverse"
        );

        let alertas = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(alertas[0].state, AlertState::Resolved);
        assert_eq!(
            alertas[0].resolved_at,
            Some(instante(4)),
            "el `resolved_at` de la resolución original no se altera"
        );
    }

    /// 1.1 — La agregación por repositorio cuenta solo las alertas **abiertas**:
    /// las resueltas no aparecen ni suman.
    #[tokio::test]
    async fn repo_alert_counts_cuenta_solo_las_alertas_abiertas() {
        let store = store().await;

        // Run 1: tres repositorios con alertas abiertas.
        let inicial = resumen(
            vec![
                repo_scan(
                    "atareao/uno",
                    vec![
                        alerta(AlertSource::Dependabot, "1", Some(Severity::High)),
                        alerta(AlertSource::Dependabot, "2", Some(Severity::Medium)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::Low)),
                    ],
                ),
                repo_scan(
                    "atareao/dos",
                    vec![
                        alerta(AlertSource::Dependabot, "4", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "5", Some(Severity::High)),
                    ],
                ),
                repo_scan(
                    "atareao/tres",
                    vec![alerta(AlertSource::Dependabot, "6", Some(Severity::Medium))],
                ),
            ],
            0,
        );
        store
            .persist_scan(&inicial, instante(1), instante(2))
            .await
            .expect("run 1");

        // Run 2: desaparecen las alertas 2 y 3 (uno) y la 6 (tres) → resueltas.
        let posterior = resumen(
            vec![
                repo_scan(
                    "atareao/uno",
                    vec![alerta(AlertSource::Dependabot, "1", Some(Severity::High))],
                ),
                repo_scan(
                    "atareao/dos",
                    vec![
                        alerta(AlertSource::Dependabot, "4", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "5", Some(Severity::High)),
                    ],
                ),
                repo_scan("atareao/tres", Vec::new()),
            ],
            0,
        );
        store
            .persist_scan(&posterior, instante(3), instante(4))
            .await
            .expect("run 2");

        let listado = store
            .repo_alert_counts(&abiertas(None))
            .await
            .expect("recuento por repositorio");

        // Solo los repositorios con alertas abiertas; `tres` queda fuera.
        assert_eq!(listado.repos.len(), 2, "solo dos repositorios con abiertas");
        assert_eq!(
            listado.known_repos, 3,
            "los tres repositorios siguen conocidos"
        );
        assert_eq!(listado.total_alerts, 3, "solo cuentan las alertas abiertas");

        let dos = &listado.repos[0];
        assert_eq!(dos.repo, "atareao/dos");
        assert_eq!(dos.alerts, 2);
        let uno = &listado.repos[1];
        assert_eq!(uno.repo, "atareao/uno");
        assert_eq!(uno.alerts, 1, "las alertas resueltas no cuentan");
        assert!(
            listado.repos.iter().all(|repo| repo.repo != "atareao/tres"),
            "un repositorio sin alertas abiertas no aparece"
        );
    }

    /// 1.2 — Orden total (recuento descendente y, a igualdad, nombre
    /// ascendente), desglose por severidad (con `NULL` contado como `unknown`)
    /// y exclusión de las alertas resueltas.
    ///
    /// Los tres repositorios empatados a cinco alertas se insertan en orden
    /// **inverso** al alfabético (`zzz`, `beta`, `alfa`), de modo que la
    /// secuencia esperada no puede satisfacerse ni con el orden de inserción ni
    /// con ningún orden implícito: solo con el desempate explícito `repo ASC`.
    #[tokio::test]
    async fn repo_alert_counts_ordena_y_desglosa_por_severidad() {
        let store = store().await;

        let escaneo = resumen(
            vec![
                repo_scan(
                    "atareao/zzz",
                    vec![
                        alerta(AlertSource::Dependabot, "1", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "2", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "4", Some(Severity::High)),
                        alerta(AlertSource::Dependabot, "5", Some(Severity::Low)),
                    ],
                ),
                repo_scan(
                    "atareao/beta",
                    vec![
                        alerta(AlertSource::Dependabot, "1", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "2", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::High)),
                        alerta(AlertSource::Dependabot, "4", Some(Severity::Low)),
                        // `unknown` explícito: también cuenta como desconocida.
                        alerta(AlertSource::Dependabot, "5", Some(Severity::Unknown)),
                    ],
                ),
                repo_scan(
                    "atareao/alfa",
                    vec![
                        alerta(AlertSource::Dependabot, "1", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "2", Some(Severity::High)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::Medium)),
                        alerta(AlertSource::Dependabot, "4", Some(Severity::Low)),
                        // Severidad nula: debe contar como `unknown`.
                        alerta(AlertSource::Dependabot, "5", None),
                    ],
                ),
                repo_scan(
                    "atareao/gamma",
                    vec![
                        alerta(AlertSource::Dependabot, "1", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "2", Some(Severity::Medium)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::Low)),
                    ],
                ),
            ],
            0,
        );
        store
            .persist_scan(&escaneo, instante(1), instante(2))
            .await
            .expect("run 1");

        // Run 2: la crítica de gamma se resuelve (no debe contar).
        let posterior = resumen(
            vec![
                repo_scan(
                    "atareao/zzz",
                    vec![
                        alerta(AlertSource::Dependabot, "1", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "2", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "4", Some(Severity::High)),
                        alerta(AlertSource::Dependabot, "5", Some(Severity::Low)),
                    ],
                ),
                repo_scan(
                    "atareao/beta",
                    vec![
                        alerta(AlertSource::Dependabot, "1", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "2", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::High)),
                        alerta(AlertSource::Dependabot, "4", Some(Severity::Low)),
                        alerta(AlertSource::Dependabot, "5", Some(Severity::Unknown)),
                    ],
                ),
                repo_scan(
                    "atareao/alfa",
                    vec![
                        alerta(AlertSource::Dependabot, "1", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "2", Some(Severity::High)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::Medium)),
                        alerta(AlertSource::Dependabot, "4", Some(Severity::Low)),
                        alerta(AlertSource::Dependabot, "5", None),
                    ],
                ),
                repo_scan(
                    "atareao/gamma",
                    vec![
                        alerta(AlertSource::Dependabot, "2", Some(Severity::Medium)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::Low)),
                    ],
                ),
            ],
            0,
        );
        store
            .persist_scan(&posterior, instante(3), instante(4))
            .await
            .expect("run 2");

        let listado = store
            .repo_alert_counts(&abiertas(None))
            .await
            .expect("recuento por repositorio");

        assert_eq!(listado.known_repos, 4);
        assert_eq!(listado.total_alerts, 17);
        assert_eq!(
            listado
                .repos
                .iter()
                .map(|repo| repo.repo.as_str())
                .collect::<Vec<_>>(),
            vec![
                "atareao/alfa",
                "atareao/beta",
                "atareao/zzz",
                "atareao/gamma"
            ],
            "los tres de cinco empatan por nombre ascendente (no por inserción); gamma (2) va última"
        );

        let alfa = &listado.repos[0];
        assert_eq!(
            (
                alfa.alerts,
                alfa.critical,
                alfa.high,
                alfa.medium,
                alfa.low,
                alfa.unknown
            ),
            (5, 1, 1, 1, 1, 1),
            "alfa: una de cada severidad, incluida la de severidad nula"
        );

        let beta = &listado.repos[1];
        assert_eq!(
            (
                beta.alerts,
                beta.critical,
                beta.high,
                beta.medium,
                beta.low,
                beta.unknown
            ),
            (5, 2, 1, 0, 1, 1),
            "beta: dos críticas y una `unknown` explícita"
        );

        let zzz = &listado.repos[2];
        assert_eq!(
            (
                zzz.alerts,
                zzz.critical,
                zzz.high,
                zzz.medium,
                zzz.low,
                zzz.unknown
            ),
            (5, 3, 1, 0, 1, 0),
            "zzz: tres críticas, y sin embargo se insertó el primero"
        );

        let gamma = &listado.repos[3];
        assert_eq!(
            (gamma.alerts, gamma.critical, gamma.medium, gamma.low),
            (2, 0, 1, 1),
            "gamma: la crítica resuelta no cuenta"
        );

        for repo in &listado.repos {
            assert_eq!(
                repo.critical + repo.high + repo.medium + repo.low + repo.unknown,
                repo.alerts,
                "el desglose de {} suma su total",
                repo.repo
            );
        }
    }

    /// 1.3 — `min_severity` conserva los repositorios con al menos una alerta de
    /// esa severidad o superior, **sin** recortar el recuento ni el desglose.
    #[tokio::test]
    async fn repo_alert_counts_filtra_por_severidad_minima() {
        let store = store().await;
        let escaneo = resumen(
            vec![
                repo_scan(
                    "atareao/critico",
                    vec![
                        alerta(AlertSource::Dependabot, "1", Some(Severity::Critical)),
                        alerta(AlertSource::Dependabot, "2", Some(Severity::Medium)),
                        alerta(AlertSource::Dependabot, "3", Some(Severity::Low)),
                    ],
                ),
                repo_scan(
                    "atareao/solo-medio",
                    vec![alerta(AlertSource::Dependabot, "1", Some(Severity::Medium))],
                ),
                repo_scan(
                    "atareao/solo-unknown",
                    vec![alerta(AlertSource::Dependabot, "1", None)],
                ),
            ],
            0,
        );
        store
            .persist_scan(&escaneo, instante(1), instante(2))
            .await
            .expect("escaneo");

        // Sin filtro aparecen los tres repositorios con alertas abiertas.
        let todos = store
            .repo_alert_counts(&abiertas(None))
            .await
            .expect("sin filtro");
        assert_eq!(todos.repos.len(), 3);

        // Con `critical`: solo el repositorio con una crítica, y **entero**.
        let criticas = store
            .repo_alert_counts(&abiertas(Some(Severity::Critical)))
            .await
            .expect("filtro critical");
        assert_eq!(criticas.repos.len(), 1, "solo un repositorio con críticas");
        let critico = &criticas.repos[0];
        assert_eq!(critico.repo, "atareao/critico");
        assert_eq!(critico.alerts, 3, "el recuento no se recorta");
        assert_eq!(
            (
                critico.critical,
                critico.high,
                critico.medium,
                critico.low,
                critico.unknown
            ),
            (1, 0, 1, 1, 0),
            "el desglose sigue mostrando todas sus alertas abiertas"
        );

        // Un repositorio con solo una alerta sin severidad no alcanza `critical`…
        assert!(
            criticas
                .repos
                .iter()
                .all(|repo| repo.repo != "atareao/solo-unknown"),
            "una alerta desconocida no alcanza la severidad crítica"
        );

        // …pero sí pasa el filtro `unknown` (rank 0, que deja pasar todo).
        let desconocidas = store
            .repo_alert_counts(&abiertas(Some(Severity::Unknown)))
            .await
            .expect("filtro unknown");
        assert_eq!(
            desconocidas.repos.len(),
            3,
            "el filtro `unknown` deja pasar todo repositorio con alguna alerta"
        );
        assert!(
            desconocidas
                .repos
                .iter()
                .any(|repo| repo.repo == "atareao/solo-unknown"),
            "el repositorio con severidad nula aparece con el filtro `unknown`"
        );
    }

    /// 1.7.2 — Una severidad **ajena** al conjunto cerrado no debe perderse: la
    /// columna es `TEXT` sin `CHECK`, así que puede contener un valor que no es
    /// ninguno de los cinco; esa fila debe contar como desconocida y el desglose
    /// debe seguir sumando el total (invariante del bucket `unknown`).
    #[tokio::test]
    async fn repo_alert_counts_cuenta_como_desconocida_una_severidad_ajena() {
        let store = store().await;

        // Inserción directa: el esquema no valida `severity`, de modo que una
        // fila con un valor fuera del enum es posible y debe seguir contando.
        // La fila de `repos` es necesaria porque la agregación parte del
        // inventario con un `LEFT JOIN`: sin ella, la alerta no tendría
        // repositorio del que colgar.
        sqlx::query("INSERT INTO repos (name_with_owner) VALUES ('atareao/ajena')")
            .execute(store.pool())
            .await
            .expect("repositorio del inventario");
        sqlx::query(
            "INSERT INTO alerts (repo, source, external_id, severity, state, \
             first_seen_run, last_seen_run) \
             VALUES ('atareao/ajena', 'dependabot', '1', 'high', 'open', 1, 1), \
                    ('atareao/ajena', 'dependabot', '2', 'moderate', 'open', 1, 1)",
        )
        .execute(store.pool())
        .await
        .expect("inserción directa de alertas");

        let listado = store
            .repo_alert_counts(&abiertas(None))
            .await
            .expect("recuento por repositorio");

        assert_eq!(listado.repos.len(), 1);
        let repo = &listado.repos[0];
        assert_eq!(repo.alerts, 2);
        assert_eq!(repo.high, 1, "la severidad conocida sigue en su bucket");
        assert_eq!(
            repo.unknown, 1,
            "una severidad ajena al conjunto cuenta como desconocida"
        );
        assert_eq!(
            repo.critical + repo.high + repo.medium + repo.low + repo.unknown,
            repo.alerts,
            "el desglose suma el total aunque haya una severidad ajena"
        );
    }

    /// 1.1 — `RepoQuery.state` elige el estado de las alertas contadas:
    /// `Some(Open)`, `Some(Resolved)` y `None` (todos).
    #[tokio::test]
    async fn repo_query_filtra_por_estado() {
        let store = store().await;

        // Run 1: una alerta abierta.
        store
            .persist_scan(
                &resumen(
                    vec![repo_scan(
                        "atareao/uno",
                        vec![alerta(AlertSource::Dependabot, "1", Some(Severity::High))],
                    )],
                    0,
                ),
                instante(1),
                instante(2),
            )
            .await
            .expect("run 1");

        // Run 2: la 1 desaparece (queda resuelta) y aparece la 2 (abierta).
        store
            .persist_scan(
                &resumen(
                    vec![repo_scan(
                        "atareao/uno",
                        vec![alerta(AlertSource::Dependabot, "2", Some(Severity::Medium))],
                    )],
                    0,
                ),
                instante(3),
                instante(4),
            )
            .await
            .expect("run 2");

        let solo_abiertas = store
            .repo_alert_counts(&RepoQuery {
                state: Some(AlertState::Open),
                ..RepoQuery::default()
            })
            .await
            .expect("estado open");
        assert_eq!(solo_abiertas.repos.len(), 1);
        assert_eq!(solo_abiertas.repos[0].alerts, 1, "solo la alerta abierta");
        assert_eq!(
            solo_abiertas.repos[0].medium, 1,
            "el desglose es el de la alerta abierta"
        );
        assert_eq!(
            solo_abiertas.repos[0].high, 0,
            "la resuelta no entra en el desglose"
        );

        let resueltas = store
            .repo_alert_counts(&RepoQuery {
                state: Some(AlertState::Resolved),
                ..RepoQuery::default()
            })
            .await
            .expect("estado resolved");
        assert_eq!(resueltas.repos.len(), 1);
        assert_eq!(resueltas.repos[0].alerts, 1, "solo la alerta resuelta");
        assert_eq!(resueltas.repos[0].high, 1);

        let todas = store
            .repo_alert_counts(&RepoQuery::default())
            .await
            .expect("todos los estados");
        assert_eq!(
            todas.repos[0].alerts, 2,
            "sin filtro de estado cuentan abiertas y resueltas"
        );
    }

    /// 1.1 — `RepoQuery.source` cuenta solo las alertas de esa fuente, con su
    /// propio desglose por severidad.
    #[tokio::test]
    async fn repo_query_filtra_por_fuente() {
        let store = store().await;
        store
            .persist_scan(
                &resumen(
                    vec![repo_scan(
                        "atareao/mixto",
                        vec![
                            alerta(AlertSource::Dependabot, "d1", Some(Severity::High)),
                            alerta(AlertSource::Secret, "s1", Some(Severity::Critical)),
                        ],
                    )],
                    0,
                ),
                instante(1),
                instante(2),
            )
            .await
            .expect("escaneo");

        let dependabot = store
            .repo_alert_counts(&RepoQuery {
                source: Some(AlertSource::Dependabot),
                ..RepoQuery::default()
            })
            .await
            .expect("fuente dependabot");
        assert_eq!(dependabot.repos.len(), 1);
        assert_eq!(dependabot.repos[0].alerts, 1, "solo las de Dependabot");
        assert_eq!(dependabot.repos[0].high, 1);
        assert_eq!(dependabot.repos[0].critical, 0);

        let secret = store
            .repo_alert_counts(&RepoQuery {
                source: Some(AlertSource::Secret),
                ..RepoQuery::default()
            })
            .await
            .expect("fuente secret");
        assert_eq!(secret.repos.len(), 1);
        assert_eq!(secret.repos[0].alerts, 1, "solo las de secret scanning");
        assert_eq!(secret.repos[0].critical, 1);
        assert_eq!(secret.repos[0].high, 0);
    }

    /// 1.1 — `RepoQuery.include_empty` decide si aparece un repositorio del
    /// inventario sin ninguna alerta.
    #[tokio::test]
    async fn repo_query_include_empty() {
        let store = store().await;
        store
            .persist_scan(
                &resumen(
                    vec![
                        repo_scan(
                            "atareao/con-alertas",
                            vec![alerta(AlertSource::Dependabot, "1", Some(Severity::High))],
                        ),
                        repo_scan("atareao/vacio", Vec::new()),
                    ],
                    0,
                ),
                instante(1),
                instante(2),
            )
            .await
            .expect("escaneo");

        let sin_vacios = store
            .repo_alert_counts(&RepoQuery::default())
            .await
            .expect("sin include_empty");
        assert!(
            sin_vacios.repos.iter().all(|r| r.repo != "atareao/vacio"),
            "sin `include_empty` el repositorio vacío no aparece"
        );

        let con_vacios = store
            .repo_alert_counts(&RepoQuery {
                include_empty: true,
                ..RepoQuery::default()
            })
            .await
            .expect("con include_empty");
        assert!(
            con_vacios.repos.iter().any(|r| r.repo == "atareao/vacio"),
            "con `include_empty` el repositorio vacío aparece"
        );
    }

    /// 1.2 — El repositorio sin alertas aparece con los cinco cubos a cero (los
    /// tres guardianes del `LEFT JOIN`) y el `HAVING` de severidad mínima no lo
    /// deja pasar cuando se exige una severidad.
    #[tokio::test]
    async fn repo_query_repositorio_vacio_cero_y_guardianes() {
        let store = store().await;
        store
            .persist_scan(
                &resumen(
                    vec![
                        repo_scan(
                            "atareao/con-alertas",
                            vec![alerta(
                                AlertSource::Dependabot,
                                "1",
                                Some(Severity::Critical),
                            )],
                        ),
                        repo_scan("atareao/vacio", Vec::new()),
                    ],
                    0,
                ),
                instante(1),
                instante(2),
            )
            .await
            .expect("escaneo");

        let listado = store
            .repo_alert_counts(&RepoQuery {
                include_empty: true,
                ..RepoQuery::default()
            })
            .await
            .expect("include_empty");
        let vacio = listado
            .repos
            .iter()
            .find(|r| r.repo == "atareao/vacio")
            .expect("el repositorio vacío aparece");
        assert_eq!(vacio.alerts, 0, "`COUNT(a.id)`, no `COUNT(*)`");
        assert_eq!(
            vacio.critical + vacio.high + vacio.medium + vacio.low + vacio.unknown,
            0,
            "ningún cubo cuenta la fila nula del `LEFT JOIN`"
        );

        // `min_severity = Critical` (rank 4) con `include_empty`: el repositorio
        // vacío no pasa el `HAVING` porque `COALESCE(MAX(...), 0) = 0 < 4`.
        let solo_criticas = store
            .repo_alert_counts(&RepoQuery {
                min_severity: Some(Severity::Critical),
                include_empty: true,
                ..RepoQuery::default()
            })
            .await
            .expect("critical + include_empty");
        assert!(
            solo_criticas
                .repos
                .iter()
                .all(|r| r.repo != "atareao/vacio"),
            "el repositorio vacío no alcanza la severidad mínima"
        );
        assert!(
            solo_criticas
                .repos
                .iter()
                .any(|r| r.repo == "atareao/con-alertas"),
            "el repositorio con una crítica sí pasa el filtro"
        );
    }

    /// 1.3 — `repos_with_alerts` cuenta solo los repositorios con al menos una
    /// alerta, y los repositorios vacíos se ordenan al final por nombre.
    #[tokio::test]
    async fn repo_query_repos_with_alerts_y_orden_con_vacios() {
        let store = store().await;
        store
            .persist_scan(
                &resumen(
                    vec![
                        repo_scan(
                            "atareao/b-con-alertas",
                            vec![
                                alerta(AlertSource::Dependabot, "1", Some(Severity::High)),
                                alerta(AlertSource::Dependabot, "2", Some(Severity::Low)),
                            ],
                        ),
                        repo_scan(
                            "atareao/a-con-alertas",
                            vec![alerta(AlertSource::Dependabot, "1", Some(Severity::High))],
                        ),
                        repo_scan("atareao/z-vacio", Vec::new()),
                        repo_scan("atareao/y-vacio", Vec::new()),
                    ],
                    0,
                ),
                instante(1),
                instante(2),
            )
            .await
            .expect("escaneo");

        let listado = store
            .repo_alert_counts(&RepoQuery {
                include_empty: true,
                ..RepoQuery::default()
            })
            .await
            .expect("include_empty");

        assert_eq!(
            listado.repos.len(),
            4,
            "los cuatro repositorios del inventario"
        );
        assert_eq!(listado.repos_with_alerts, 2, "solo dos tienen alertas");
        assert!(
            listado.repos_with_alerts < i64::try_from(listado.repos.len()).expect("len"),
            "hay repositorios sin alertas dentro del conjunto"
        );

        assert_eq!(
            listado
                .repos
                .iter()
                .map(|r| r.repo.as_str())
                .collect::<Vec<_>>(),
            vec![
                "atareao/b-con-alertas",
                "atareao/a-con-alertas",
                "atareao/y-vacio",
                "atareao/z-vacio",
            ],
            "más alertas primero y los vacíos al final, por nombre ascendente"
        );
    }
}
