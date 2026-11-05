//! Tests de integración de la priorización determinista.
//!
//! La priorización es una función pura sobre el modelo de alerta: no toca red
//! ni disco. El instante de referencia (`now`) se inyecta, de modo que la
//! antigüedad es reproducible sin depender del reloj real. Los tests viven en el
//! módulo `priority` para que el filtro `cargo test priority` los seleccione por
//! nombre.

mod priority {
    use chrono::{DateTime, TimeZone, Utc};

    use ghwatch::domain::alert::{Alert, AlertSource, Severity};
    use ghwatch::domain::priority::{prioritize, score};

    /// Instante determinista a partir de un segundo Unix.
    fn instante(segundos: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(segundos, 0)
            .single()
            .expect("instante válido")
    }

    /// Un día en segundos.
    const DIA: i64 = 86_400;

    /// Alerta con todos los campos a `None` salvo el identificador, el paquete y
    /// el ecosistema.
    fn alerta(external_id: &str) -> Alert {
        Alert {
            source: AlertSource::Dependabot,
            external_id: external_id.to_string(),
            ghsa_id: None,
            cve_id: None,
            severity: None,
            cvss: None,
            summary: None,
            package: Some("django".to_string()),
            ecosystem: Some("pip".to_string()),
            manifest_path: None,
            scope: None,
            vulnerable_range: None,
            first_patched: None,
            secret_type: None,
            created_at: None,
        }
    }

    /// Identificadores de las alertas en el orden devuelto.
    fn ids<'a>(alertas: &[&'a Alert]) -> Vec<&'a str> {
        alertas
            .iter()
            .map(|alerta| alerta.external_id.as_str())
            .collect()
    }

    /// Escenario «Influencia de la antigüedad»: con idénticos campos de entrada,
    /// la alerta más antigua puntúa más alto y encabeza el orden.
    #[test]
    fn la_mas_antigua_puntua_mas_alto() {
        let now = instante(2_000_000_000);

        let mut antigua = alerta("antigua");
        antigua.created_at = Some(instante(2_000_000_000 - 30 * DIA));
        let mut reciente = alerta("reciente");
        reciente.created_at = Some(instante(2_000_000_000 - DIA));

        assert!(
            score(&antigua, now) > score(&reciente, now),
            "la más antigua debe puntuar más alto"
        );

        let alertas = vec![reciente, antigua];
        let orden = prioritize(&alertas, now);
        assert_eq!(ids(&orden), vec!["antigua", "reciente"]);
    }

    /// Escenario «Reproducibilidad del orden»: con los mismos datos, el orden es
    /// idéntico entre ejecuciones y **no depende del orden de entrada**.
    #[test]
    fn el_orden_es_reproducible_e_independiente_del_orden_de_entrada() {
        let now = instante(2_000_000_000);

        let mut a = alerta("a");
        a.severity = Some(Severity::High);
        a.cvss = Some(7.0);
        let mut b = alerta("b");
        b.severity = Some(Severity::High);
        b.cvss = Some(7.0);
        let mut c = alerta("c");
        c.severity = Some(Severity::Critical);
        let mut d = alerta("d");
        d.severity = Some(Severity::Low);
        d.first_patched = Some("1.2.3".to_string());

        let directo = vec![a.clone(), b.clone(), c.clone(), d.clone()];
        let inverso = vec![d, c, b, a];

        let primera = prioritize(&directo, now);
        let segunda = prioritize(&directo, now);
        let al_reves = prioritize(&inverso, now);

        assert_eq!(
            ids(&primera),
            ids(&segunda),
            "mismo orden entre ejecuciones"
        );
        assert_eq!(
            ids(&primera),
            ids(&al_reves),
            "el orden no depende del orden de entrada"
        );
        assert_eq!(ids(&primera)[0], "c", "la crítica encabeza el orden");
    }

    /// La severidad domina sobre la antigüedad: una crítica reciente va por
    /// delante de una baja muy antigua.
    #[test]
    fn la_severidad_domina_sobre_la_antiguedad() {
        let now = instante(2_000_000_000);

        let mut critica_reciente = alerta("critica");
        critica_reciente.severity = Some(Severity::Critical);
        critica_reciente.created_at = Some(instante(2_000_000_000 - DIA));

        let mut baja_antigua = alerta("baja");
        baja_antigua.severity = Some(Severity::Low);
        baja_antigua.created_at = Some(instante(2_000_000_000 - 1000 * DIA));

        assert!(score(&critica_reciente, now) > score(&baja_antigua, now));
    }

    /// El `scope` `runtime` puntúa por encima de `development`.
    #[test]
    fn runtime_puntua_mas_que_development() {
        let now = instante(2_000_000_000);

        let mut runtime = alerta("runtime");
        runtime.scope = Some("runtime".to_string());
        let mut development = alerta("development");
        development.scope = Some("development".to_string());

        assert!(score(&runtime, now) > score(&development, now));
    }

    /// Una versión corregida disponible y un CVSS alto suben la puntuación.
    #[test]
    fn el_cvss_y_la_version_corregida_suben_la_puntuacion() {
        let now = instante(2_000_000_000);

        let base = alerta("base");

        let mut con_cvss = base.clone();
        con_cvss.cvss = Some(9.8);
        let mut con_fix = base.clone();
        con_fix.first_patched = Some("3.0.2".to_string());

        assert!(score(&con_cvss, now) > score(&base, now));
        assert!(score(&con_fix, now) > score(&base, now));
    }

    /// M3 — El desempate es **total**: dos alertas idénticas en todo lo que
    /// afecta a la puntuación se ordenan por los campos restantes
    /// (`secret_type` aquí), con independencia del orden de entrada.
    #[test]
    fn el_desempate_es_total_hasta_el_ultimo_campo() {
        let now = instante(2_000_000_000);

        let mut alpha = alerta("1");
        alpha.secret_type = Some("alpha".to_string());
        let mut beta = alerta("1");
        beta.secret_type = Some("beta".to_string());

        assert_eq!(
            score(&alpha, now),
            score(&beta, now),
            "las dos alertas puntúan igual"
        );

        let directo_alerta = vec![alpha.clone(), beta.clone()];
        let inverso_alerta = vec![beta, alpha];
        let directo = prioritize(&directo_alerta, now);
        let inverso = prioritize(&inverso_alerta, now);

        assert_eq!(
            directo[0].secret_type.as_deref(),
            Some("alpha"),
            "el desempate usa un campo que no afecta a la puntuación"
        );
        assert_eq!(
            inverso[0].secret_type.as_deref(),
            Some("alpha"),
            "el orden es total e independiente del orden de entrada"
        );
    }
}
