//! Tests de integración del histórico: serie por ejecución y diff entre runs.
//!
//! Cada test usa una base de datos SQLite **en memoria** (`sqlite::memory:`) y no
//! toca red ni disco. Los tests viven en el módulo `history` para que el filtro
//! `cargo test history` los seleccione por nombre.
//!
//! La spec `alert-history` pide, para el subcomando `diff`, los «cambios respecto
//! a la ejecución anterior» —y `PLAN.md` (§T2.4) lo concreta como «cambios desde
//! el último run»—, de modo que la superficie que se prueba aquí es
//! [`Store::history`] (serie completa) y [`Store::latest_diff`] (diff del último
//! run), con [`Store::diff`] como primitiva de una ejecución concreta.

mod history {
    use chrono::{DateTime, TimeZone, Utc};
    use ghwatch::domain::alert::{Alert, AlertSource, Severity};
    use ghwatch::domain::repo::{DependabotState, Repo};
    use ghwatch::domain::{AlertState, Drift};
    use ghwatch::github::code_scanning::CodeScanningReport;
    use ghwatch::github::secret_scanning::SecretScanningReport;
    use ghwatch::scan::{RepoScan, ScanFailure, ScanSummary};
    use ghwatch::store::{Diff, HistoryPoint, Store};

    /// Instante determinista a partir de un segundo Unix.
    fn instante(segundos: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(segundos, 0)
            .single()
            .expect("instante válido")
    }

