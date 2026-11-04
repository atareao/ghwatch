//! Tests de integración de los informes (consola, Markdown, JSON y escritura).
//!
//! Los tests viven en el módulo `report` para que el filtro `cargo test report`
//! los seleccione por nombre. Cubren los escenarios de la spec `reporting`:
//! consola agrupada por severidad y ordenada por prioridad, Markdown de
//! situación actual (con exactamente 15 repositorios) y de evolución (con
//! tendencia gráfica y acciones por paquete), JSON versionado y escritura en
//! disco. La coordinación informe/notificación se prueba en `tests/notify.rs`.

mod report {
    use chrono::{DateTime, TimeZone, Utc};
    use comfy_table::Color;
    use ghwatch::domain::alert::{Alert, AlertSource, Severity};
    use ghwatch::domain::repo::{DependabotState, Repo};
    use ghwatch::github::GithubError;
    use ghwatch::github::code_scanning::CodeScanningReport;
    use ghwatch::github::secret_scanning::SecretScanningReport;
    use ghwatch::report::{
        self, HISTORY_EN_INFORME, ReportDocument, SeverityCounts, render_console, severity_color,
    };
    use ghwatch::scan::{RepoScan, ScanFailure, ScanSummary};
    use ghwatch::store::HistoryPoint;

    /// Instante determinista a partir de un segundo Unix.
    fn instante(segundos: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(segundos, 0)
            .single()
            .expect("instante válido")
    }

    /// Alerta de Dependabot mínima con el identificador y la severidad dados.
    fn alerta(id: &str, severidad: Severity) -> Alert {
        Alert {
            source: AlertSource::Dependabot,
            external_id: id.to_string(),
            ghsa_id: None,
            cve_id: None,
            severity: Some(severidad),
            cvss: None,
            summary: Some(format!("alerta {id}")),
            package: None,
            ecosystem: None,
            manifest_path: None,
            scope: None,
            vulnerable_range: None,
            first_patched: None,
            secret_type: None,
            created_at: None,
        }
    }

