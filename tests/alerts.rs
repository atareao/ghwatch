//! Tests de integración de la recolección y normalización de alertas.
//!
//! Se usa `wiremock` como servidor falso: la suite no hace red real. Los tests
//! viven en el módulo `alerts` para que el filtro `cargo test alerts` los
//! seleccione por nombre.

mod alerts {
    use ghwatch::auth::Token;
    use ghwatch::domain::alert::{AlertSource, Severity};
    use ghwatch::domain::repo::DependabotState;
    use ghwatch::github::{
        GithubClient, GithubConfig, GithubError, code_scanning, dependabot, secret_scanning,
    };
    use serde_json::Value;
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Token reconocible: no debe filtrarse, pero aquí basta con que sea válido.
    const TOKEN: &str = "ghp_token_de_prueba";

    /// Construye un cliente apuntando al servidor falso.
    fn cliente(server: &MockServer) -> GithubClient {
        GithubClient::new(
            Token::new(TOKEN).expect("token válido"),
            GithubConfig {
                base_url: server.uri(),
                ..GithubConfig::default()
            },
        )
        .expect("el cliente debería construirse sin resolver credenciales")
    }

    /// Alerta de Dependabot completa, con todos los campos que aporta GitHub.
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
                "cve_id": "CVE-2026-0001",
                "summary": "Vulnerabilidad de prueba",
                "severity": "high",
                "cvss": { "score": 7.5, "vector_string": "CVSS:3.1/AV:N/AC:L" }
            },
            "security_vulnerability": {
                "package": { "ecosystem": "pip", "name": "django" },
                "severity": "high",
                "vulnerable_version_range": "< 3.0.1",
                "first_patched_version": { "identifier": "3.0.1" }
            }
        })
    }

    // --- 3.3 Normalización de Dependabot ---

    /// Un repositorio con 112 alertas repartidas en 2 páginas se recolecta
    /// entero (112, no 100) y se normaliza a modelo único con el paquete, el
    /// ecosistema, el manifiesto y el ámbito informados.
    #[tokio::test]
    async fn dependabot_normaliza_112_alertas_repartidas_en_dos_paginas() {
        let server = MockServer::start().await;
        let cursor = "Y3Vyc29yOnYyOpPPAAAB";
        let link = format!(
            "<{uri}/repos/atareao/reto-python/dependabot/alerts?state=open&per_page=100&after={cursor}>; rel=\"next\"",
            uri = server.uri()
        );

        let pagina1: Vec<Value> = (1..=100).map(alerta_dependabot).collect();
        let pagina2: Vec<Value> = (101..=112).map(alerta_dependabot).collect();

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/dependabot/alerts"))
            .and(query_param("state", "open"))
            .and(query_param_is_missing("after"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", link.as_str())
                    .set_body_json(pagina1),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/dependabot/alerts"))
            .and(query_param("after", cursor))
            .respond_with(ResponseTemplate::new(200).set_body_json(pagina2))
            .mount(&server)
            .await;

        let client = cliente(&server);
        let report = dependabot::collect(&client, "atareao/reto-python")
            .await
            .expect("la recolección debería completarse");

        assert_eq!(report.state, DependabotState::Enabled);
        assert_eq!(report.alerts.len(), 112, "no debe truncarse en 100");

        let primera = &report.alerts[0];
        assert_eq!(primera.source, AlertSource::Dependabot);
        assert_eq!(primera.external_id, "1");
        assert_eq!(primera.ghsa_id.as_deref(), Some("GHSA-aaaa-bbbb-cccc"));
        assert_eq!(primera.cve_id.as_deref(), Some("CVE-2026-0001"));
        assert_eq!(primera.severity, Some(Severity::High));
        assert_eq!(primera.cvss, Some(7.5));
        assert_eq!(primera.summary.as_deref(), Some("Vulnerabilidad de prueba"));
        assert_eq!(primera.package.as_deref(), Some("django"));
        assert_eq!(primera.ecosystem.as_deref(), Some("pip"));
        assert_eq!(primera.manifest_path.as_deref(), Some("requirements.txt"));
        assert_eq!(primera.scope.as_deref(), Some("runtime"));
        assert_eq!(primera.vulnerable_range.as_deref(), Some("< 3.0.1"));
        assert_eq!(primera.first_patched.as_deref(), Some("3.0.1"));
        assert!(primera.secret_type.is_none());
    }

    // --- 3.4 Normalización de secret scanning ---

    /// Una alerta de secret scanning se normaliza con `source = secret`,
    /// conserva su tipo de secreto y su `cvss` queda vacío **sin ser error**.
    #[tokio::test]
    async fn secret_scanning_normaliza_con_source_secret_y_cvss_vacio() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/secret-scanning/alerts"))
            .and(query_param("state", "open"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "number": 7,
                    "secret_type": "adafruit_io_key",
                    "secret_type_display_name": "Adafruit IO Key"
                }
            ])))
            .mount(&server)
            .await;

        let client = cliente(&server);
        let alertas = secret_scanning::collect(&client, "atareao/reto-python")
            .await
            .expect("la recolección debería completarse");

        assert_eq!(alertas.len(), 1);
        let alerta = &alertas[0];
        assert_eq!(alerta.source, AlertSource::Secret);
        assert_eq!(alerta.external_id, "7");
        assert_eq!(alerta.secret_type.as_deref(), Some("adafruit_io_key"));
        assert_eq!(alerta.summary.as_deref(), Some("Adafruit IO Key"));
        assert!(
            alerta.cvss.is_none(),
            "el cvss de un secreto debe quedar vacío sin ser un error"
        );
        assert!(alerta.ghsa_id.is_none());
        assert!(alerta.package.is_none());
    }

    // --- 3.5 Code scanning sin análisis configurado ---

    /// Un `404` `no analysis found` marca el repositorio como «no configurado» y
    /// no aborta.
    #[tokio::test]
    async fn code_scanning_sin_analisis_no_aborta() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/code-scanning/alerts"))
            .and(query_param("state", "open"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({ "message": "no analysis found" })),
            )
            .mount(&server)
            .await;

        let client = cliente(&server);
        let report = code_scanning::collect(&client, "atareao/reto-python")
            .await
            .expect("un 404 `no analysis found` no debe abortar");

        assert!(!report.configured, "debe marcarse como no configurado");
        assert!(report.alerts.is_empty());
    }

    // --- 3.6 Dependabot deshabilitado ---

    /// Un `403` de Dependabot deshabilitado marca `disabled` y no cuenta como
    /// fallo del escaneo.
    #[tokio::test]
    async fn dependabot_deshabilitado_marca_disabled_sin_fallar() {
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

        let client = cliente(&server);
        let report = dependabot::collect(&client, "atareao/gitignore")
            .await
            .expect("Dependabot deshabilitado no es un fallo");

        assert_eq!(report.state, DependabotState::Disabled);
        assert!(report.alerts.is_empty());
    }

    // --- 3.11 Endurecimiento tras la auditoría ---

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

    /// I1 — Un `404` genérico de secret scanning (repo público o capacidad no
    /// disponible) se tolera como lista vacía y **no** cuenta como fallo.
    #[tokio::test]
    async fn secret_scanning_404_se_tolera_como_lista_vacia() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/secret-scanning/alerts"))
            .and(query_param("state", "open"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({ "message": "Not Found" })),
            )
            .mount(&server)
            .await;

        let client = cliente(&server);
        let alertas = secret_scanning::collect(&client, "atareao/reto-python")
            .await
            .expect("un 404 no debe contabilizarse como fallo");

        assert!(alertas.is_empty(), "un 404 debe traducirse a lista vacía");
    }

    /// I2 — Un `403` con un cuerpo ajeno al literal de capacidad deshabilitada
    /// **no** se confunde con `Disabled`: el colector propaga `Http { 403 }`.
    #[tokio::test]
    async fn dependabot_403_generico_propaga_http_403() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/dependabot/alerts"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "message": "Resource not accessible"
            })))
            .mount(&server)
            .await;

        let client = cliente(&server);
        let error = dependabot::collect(&client, "atareao/reto-python")
            .await
            .expect_err("un 403 ajeno debe propagarse como error");

        match error {
            GithubError::Http { status, .. } => assert_eq!(status, 403),
            otro => panic!("se esperaba Http {{ status: 403 }}, no: {otro:?}"),
        }
    }

    /// M6 — Una colección de secretos de 2 páginas encadenadas por `Link` se
    /// recolecta entera (no se trunca en la primera página).
    #[tokio::test]
    async fn secret_scanning_recolecta_dos_paginas() {
        let server = MockServer::start().await;
        let cursor = "Y3Vyc29yOnYyOpPPAAAB";
        let link = format!(
            "<{uri}/repos/atareao/reto-python/secret-scanning/alerts?state=open&per_page=100&after={cursor}>; rel=\"next\"",
            uri = server.uri()
        );

        let pagina1: Vec<Value> = (1..=100).map(alerta_secreto).collect();
        let pagina2: Vec<Value> = (101..=101).map(alerta_secreto).collect();

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/secret-scanning/alerts"))
            .and(query_param("state", "open"))
            .and(query_param_is_missing("after"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", link.as_str())
                    .set_body_json(pagina1),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/secret-scanning/alerts"))
            .and(query_param("after", cursor))
            .respond_with(ResponseTemplate::new(200).set_body_json(pagina2))
            .mount(&server)
            .await;

        let client = cliente(&server);
        let alertas = secret_scanning::collect(&client, "atareao/reto-python")
            .await
            .expect("la recolección debería completarse");

        assert_eq!(alertas.len(), 101, "no debe truncarse en la primera página");
    }

    /// M6 — Una colección de code scanning de 2 páginas encadenadas por `Link`
    /// se recolecta entera y se marca como configurada.
    #[tokio::test]
    async fn code_scanning_recolecta_dos_paginas() {
        let server = MockServer::start().await;
        let cursor = "Y3Vyc29yOnYyOpPPAAAB";
        let link = format!(
            "<{uri}/repos/atareao/reto-python/code-scanning/alerts?state=open&per_page=100&after={cursor}>; rel=\"next\"",
            uri = server.uri()
        );

        let pagina1: Vec<Value> = (1..=100).map(alerta_code).collect();
        let pagina2: Vec<Value> = (101..=101).map(alerta_code).collect();

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/code-scanning/alerts"))
            .and(query_param("state", "open"))
            .and(query_param_is_missing("after"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", link.as_str())
                    .set_body_json(pagina1),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/code-scanning/alerts"))
            .and(query_param("after", cursor))
            .respond_with(ResponseTemplate::new(200).set_body_json(pagina2))
            .mount(&server)
            .await;

        let client = cliente(&server);
        let report = code_scanning::collect(&client, "atareao/reto-python")
            .await
            .expect("la recolección debería completarse");

        assert!(report.configured, "debe marcarse como configurado");
        assert_eq!(
            report.alerts.len(),
            101,
            "no debe truncarse en la primera página"
        );
    }

    /// M3 — El `403` de capacidad deshabilitada reutilizado por code scanning
    /// se traduce a `configured = false` sin abortar.
    #[tokio::test]
    async fn code_scanning_403_disabled_marca_no_configurado() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/atareao/reto-python/code-scanning/alerts"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "message": "Dependabot alerts are disabled for this repository."
            })))
            .mount(&server)
            .await;

        let client = cliente(&server);
        let report = code_scanning::collect(&client, "atareao/reto-python")
            .await
            .expect("una capacidad no disponible no debe abortar");

        assert!(!report.configured, "debe marcarse como no configurado");
        assert!(report.alerts.is_empty());
    }
}
