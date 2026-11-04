//! Tests de integración de la notificación a Valet.
//!
//! Los tests viven en el módulo `notify` para que el filtro `cargo test notify`
//! los seleccione por nombre. Cubren los escenarios de la spec `valet-notify`:
//! envío con `POST /api/notifications` y token de ingesta, recorte del `message`
//! por bytes UTF-8, reducción del metadato, derivación del nivel, modo silencioso
//! con `--force`, permisos de configuración y registros sin token.
//!
//! No hay red real: Valet se simula con `wiremock`.

mod notify {
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Arc, Mutex, OnceLock};

    use chrono::{DateTime, TimeZone, Utc};
    use ghwatch::notify::{
        DEFAULT_BASE_URL, ENV_BASE_URL, ENV_INGEST_TOKEN, IngestToken, MESSAGE_LIMIT,
        METADATA_LIMIT, NotifyError, NotifyLevel, NotifyOutcome, ValetClient, ValetConfig,
        ValetSettings, build_notification, default_config_path, emit_and_notify, level_for,
        should_notify, truncate_utf8,
    };
    use ghwatch::report::{EcosystemCount, RepoCount, ReportDocument, SeverityCounts};
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Token de ingesta usado en los tests (falso: nunca sale del proceso).
    const TOKEN: &str = "val_token_de_prueba_para_los_tests";

    /// Serializa los dos tests que alcanzan el `warn!` de `notify::emit::enviar`
    /// (el de captura de log y el de Valet caído).
    ///
    /// `tracing_core` cachea **globalmente** el *interest* de cada callsite: la
    /// primera evaluación del `warn!` la hace el test que llegue antes y, si lo
    /// hace sin suscriptor instalado, deja el evento cacheado como inerte para
    /// todos. Sin serializar, ambos tests pueden registrar el callsite a la vez y
    /// la reconstrucción de la caché del test de log pierde la carrera. Tomar
    /// este cerrojo impide esa concurrencia; el `rebuild_interest_cache` de
    /// después re-evalúa el interés contra el suscriptor ya instalado.
    ///
    /// Es un `Mutex` asíncrono porque el cerrojo se mantiene a lo largo de los
    /// `await` de cada test (un `std::sync::Mutex` dispararía
    /// `clippy::await_holding_lock`).
    static WARN_CALLSITE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

    /// Cerrojo compartido por los tests que alcanzan el `warn!` de `emit::enviar`.
    fn warn_callsite() -> &'static tokio::sync::Mutex<()> {
        WARN_CALLSITE.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    /// Instante determinista a partir de un segundo Unix.
    fn instante(segundos: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(segundos, 0)
            .single()
            .expect("instante válido")
    }

    /// Cliente Valet apuntando al servidor falso.
    fn cliente_valet(server: &MockServer) -> ValetClient {
        ValetClient::new(
            ValetConfig {
                base_url: server.uri(),
                ..ValetConfig::default()
            },
            IngestToken::new(TOKEN).expect("token válido"),
        )
        .expect("cliente Valet")
    }

    /// Documento de informe mínimo para las pruebas de notificación.
    fn documento(run_id: i64, new: i64, resolved: i64) -> ReportDocument {
        ReportDocument {
            schema_version: 1,
            generated_at: instante(1_700_000_000),
            run_id,
            scanned: 367,
            failed: 0,
            total: 1333,
            by_severity: SeverityCounts {
                critical: 3,
                high: 40,
                medium: 1290,
                low: 0,
                unknown: 0,
            },
            by_ecosystem: vec![EcosystemCount {
                ecosystem: "pip".to_string(),
                alerts: 1333,
            }],
            worst_repos: vec![RepoCount {
                repo: "atareao/uno".to_string(),
                alerts: 15,
            }],
            new,
            resolved,
            actions: Vec::new(),
            history: Vec::new(),
        }
    }

    // --- 6.6 Envío a la API de ingesta ---

    /// Un envío con token válido usa `POST /api/notifications`, con `Bearer` y
    /// `application/json`, y devuelve el identificador del `201`.
    #[tokio::test]
    async fn el_envio_con_token_valido_devuelve_el_identificador() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .and(header("content-type", "application/json"))
            .and(body_partial_json(json!({
                "level": "critical",
                "metadata": {"schema_version": 1, "run_id": 7}
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": "3f2a1c7e-9b4d-4e6a-8f0c-1d2e3f4a5b6c"
            })))
            .mount(&server)
            .await;

        let cliente = cliente_valet(&server);
        let notificacion = build_notification(&documento(7, 1, 0), Path::new("/tmp/informe.md"));

        let id = cliente
            .send(&notificacion)
            .await
            .expect("el envío debe completarse");
        assert_eq!(id, "3f2a1c7e-9b4d-4e6a-8f0c-1d2e3f4a5b6c");

        let peticiones = server
            .received_requests()
            .await
            .expect("wiremock registra las peticiones");
        assert_eq!(peticiones.len(), 1, "una sola petición");
    }

    /// Un `401` (token revocado) aborta el envío sin reintentar.
    #[tokio::test]
    async fn un_401_aborta_sin_reintentar() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "unauthorized"
            })))
            .mount(&server)
            .await;

        let cliente = cliente_valet(&server);
        let notificacion = build_notification(&documento(7, 1, 0), Path::new("/tmp/informe.md"));

        let error = cliente
            .send(&notificacion)
            .await
            .expect_err("un 401 no es un envío válido");
        assert!(matches!(error, NotifyError::Unauthorized));

        let peticiones = server
            .received_requests()
            .await
            .expect("wiremock registra las peticiones");
        assert_eq!(peticiones.len(), 1, "no hay reintentos tras el 401");
    }

    // --- 6.7 Límites exactos ---

    /// `truncate_utf8` recorta por frontera de carácter, sin partir multibyte.
    #[test]
    fn truncate_utf8_no_parte_caracteres_multibyte() {
        let texto = "aaaaaaaéééé";
        assert_eq!(truncate_utf8(texto, 7), "aaaaaaa");
        assert_eq!(truncate_utf8(texto, 8), "aaaaaaa");
        assert_eq!(truncate_utf8(texto, 9), "aaaaaaaé");
        assert_eq!(truncate_utf8(texto, 10), "aaaaaaaé");

        let emoji = "🦀🦀";
        assert_eq!(truncate_utf8(emoji, 3), "");
        assert_eq!(truncate_utf8(emoji, 4), "🦀");
        assert_eq!(truncate_utf8(emoji, 7), "🦀");
        assert_eq!(truncate_utf8(emoji, 8), "🦀🦀");

        assert_eq!(truncate_utf8("abc", 10), "abc");
        assert_eq!(truncate_utf8("abc", 0), "");
    }

    /// Un resumen que excede el límite se recorta a ≤4096 bytes, sigue siendo
    /// UTF-8 válido y conserva la ruta del informe completo.
    #[test]
    fn el_mensaje_respeta_el_limite_y_lleva_la_ruta() {
        let mut doc = documento(7, 1, 0);
        doc.worst_repos = (0..15)
            .map(|indice| RepoCount {
                repo: format!("atareao/{}", "x".repeat(400)),
                alerts: indice + 1,
            })
            .collect();

        let ruta = "/tmp/informes/ghwatch-1700000000.md";
        let notificacion = build_notification(&doc, Path::new(ruta));

        assert!(
            notificacion.message.len() <= MESSAGE_LIMIT,
            "el mensaje mide {} bytes",
            notificacion.message.len()
        );
        assert!(
            notificacion.message.contains(ruta),
            "el mensaje lleva la ruta del informe: {}",
            notificacion.message
        );
        assert!(
            !notificacion.message.contains('\u{FFFD}'),
            "el recorte no introduce caracteres de reemplazo"
        );
        assert!(!notificacion.message.trim().is_empty());
    }

    /// I2 — con un cuerpo ASCII saturado, el mensaje ocupa **exactamente**
    /// [`MESSAGE_LIMIT`] bytes (no uno menos): el recorte deja al cuerpo todo el
    /// presupuesto que no ocupa la línea de la ruta.
    #[test]
    fn el_mensaje_ascii_saturado_alcanza_el_limite_exacto() {
        let mut doc = documento(7, 1, 0);
        doc.worst_repos = (0..15)
            .map(|indice| RepoCount {
                repo: format!("atareao/{}", "x".repeat(400)),
                alerts: indice + 1,
            })
            .collect();

        let notificacion = build_notification(&doc, Path::new("/tmp/informe.md"));
        assert_eq!(
            notificacion.message.len(),
            MESSAGE_LIMIT,
            "un cuerpo saturado llena el mensaje hasta el límite exacto"
        );
        assert!(
            notificacion.message.contains("/tmp/informe.md"),
            "sigue llevando la ruta del informe"
        );
    }

    /// I2 — una ruta de informe sintética mayor que [`METADATA_LIMIT`] alcanza la
    /// reducción extrema: se recorta `report_path` y el metadato sigue cabiendo
    /// en el límite, conservando todas sus claves.
    #[test]
    fn el_metadato_con_ruta_patologica_se_recorta_y_conserva_las_claves() {
        let ruta_larga = format!("/tmp/{}", "a".repeat(METADATA_LIMIT + 200));
        let doc = documento(7, 1, 0);

        let notificacion = build_notification(&doc, Path::new(&ruta_larga));
        let serializado =
            serde_json::to_vec(&notificacion.metadata).expect("el metadato es serializable");
        assert!(
            serializado.len() <= METADATA_LIMIT,
            "el metadato mide {} bytes",
            serializado.len()
        );
        for clave in [
            "schema_version",
            "run_id",
            "scanned",
            "by_severity",
            "new",
            "resolved",
            "worst_repos",
            "report_path",
        ] {
            assert!(
                notificacion.metadata.get(clave).is_some(),
                "falta la clave obligatoria {clave}"
            );
        }
        let ruta = notificacion.metadata["report_path"]
            .as_str()
            .expect("report_path es una cadena");
        assert!(
            ruta.len() <= 128,
            "la ruta se recorta a lo sumo a 128 bytes: {}",
            ruta.len()
        );
    }

    /// I2 — `truncate_utf8` respeta la frontera de un carácter de **3 bytes**
    /// (los de 2 y 4 ya están cubiertos), incluida la frontera exacta.
    #[test]
    fn truncate_utf8_respeta_los_caracteres_de_tres_bytes() {
        // '€' ocupa 3 bytes (E2 82 AC).
        let texto = "€€€";
        assert_eq!(texto.len(), 9, "tres caracteres de tres bytes cada uno");
        assert_eq!(truncate_utf8(texto, 9), "€€€");
        assert_eq!(truncate_utf8(texto, 8), "€€");
        assert_eq!(truncate_utf8(texto, 6), "€€");
        assert_eq!(truncate_utf8(texto, 5), "€");
        assert_eq!(truncate_utf8(texto, 3), "€");
        assert_eq!(truncate_utf8(texto, 2), "");
    }

    /// Un metadato que excedería el límite se reduce hasta caber, sin perder sus
    /// claves obligatorias.
    #[test]
    fn el_metadato_se_reduce_por_debajo_del_limite() {
        let mut doc = documento(7, 1, 0);
        doc.worst_repos = (0..15)
            .map(|indice| RepoCount {
                repo: format!("atareao/{}", "y".repeat(2000)),
                alerts: indice + 1,
            })
            .collect();

        let notificacion = build_notification(&doc, Path::new("/tmp/informe.md"));
        let serializado =
            serde_json::to_vec(&notificacion.metadata).expect("el metadato es serializable");
        assert!(
            serializado.len() <= METADATA_LIMIT,
            "el metadato mide {} bytes",
            serializado.len()
        );
        assert!(notificacion.metadata.is_object(), "un objeto JSON");
        for clave in [
            "schema_version",
            "run_id",
            "scanned",
            "by_severity",
            "new",
            "resolved",
            "worst_repos",
            "report_path",
        ] {
            assert!(
                notificacion.metadata.get(clave).is_some(),
                "falta la clave obligatoria {clave}"
            );
        }
        assert_eq!(notificacion.metadata["schema_version"], json!(1));
    }

    /// `worst_repos` del metadato contiene como máximo los 5 peores repositorios.
    #[test]
    fn el_metadato_lleva_los_cinco_peores_repos() {
        let mut doc = documento(7, 1, 0);
        doc.worst_repos = (0..20)
            .map(|indice| RepoCount {
                repo: format!("atareao/r{indice:03}"),
                alerts: 100 - indice,
            })
            .collect();

        let notificacion = build_notification(&doc, Path::new("/tmp/informe.md"));
        let peores = notificacion.metadata["worst_repos"]
            .as_array()
            .expect("worst_repos es una lista");
        assert_eq!(peores.len(), 5, "cinco peores");
        assert_eq!(peores[0]["repo"], json!("atareao/r000"));
    }

    // --- 6.12 Estados del cliente y redirecciones ---

    /// M4 — un `400` (validación) y un `415` (tipo no soportado) se traducen a
    /// `Status` con su código, sin confundirse con el `401`.
    #[tokio::test]
    async fn los_errores_de_cliente_se_traducen_a_status() {
        for status in [400u16, 415] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/notifications"))
                .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                    "error": "petición inválida"
                })))
                .mount(&server)
                .await;

            let cliente = cliente_valet(&server);
            let notificacion =
                build_notification(&documento(7, 1, 0), Path::new("/tmp/informe.md"));

            let error = cliente
                .send(&notificacion)
                .await
                .expect_err("un estado no exitoso no es un envío válido");
            assert!(
                matches!(error, NotifyError::Status { status: s } if s == status),
                "se esperaba Status {status}, no: {error:?}"
            );
        }
    }

    /// M4 — una redirección no se sigue en silencio: se mapea a `Status` con el
    /// código `3xx`, coherente con cómo el cliente de GitHub trata los `3xx`.
    #[tokio::test]
    async fn una_redireccion_no_se_sigue_y_se_traduce_a_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "http://otro.example/ignorado"),
            )
            .mount(&server)
            .await;

        let cliente = cliente_valet(&server);
        let notificacion = build_notification(&documento(7, 1, 0), Path::new("/tmp/informe.md"));

        let error = cliente
            .send(&notificacion)
            .await
            .expect_err("un 3xx no es un envío válido");
        assert!(
            matches!(error, NotifyError::Status { status: 302 }),
            "se esperaba Status 302, no: {error:?}"
        );
    }

    /// M4 — un `2xx` con un cuerpo que no es el JSON esperado falla al
    /// decodificar y se traduce a un error de petición.
    #[tokio::test]
    async fn una_respuesta_de_exito_con_cuerpo_no_json_es_un_error_de_peticion() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .respond_with(ResponseTemplate::new(201).set_body_string("no soy json"))
            .mount(&server)
            .await;

        let cliente = cliente_valet(&server);
        let notificacion = build_notification(&documento(7, 1, 0), Path::new("/tmp/informe.md"));

        let error = cliente
            .send(&notificacion)
            .await
            .expect_err("un cuerpo no JSON no se decodifica");
        assert!(
            matches!(error, NotifyError::Request(_)),
            "se esperaba Request, no: {error:?}"
        );
    }

    // --- 6.12 Configuración: precedencia y permisos ---

    /// Escribe un `config.toml` en un directorio temporal con el modo indicado
    /// y devuelve su ruta. Solo en unix: los permisos solo existen ahí.
    #[cfg(unix)]
    fn escribir_config(
        directorio: &tempfile::TempDir,
        modo: u32,
        contenido: &str,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let ruta = directorio.path().join("config.toml");
        let mut fichero = std::fs::File::create(&ruta).expect("crear config.toml");
        fichero
            .write_all(contenido.as_bytes())
            .expect("escribir config.toml");
        std::fs::set_permissions(&ruta, std::fs::Permissions::from_mode(modo))
            .expect("fijar permisos del config.toml");
        ruta
    }

    /// I1 — el entorno tiene precedencia: cuando define una variable, el valor
    /// del fichero se ignora aunque el fichero esté bien formado y con `600`.
    #[cfg(unix)]
    #[test]
    fn el_entorno_tiene_precedencia_sobre_el_fichero() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(
            &directorio,
            0o600,
            "base_url = \"http://fichero:1234\"\ningest_token = \"val_del_fichero\"\n",
        );

        let settings = ValetSettings::from_env_or_file(
            |clave| match clave {
                ENV_BASE_URL => Some("http://entorno:9999".to_string()),
                ENV_INGEST_TOKEN => Some("val_del_entorno".to_string()),
                _ => None,
            },
            &ruta,
        );

        assert_eq!(settings.base_url, "http://entorno:9999");
        assert_eq!(
            settings.token.as_ref().map(IngestToken::expose_secret),
            Some("val_del_entorno"),
            "el token del fichero no puede pisar al del entorno"
        );
    }

    /// I1 — el fichero solo **completa** lo que el entorno no aporta: aquí la
    /// URL viene del entorno y el token del fichero.
    #[cfg(unix)]
    #[test]
    fn el_fichero_completa_lo_que_falta_del_entorno() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(
            &directorio,
            0o600,
            "base_url = \"http://fichero:1234\"\ningest_token = \"val_del_fichero\"\n",
        );

        let settings = ValetSettings::from_env_or_file(
            |clave| match clave {
                ENV_BASE_URL => Some("http://entorno:9999".to_string()),
                _ => None,
            },
            &ruta,
        );

        assert_eq!(settings.base_url, "http://entorno:9999");
        assert_eq!(
            settings.token.as_ref().map(IngestToken::expose_secret),
            Some("val_del_fichero"),
            "el token ausente del entorno se toma del fichero"
        );
    }

    /// I1 — un fichero ausente no es un error: se cae a los valores por defecto
    /// (y, si el entorno no dice nada, sin token).
    #[test]
    fn un_fichero_ausente_no_es_un_error() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = directorio.path().join("no-existe.toml");

        let settings = ValetSettings::from_env_or_file(|_| None, &ruta);

        assert_eq!(settings.base_url, DEFAULT_BASE_URL);
        assert!(settings.token.is_none());
    }

    /// I1 — un fichero con permisos más laxos que `600` se **rechaza**: ni su
    /// URL ni, sobre todo, su token se usan.
    #[cfg(unix)]
    #[test]
    fn un_fichero_con_permisos_laxos_se_rechaza() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(
            &directorio,
            0o644,
            "base_url = \"http://fichero:1234\"\ningest_token = \"val_del_fichero\"\n",
        );

        let settings = ValetSettings::from_env_or_file(|_| None, &ruta);

        assert_eq!(
            settings.base_url, DEFAULT_BASE_URL,
            "la URL de un fichero con permisos laxos no se usa"
        );
        assert!(
            settings.token.is_none(),
            "el token de un fichero con permisos laxos nunca se usa"
        );
    }

    /// I1 — un fichero con TOML inválido se ignora sin propagar error.
    #[cfg(unix)]
    #[test]
    fn un_fichero_con_toml_invalido_se_ignora() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(&directorio, 0o600, "esto no es = = toml válido\n");

        let settings = ValetSettings::from_env_or_file(|_| None, &ruta);

        assert_eq!(settings.base_url, DEFAULT_BASE_URL);
        assert!(settings.token.is_none());
    }

    /// I1 — la ruta por defecto cuelga de `$HOME/.config/ghwatch/config.toml`.
    #[test]
    fn la_ruta_por_defecto_cuelga_de_home() {
        if let Ok(home) = std::env::var("HOME") {
            let ruta = default_config_path().expect("HOME está definido");
            assert!(ruta.starts_with(&home), "{ruta:?} no cuelga de {home:?}");
            assert!(
                ruta.ends_with(".config/ghwatch/config.toml"),
                "{ruta:?} no acaba en el config.toml convencional"
            );
        }
    }

    // --- 6.8 Nivel y modo silencioso ---

    /// El nivel se deriva de la severidad: `critical` > `warning` > `info`.
    #[test]
    fn el_nivel_se_deriva_de_la_severidad() {
        let critico = SeverityCounts {
            critical: 1,
            ..Default::default()
        };
        let alto = SeverityCounts {
            high: 2,
            ..Default::default()
        };
        let medio = SeverityCounts {
            medium: 5,
            low: 1,
            ..Default::default()
        };

        assert_eq!(level_for(&critico), NotifyLevel::Critical);
        assert_eq!(level_for(&alto), NotifyLevel::Warning);
        assert_eq!(level_for(&medio), NotifyLevel::Info);
        assert_eq!(level_for(&SeverityCounts::default()), NotifyLevel::Info);

        assert_eq!(NotifyLevel::Critical.as_str(), "critical");
        assert_eq!(NotifyLevel::Warning.as_str(), "warning");
        assert_eq!(NotifyLevel::Info.as_str(), "info");

        let mut doc = documento(7, 0, 0);
        doc.by_severity = critico;
        assert_eq!(
            build_notification(&doc, Path::new("/tmp/informe.md")).level,
            NotifyLevel::Critical
        );
    }

    /// El modo silencioso por defecto solo envía si hay novedades; `force` lo
    /// salta.
    #[test]
    fn el_envio_es_silencioso_sin_novedades_salvo_force() {
        assert!(!should_notify(0, 0, false));
        assert!(should_notify(1, 0, false));
        assert!(should_notify(0, 1, false));
        assert!(should_notify(0, 0, true));
    }

    /// Sin novedades no se realiza ninguna petición a Valet.
    #[tokio::test]
    async fn sin_novedades_no_se_hace_ninguna_peticion() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "x"})))
            .mount(&server)
            .await;

        let cliente = cliente_valet(&server);
        let dir = tempfile::tempdir().expect("directorio temporal");
        let md = dir.path().join("report.md");
        let json_path = dir.path().join("report.json");

        let resultado =
            emit_and_notify(&documento(7, 0, 0), &md, &json_path, Some(&cliente), false)
                .await
                .expect("la escritura no falla");

        assert!(matches!(resultado.notify, NotifyOutcome::Skipped));
        let peticiones = server
            .received_requests()
            .await
            .expect("wiremock registra las peticiones");
        assert!(peticiones.is_empty(), "no se llama a Valet sin novedades");
    }

    /// Con `force` se envía aunque no haya novedades.
    #[tokio::test]
    async fn con_force_se_envia_aunque_no_haya_novedades() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "forzado"})))
            .mount(&server)
            .await;

        let cliente = cliente_valet(&server);
        let dir = tempfile::tempdir().expect("directorio temporal");
        let md = dir.path().join("report.md");
        let json_path = dir.path().join("report.json");

        let resultado = emit_and_notify(&documento(7, 0, 0), &md, &json_path, Some(&cliente), true)
            .await
            .expect("la escritura no falla");

        assert!(
            matches!(resultado.notify, NotifyOutcome::Sent(_)),
            "el envío forzado se completa: {:?}",
            resultado.notify
        );
        let peticiones = server
            .received_requests()
            .await
            .expect("wiremock registra las peticiones");
        assert_eq!(peticiones.len(), 1, "una petición forzada");
    }

    // --- 6.11 Escritura en disco e independencia de la notificación ---

    /// Con Valet caído (5xx) los informes quedan escritos y completos y la
    /// llamada no devuelve error: la notificación es best-effort.
    #[tokio::test]
    async fn con_valet_caido_los_informes_quedan_escritos() {
        // Comparte el callsite del `warn!` de `emit::enviar` con el test de
        // captura de log: se serializan para que ninguno registre el callsite
        // mientras el otro lo hace (ver `WARN_CALLSITE`).
        let _serializado = warn_callsite().lock().await;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let cliente = cliente_valet(&server);
        let dir = tempfile::tempdir().expect("directorio temporal");
        let md = dir.path().join("report.md");
        let json = dir.path().join("report.json");

        let resultado = emit_and_notify(&documento(1, 1, 0), &md, &json, Some(&cliente), true)
            .await
            .expect("los informes se escriben aunque Valet falle");

        assert!(
            resultado.paths.markdown.exists(),
            "el Markdown está escrito"
        );
        assert!(resultado.paths.json.exists(), "el JSON está escrito");
        assert!(
            matches!(resultado.notify, NotifyOutcome::Failed(_)),
            "el envío falla pero no aborta: {:?}",
            resultado.notify
        );

        let md_texto = std::fs::read_to_string(&md).expect("leer Markdown");
        assert!(md_texto.contains("# Informe"), "el Markdown está completo");
        let json_texto = std::fs::read_to_string(&json).expect("leer JSON");
        let valor: serde_json::Value = serde_json::from_str(&json_texto).expect("JSON válido");
        assert_eq!(valor["schema_version"], serde_json::json!(1));
        assert_eq!(valor["run_id"], serde_json::json!(1));
    }

    /// Sin notificador configurado los informes igualmente se escriben.
    #[tokio::test]
    async fn sin_notificador_los_informes_se_escriben() {
        let dir = tempfile::tempdir().expect("directorio temporal");
        let md = dir.path().join("report.md");
        let json = dir.path().join("report.json");

        let resultado = emit_and_notify(&documento(1, 0, 0), &md, &json, None, false)
            .await
            .expect("escritura");

        assert!(matches!(resultado.notify, NotifyOutcome::NotConfigured));
        assert!(md.exists(), "el Markdown está escrito");
        assert!(json.exists(), "el JSON está escrito");
    }

    // --- 6.9 Configuración y registros sin token ---

    /// Un fichero de configuración con permisos más laxos que `600` se rechaza.
    #[cfg(unix)]
    #[test]
    fn los_permisos_laxos_de_la_configuracion_se_rechazan() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("directorio temporal");
        let config = dir.path().join("config.toml");
        fs::write(&config, "VALET_BASE_URL = \"http://localhost:3000\"\n").expect("escritura");

        fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).expect("permisos laxos");
        let error = ghwatch::notify::check_permissions(&config).expect_err("644 se rechaza");
        match error {
            NotifyError::InsecurePermissions { mode, .. } => assert_eq!(mode, 0o644),
            otro => panic!("se esperaba InsecurePermissions, no: {otro:?}"),
        }

        fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
            .expect("permisos estrictos");
        ghwatch::notify::check_permissions(&config).expect("600 es aceptable");
    }

    /// Un envío fallido se registra sin incluir el valor del token.
    ///
    /// M6 — la garantía es **estructural**: [`IngestToken`] no implementa
    /// `Serialize` y ningún `NotifyError` transporta el valor, así que el token
    /// no puede llegar al log ni al JSON por construcción. Este test comprueba
    /// además que el mensaje registrado no lo contiene.
    #[tokio::test]
    async fn el_log_de_un_envio_fallido_no_contiene_el_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/notifications"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "unauthorized"
            })))
            .mount(&server)
            .await;

        let cliente = cliente_valet(&server);
        let dir = tempfile::tempdir().expect("directorio temporal");
        let md = dir.path().join("report.md");
        let json_path = dir.path().join("report.json");

        let captura = Captura::default();
        let _serializado = warn_callsite().lock().await;
        let _guarda = tracing::subscriber::set_default(suscriptor(captura.clone()));
        // `tracing_core` cachea **globalmente** el *interest* de cada callsite:
        // `set_default` instala un suscriptor thread-local, pero si otro test del
        // binario evalúa el interés del `warn!` de `emit::enviar` antes (o en
        // paralelo), el evento puede quedar cacheado como inactivo y no llegar a
        // este suscriptor. Reconstruir la caché tras instalar el suscriptor (y con
        // el cerrojo `WARN_CALLSITE` tomado, para que nadie registre el callsite a
        // la vez) garantiza que el evento se emita. No borrar.
        tracing::callsite::rebuild_interest_cache();

        let resultado = emit_and_notify(&documento(7, 1, 0), &md, &json_path, Some(&cliente), true)
            .await
            .expect("la escritura no falla");

        assert!(
            matches!(resultado.notify, NotifyOutcome::Failed(_)),
            "el envío falla sin abortar: {:?}",
            resultado.notify
        );

        let log = captura.texto();
        assert!(
            !log.contains(TOKEN),
            "el log no debe filtrar el token: {log}"
        );
        assert!(
            log.contains("401") || log.to_lowercase().contains("rechazad"),
            "el log explica el fallo: {log}"
        );
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

    /// Suscriptor que registra a nivel `WARN` en el sumidero dado.
    fn suscriptor(captura: Captura) -> impl tracing::Subscriber + Send + Sync {
        tracing_subscriber::fmt()
            .with_writer(captura)
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::WARN)
            .finish()
    }
}
