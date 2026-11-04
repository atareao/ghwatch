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
    use ghwatch::store::{SCHEMA_VERSION, Store};

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
}