    /// Recolección de un repositorio de Dependabot con las alertas indicadas.
    fn repo_scan(nombre: &str, alertas: Vec<Alert>) -> RepoScan {
        RepoScan {
            repo: Repo {
                name_with_owner: nombre.to_string(),
                private: false,
                archived: false,
                fork: false,
                language: None,
                pushed_at: None,
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

    // --- 6.1 Consola agrupada por severidad y ordenada por prioridad ---

    /// La consola agrupa por severidad (críticas → altas → medias) y, dentro de
    /// cada grupo, ordena por prioridad descendente. Además aplica un color
    /// distinto por severidad, visible incluso sin TTY.
    #[test]
    fn la_consola_agrupa_por_severidad_y_ordena_por_prioridad() {
        let now = instante(2_000_000_000);

        // Dos críticas con distinto CVSS: la de mayor CVSS debe ir primero.
        let mut critica_alta = alerta("1", Severity::Critical);
        critica_alta.cvss = Some(9.8);
        critica_alta.summary = Some("ALFA".to_string());
        let mut critica_baja = alerta("2", Severity::Critical);
        critica_baja.cvss = Some(3.0);
        critica_baja.summary = Some("BETA".to_string());

        let mut alta = alerta("3", Severity::High);
        alta.summary = Some("GAMMA".to_string());
        let mut media = alerta("4", Severity::Medium);
        media.summary = Some("DELTA".to_string());

        let alertas = vec![media.clone(), alta.clone(), critica_baja, critica_alta];
        let salida = render_console(&alertas, now);

        // La salida contiene códigos ANSI de color aunque no haya TTY.
        assert!(
            salida.contains('\u{1b}'),
            "la salida debería llevar color ANSI: {salida}"
        );

        // Colores distintos por severidad (crossterm: rojo, rojo oscuro, amarillo).
        assert!(
            salida.contains("\u{1b}[38;5;9m"),
            "falta el color de crítica"
        );
        assert!(salida.contains("\u{1b}[38;5;1m"), "falta el color de alta");
        assert!(
            salida.contains("\u{1b}[38;5;11m"),
            "falta el color de media"
        );

        assert_ne!(
            severity_color(Severity::Critical),
            severity_color(Severity::High)
        );
        assert_ne!(
            severity_color(Severity::High),
            severity_color(Severity::Medium)
        );
        assert_ne!(
            severity_color(Severity::Medium),
            severity_color(Severity::Low)
        );

        // Orden: ALFA antes que BETA (prioridad dentro de las críticas) y ambas
        // antes que GAMMA (grupo de altas) y DELTA (grupo de medias).
        let pos = |marca: &str| {
            salida
                .find(marca)
                .unwrap_or_else(|| panic!("no aparece {marca} en la salida:\n{salida}"))
        };
        assert!(
            pos("ALFA") < pos("BETA"),
            "prioridad dentro de las críticas"
        );
        assert!(
            pos("BETA") < pos("GAMMA"),
            "las críticas van antes que las altas"
        );
        assert!(
            pos("GAMMA") < pos("DELTA"),
            "las altas van antes que las medias"
        );
    }

    // --- 6.2 Markdown de situación actual ---

    /// El informe Markdown muestra totales, desglose por severidad y ecosistema,
    /// e incluye exactamente 15 repositorios en la tabla de cabecera, con los
    /// datos del escaneo de referencia (≈1.333 alertas en 90 repos de 367).
    #[test]
    fn el_markdown_de_situacion_muestra_totales_y_exactamente_15_repos() {
        let mut alertas = Vec::new();
        for i in 0..1333 {
            let severidad = if i < 3 {
                Severity::Critical
            } else if i < 43 {
                Severity::High
            } else {
                Severity::Medium
            };
            let mut alerta = alerta(&format!("d{i}"), severidad);
            alerta.ecosystem = Some("pip".to_string());
            alertas.push(alerta);
        }

        // 90 repositorios con alertas (15 cada uno para los 73 primeros, 14 para
        // los 17 últimos) y el resto vacíos hasta los 367 escaneados.
        let mut scanned = Vec::new();
        let mut cursor = 0;
        for r in 0..90 {
            let n = if r < 73 { 15 } else { 14 };
            let trozo = alertas[cursor..cursor + n].to_vec();
            cursor += n;
            scanned.push(repo_scan(&format!("atareao/r{:03}", r + 1), trozo));
        }
        assert_eq!(cursor, 1333, "se reparten las 1333 alertas");
        for r in 90..367 {
            scanned.push(repo_scan(&format!("atareao/r{:03}", r + 1), Vec::new()));
        }

        let resumen = ScanSummary {
            scanned,
            failed: vec![
                ScanFailure {
                    repo: "atareao/roto-1".to_string(),
                    error: GithubError::NotFound,
                },
                ScanFailure {
                    repo: "atareao/roto-2".to_string(),
                    error: GithubError::NotFound,
                },
            ],
        };

        let documento = ReportDocument::from_scan(
            &resumen,
            &[],
            HISTORY_EN_INFORME,
            1,
            0,
            0,
            instante(1_700_000_000),
        );
        let md = documento.to_markdown();

        assert!(md.contains("# Informe"), "cabecera del informe: {md}");
        assert!(md.contains("367"), "el total de repositorios escaneados");
        assert!(md.contains("1333"), "el total de alertas");
        assert!(md.contains("Críticas: 3"), "desglose de críticas");
        assert!(md.contains("Altas: 40"), "desglose de altas");
        assert!(md.contains("Medias: 1290"), "desglose de medias");
        assert!(md.contains("| pip |"), "desglose por ecosistema");

        // Exactamente 15 filas de repositorio, las de mayor número de alertas.
        let filas: Vec<&str> = md
            .lines()
            .filter(|linea| linea.starts_with("| atareao/"))
            .collect();
        assert_eq!(
            filas.len(),
            15,
            "la tabla de cabecera tiene 15 repos: {filas:?}"
        );
        assert_eq!(
            filas[0], "| atareao/r001 | 15 |",
            "el peor repositorio va primero"
        );
        assert_eq!(
            filas[14], "| atareao/r015 | 15 |",
            "el decimoquinto cierra la tabla"
        );
    }

    // --- 6.3 Markdown de evolución y acciones ---

    /// El informe Markdown incluye una fila por ejecución (fecha, nuevas,
    /// resueltas y neto), una tendencia gráfica determinista y un bloque de
    /// acciones por paquete con su versión que corrige, ordenado por número de
    /// repositorios afectados descendente.
    #[test]
    fn el_markdown_de_evolucion_incluye_tendencia_y_acciones() {
        let historia = vec![
            HistoryPoint {
                run_id: 1,
                started_at: instante(1_700_000_000),
                total: 10,
                new: 10,
                resolved: 0,
            },
            HistoryPoint {
                run_id: 2,
                started_at: instante(1_700_086_400),
                total: 12,
                new: 4,
                resolved: 2,
            },
            HistoryPoint {
                run_id: 3,
                started_at: instante(1_700_172_800),
                total: 9,
                new: 1,
                resolved: 4,
            },
            HistoryPoint {
                run_id: 4,
                started_at: instante(1_700_259_200),
                total: 15,
                new: 8,
                resolved: 2,
            },
            HistoryPoint {
                run_id: 5,
                started_at: instante(1_700_345_600),
                total: 8,
                new: 0,
                resolved: 7,
            },
        ];

        // Acciones: django en 3 repos, requests en 2 y flask en 1.
        let mut alertas = Vec::new();
        let mut repos = Vec::new();
        for (repos_afectados, paquete, version, marca) in [
            (3usize, "django", "3.0.2", "d"),
            (2usize, "requests", "2.31.0", "r"),
            (1usize, "flask", "2.3.0", "f"),
        ] {
            for r in 0..repos_afectados {
                let mut alerta = alerta(&format!("{marca}{r}"), Severity::High);
                alerta.package = Some(paquete.to_string());
                alerta.first_patched = Some(version.to_string());
                alertas.push(alerta.clone());
                repos.push(repo_scan(&format!("atareao/{marca}-{r}"), vec![alerta]));
            }
        }
        let resumen = ScanSummary {
            scanned: repos,
            failed: Vec::new(),
        };

        let documento = ReportDocument::from_scan(
            &resumen,
            &historia,
            HISTORY_EN_INFORME,
            5,
            0,
            7,
            instante(1_700_345_600),
        );
        let md = documento.to_markdown();

        // Una fila por ejecución, con neto = nuevas - resueltas.
        assert!(md.contains("2023-11-14"), "fecha de la primera ejecución");
        assert!(
            md.contains("| 4 | 2 | 2 |"),
            "neto de la segunda ejecución (4-2)"
        );
        assert!(
            md.contains("| 0 | 7 | -7 |"),
            "neto de la última ejecución (0-7)"
        );

        // La tendencia gráfica usa exactamente cinco bloques (uno por ejecución).
        let bloques = md
            .chars()
            .filter(|c| ('\u{2581}'..='\u{2588}').contains(c))
            .count();
        assert_eq!(bloques, 5, "una barra de tendencia por ejecución: {md}");

        // Acciones ordenadas por repositorios afectados descendente.
        let pos = |paquete: &str| {
            md.find(paquete)
                .unwrap_or_else(|| panic!("no aparece la acción {paquete}:\n{md}"))
        };
        assert!(pos("django") < pos("requests"), "3 repos antes que 2");
        assert!(pos("requests") < pos("flask"), "2 repos antes que 1");
        assert!(md.contains("3.0.2"), "la versión que corrige django");

        let django = documento
            .actions
            .iter()
            .find(|accion| accion.package == "django")
            .expect("acción de django");
        assert_eq!(django.repos, 3);
        assert_eq!(django.fixed_version.as_deref(), Some("3.0.2"));
    }

    /// M7 — `from_scan` acota la historia a las **últimas** N ejecuciones, en
    /// orden cronológico.
    #[test]
    fn el_informe_acota_la_historia_a_las_ultimas_ejecuciones() {
        let historia: Vec<HistoryPoint> = (1..=5)
            .map(|indice| HistoryPoint {
                run_id: indice,
                started_at: instante(1_700_000_000 + indice * 86_400),
                total: indice,
                new: 1,
                resolved: 0,
            })
            .collect();

        let resumen = ScanSummary {
            scanned: Vec::new(),
            failed: Vec::new(),
        };
        let documento =
            ReportDocument::from_scan(&resumen, &historia, 2, 5, 0, 0, instante(1_700_345_600));

        let runs: Vec<i64> = documento
            .history
            .iter()
            .map(|entrada| entrada.run_id)
            .collect();
        assert_eq!(runs, vec![4, 5], "se conservan las dos últimas, en orden");

        let bloques = documento
            .to_markdown()
            .chars()
            .filter(|caracter| ('\u{2581}'..='\u{2588}').contains(caracter))
            .count();
        assert_eq!(bloques, 2, "una barra por ejecución conservada");
    }

    /// M7 — una serie con una sola ejecución se representa igualmente (una fila
    /// y un bloque) sin romper el cálculo de la tendencia.
    #[test]
    fn la_evolucion_admite_una_sola_ejecucion() {
        let historia = vec![HistoryPoint {
            run_id: 1,
            started_at: instante(1_700_000_000),
            total: 10,
            new: 10,
            resolved: 0,
        }];
        let resumen = ScanSummary {
            scanned: Vec::new(),
            failed: Vec::new(),
        };
        let documento = ReportDocument::from_scan(
            &resumen,
            &historia,
            HISTORY_EN_INFORME,
            1,
            10,
            0,
            instante(1_700_000_000),
        );

        assert_eq!(documento.history.len(), 1);
        assert_eq!(documento.history[0].net, 10);

        let bloques = documento
            .to_markdown()
            .chars()
            .filter(|caracter| ('\u{2581}'..='\u{2588}').contains(caracter))
            .count();
        assert_eq!(bloques, 1, "una sola ejecución produce un bloque");
    }

    /// M7 — si hay menos repositorios con alertas que el máximo de la tabla, se
    /// listan solo los disponibles (la tabla muestra «hasta 15»).
    #[test]
    fn el_markdown_lista_solo_los_repositorios_disponibles() {
        let resumen = ScanSummary {
            scanned: vec![
                repo_scan("atareao/uno", vec![alerta("1", Severity::High)]),
                repo_scan("atareao/dos", vec![alerta("2", Severity::High)]),
                repo_scan("atareao/tres", vec![alerta("3", Severity::Medium)]),
                repo_scan("atareao/vacio", Vec::new()),
            ],
            failed: Vec::new(),
        };
        let documento = ReportDocument::from_scan(
            &resumen,
            &[],
            HISTORY_EN_INFORME,
            1,
            0,
            0,
            instante(1_700_000_000),
        );
        let md = documento.to_markdown();

        let filas: Vec<&str> = md
            .lines()
            .filter(|linea| linea.starts_with("| atareao/"))
            .collect();
        assert_eq!(filas.len(), 3, "solo los tres repos con alertas: {filas:?}");
    }

    // --- 6.4 JSON estable y versionado ---

    /// El documento JSON declara `schema_version` y usa claves estables.
    #[test]
    fn el_json_declara_schema_version_y_claves_estables() {
        let mut alerta = alerta("1", Severity::Critical);
        alerta.package = Some("django".to_string());
        alerta.first_patched = Some("3.0.2".to_string());
        alerta.ecosystem = Some("pip".to_string());
        let resumen = ScanSummary {
            scanned: vec![repo_scan("atareao/uno", vec![alerta])],
            failed: Vec::new(),
        };

        let documento = ReportDocument::from_scan(
            &resumen,
            &[],
            HISTORY_EN_INFORME,
            7,
            1,
            0,
            instante(1_700_000_000),
        );
        let json = documento.to_json().expect("serialización del informe");
        let valor: serde_json::Value = serde_json::from_str(&json).expect("JSON válido");

        assert_eq!(valor["schema_version"], serde_json::json!(1));
        for clave in [
            "schema_version",
            "generated_at",
            "run_id",
            "scanned",
            "failed",
            "total",
            "by_severity",
            "by_ecosystem",
            "worst_repos",
            "new",
            "resolved",
            "actions",
            "history",
        ] {
            assert!(valor.get(clave).is_some(), "falta la clave estable {clave}");
        }
        assert_eq!(valor["total"], serde_json::json!(1));
        assert_eq!(valor["by_severity"]["critical"], serde_json::json!(1));
    }

    /// `SeverityCounts` agrega por severidad incluyendo las desconocidas.
    #[test]
    fn severity_counts_agrega_por_severidad() {
        let alertas = vec![
            alerta("1", Severity::Critical),
            alerta("2", Severity::High),
            alerta("3", Severity::High),
            alerta("4", Severity::Medium),
            alerta("5", Severity::Low),
            {
                let mut a = alerta("6", Severity::Low);
                a.severity = None;
                a
            },
        ];
        let conteo = SeverityCounts::from_alerts(&alertas);
        assert_eq!(conteo.critical, 1);
        assert_eq!(conteo.high, 2);
        assert_eq!(conteo.medium, 1);
        assert_eq!(conteo.low, 1);
        assert_eq!(conteo.unknown, 1);
        assert_eq!(conteo.total(), 6);
    }

    /// El desglose por ecosistema no descarta ninguna alerta: las que no tienen
    /// ecosistema se agrupan bajo «desconocido» (M2), de modo que la suma del
    /// desglose coincide con el total de alertas.
    #[test]
    fn el_desglose_por_ecosistema_agrupa_las_alertas_sin_ecosistema() {
        let mut con_eco = alerta("1", Severity::High);
        con_eco.ecosystem = Some("pip".to_string());
        let sin_eco = alerta("2", Severity::Medium);

        let resumen = ScanSummary {
            scanned: vec![repo_scan("atareao/uno", vec![con_eco, sin_eco])],
            failed: Vec::new(),
        };
        let documento = ReportDocument::from_scan(
            &resumen,
            &[],
            HISTORY_EN_INFORME,
            1,
            0,
            0,
            instante(1_700_000_000),
        );

        let suma: usize = documento.by_ecosystem.iter().map(|eco| eco.alerts).sum();
        assert_eq!(
            suma, documento.total,
            "ninguna alerta se pierde en el desglose: {:?}",
            documento.by_ecosystem
        );
        assert!(
            documento
                .by_ecosystem
                .iter()
                .any(|eco| eco.ecosystem == "desconocido"),
            "las alertas sin ecosistema van a «desconocido»: {:?}",
            documento.by_ecosystem
        );
    }

    // --- 6.5 Escritura en disco ---

    /// `write_reports` escribe Markdown y JSON y devuelve sus rutas.
    #[test]
    fn write_reports_escribe_markdown_y_json_y_devuelve_las_rutas() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let md = dir.path().join("report.md");
        let json = dir.path().join("report.json");

        let resumen = ScanSummary {
            scanned: vec![repo_scan("atareao/uno", vec![alerta("1", Severity::High)])],
            failed: Vec::new(),
        };
        let documento = ReportDocument::from_scan(
            &resumen,
            &[],
            HISTORY_EN_INFORME,
            1,
            1,
            0,
            instante(1_700_000_000),
        );

        let rutas = report::write_reports(&documento, &md, &json).expect("escritura de informes");
        assert_eq!(rutas.markdown, md);
        assert_eq!(rutas.json, json);
        assert!(rutas.markdown.exists(), "el Markdown queda escrito");
        assert!(rutas.json.exists(), "el JSON queda escrito");
    }

    /// El color de `Severity::Unknown` (sin severidad) es distinto del de las
    /// severidades conocidas.
    #[test]
    fn el_color_de_unknown_es_distinto() {
        assert_ne!(
            severity_color(Severity::Unknown),
            severity_color(Severity::Critical)
        );
        assert_ne!(
            severity_color(Severity::Unknown),
            severity_color(Severity::Low)
        );
        assert_eq!(severity_color(Severity::Low), Color::Green);
    }
}