    /// Alerta de Dependabot con el identificador externo indicado.
    fn alerta(external_id: &str) -> Alert {
        Alert {
            source: AlertSource::Dependabot,
            external_id: external_id.to_string(),
            ghsa_id: None,
            cve_id: None,
            severity: Some(Severity::Medium),
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
    /// indicadas.
    fn repo_scan(nombre: &str, alertas: Vec<Alert>) -> RepoScan {
        RepoScan {
            repo: Repo {
                name_with_owner: nombre.to_string(),
                private: false,
                archived: false,
                fork: false,
                language: Some("Rust".to_string()),
                pushed_at: Some(instante(0)),
                dependabot: DependabotState::Enabled,
            },
            dependabot_alerts: alertas,
            secrets: SecretScanningReport {
                observable: true,
                alerts: Vec::new(),
            },
            code: CodeScanningReport {
                configured: true,
                alerts: Vec::new(),
            },
        }
    }

    /// Escaneo de `atareao/uno` con las alertas de Dependabot indicadas por su
    /// identificador externo.
    fn escaneo(ids: &[&str]) -> ScanSummary {
        ScanSummary {
            scanned: vec![repo_scan(
                "atareao/uno",
                ids.iter().map(|id| alerta(id)).collect(),
            )],
            failed: Vec::<ScanFailure>::new(),
        }
    }

    /// Store nuevo con base de datos en memoria y migraciones aplicadas.
    async fn store() -> Store {
        Store::in_memory().await.expect("store en memoria")
    }

    /// Cinco ejecuciones almacenadas producen una serie con fecha, total, nuevas
    /// y resueltas por ejecución, en orden cronológico.
    #[tokio::test]
    async fn cinco_ejecuciones_producen_la_serie_historica_completa() {
        let store = store().await;

        // Serie planificada: (ids presentes, total, nuevas, resueltas).
        let guion: [(&[&str], i64, i64, i64); 5] = [
            (&["1", "2"], 2, 2, 0),
            (&["1", "2", "3"], 3, 1, 0),
            (&["1", "3"], 2, 0, 1),
            (&["1", "3"], 2, 0, 0),
            (&["1"], 1, 0, 1),
        ];

        let mut ids_creados = Vec::new();
        for (indice, (ids, ..)) in guion.iter().enumerate() {
            let inicio = instante(1 + 2 * indice as i64);
            let fin = instante(2 + 2 * indice as i64);
            let outcome = store
                .persist_scan(&escaneo(ids), inicio, fin)
                .await
                .expect("escaneo persistido");
            ids_creados.push(outcome.run_id);
        }

        let serie = store.history().await.expect("serie histórica");
        assert_eq!(serie.len(), 5, "una entrada por ejecución");

        let esperados: Vec<HistoryPoint> = guion
            .iter()
            .enumerate()
            .map(|(indice, (_, total, new, resolved))| HistoryPoint {
                run_id: ids_creados[indice],
                started_at: instante(1 + 2 * indice as i64),
                total: *total,
                new: *new,
                resolved: *resolved,
            })
            .collect();

        assert_eq!(serie, esperados, "serie completa, por ejecución y ordenada");
        assert!(
            serie.windows(2).all(|par| par[0].run_id < par[1].run_id),
            "las ejecuciones van en orden cronológico"
        );
    }

    /// El diff de una ejecución lista, por separado, las alertas nuevas y las
    /// resueltas; el diff del último run coincide con el de su ejecución.
    #[tokio::test]
    async fn el_diff_lista_nuevas_y_resueltas_por_separado() {
        let store = store().await;

        store
            .persist_scan(&escaneo(&["1"]), instante(1), instante(2))
            .await
            .expect("run 1");
        let run2 = store
            .persist_scan(&escaneo(&["1", "2"]), instante(3), instante(4))
            .await
            .expect("run 2");
        let run3 = store
            .persist_scan(&escaneo(&["1"]), instante(5), instante(6))
            .await
            .expect("run 3");

        let segundo = store.diff(run2.run_id).await.expect("diff del run 2");
        assert_eq!(ids(&segundo.new), vec!["2"], "el run 2 trae una nueva");
        assert!(segundo.resolved.is_empty(), "el run 2 no resuelve nada");

        let tercero = store.diff(run3.run_id).await.expect("diff del run 3");
        assert!(tercero.new.is_empty(), "el run 3 no trae ninguna nueva");
        assert_eq!(ids(&tercero.resolved), vec!["2"], "el run 3 resuelve una");
        assert_eq!(
            tercero.resolved[0].state,
            AlertState::Resolved,
            "la alerta listada queda resuelta"
        );

        let ultimo = store
            .latest_diff()
            .await
            .expect("diff del último run")
            .expect("hay ejecuciones");
        assert_eq!(ultimo, tercero, "el último run es el run 3");
    }

    /// El diff de una ejecución es **histórico**: refleja el estado que la
    /// alerta tenía en ese run, no el estado actual. Una alerta resuelta en el
    /// run 2 y reabierta en el run 3 sigue apareciendo como `resolved` —con su
    /// `resolved_at` del run 2— al consultar el diff del run 2.
    #[tokio::test]
    async fn el_diff_de_un_run_no_cambia_al_reabrir_la_alerta_despues() {
        let store = store().await;

        store
            .persist_scan(&escaneo(&["1"]), instante(1), instante(2))
            .await
            .expect("run 1");
        let run2 = store
            .persist_scan(&escaneo(&[]), instante(3), instante(4))
            .await
            .expect("run 2");
        store
            .persist_scan(&escaneo(&["1"]), instante(5), instante(6))
            .await
            .expect("run 3");

        let segundo = store.diff(run2.run_id).await.expect("diff del run 2");
        assert_eq!(
            ids(&segundo.resolved),
            vec!["1"],
            "el run 2 resuelve la alerta"
        );
        assert_eq!(
            segundo.resolved[0].state,
            AlertState::Resolved,
            "el diff muestra el estado histórico del run consultado, no el actual"
        );
        assert_eq!(
            segundo.resolved[0].resolved_at,
            Some(instante(4)),
            "el `resolved_at` del diff es el de la ejecución consultada"
        );

        // El estado **actual** sí refleja la reapertura; el diff del run 2 no.
        let actual = store
            .alerts_for_repo("atareao/uno")
            .await
            .expect("alertas del repositorio");
        assert_eq!(actual[0].state, AlertState::Open);
        assert_eq!(actual[0].resolved_at, None);
    }

    /// Sin ninguna ejecución almacenada no hay diff que mostrar.
    #[tokio::test]
    async fn sin_ejecuciones_no_hay_diff() {
        let store = store().await;

        assert!(
            store
                .latest_diff()
                .await
                .expect("consulta de diff")
                .is_none(),
            "sin ejecuciones no hay diff"
        );
        assert!(
            store.history().await.expect("serie histórica").is_empty(),
            "sin ejecuciones la serie está vacía"
        );
    }

    /// Una ejecución sin cambios respecto a la anterior no aporta nuevas ni
    /// resueltas.
    #[tokio::test]
    async fn una_ejecucion_sin_cambios_no_aporta_nuevas_ni_resueltas() {
        let store = store().await;

        store
            .persist_scan(&escaneo(&["1", "2"]), instante(1), instante(2))
            .await
            .expect("run 1");
        store
            .persist_scan(&escaneo(&["1", "2"]), instante(3), instante(4))
            .await
            .expect("run 2 idéntico");

        let serie = store.history().await.expect("serie histórica");
        assert_eq!(serie.len(), 2);
        assert_eq!(serie[1].new, 0, "ninguna nueva");
        assert_eq!(serie[1].resolved, 0, "ninguna resuelta");
        assert_eq!(serie[1].total, 2, "las dos siguen presentes");

        let ultimo: Diff = store
            .latest_diff()
            .await
            .expect("diff del último run")
            .expect("hay ejecuciones");
        assert!(ultimo.new.is_empty(), "el diff no lista nuevas");
        assert!(ultimo.resolved.is_empty(), "el diff no lista resueltas");

        let cambios: Vec<Drift> = store
            .events_for_run(serie[1].run_id)
            .await
            .expect("eventos del run 2")
            .into_iter()
            .map(|evento| evento.change)
            .collect();
        assert!(
            cambios.iter().all(|cambio| *cambio == Drift::StillOpen),
            "un run idéntico solo registra still_open; encontrados: {cambios:?}"
        );
    }

    /// Identificadores externos de una lista de alertas almacenadas.
    fn ids(alertas: &[ghwatch::store::StoredAlert]) -> Vec<&str> {
        alertas
            .iter()
            .map(|alerta| alerta.external_id.as_str())
            .collect()
    }
}
