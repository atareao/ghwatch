//! Tests de la clasificación de deriva del ciclo de vida de las alertas.
//!
//! La deriva es una función **pura**: no toca red ni disco, solo compara el
//! estado previo de una alerta con su presencia y severidad actuales. Los tests
//! viven en el módulo `drift` para que el filtro `cargo test drift` los
//! seleccione por nombre.

mod drift {
    use ghwatch::domain::{AlertState, Drift, PriorAlert, Severity};

    /// Alerta previa que estaba abierta con la severidad indicada.
    fn abierta(severity: Option<Severity>) -> PriorAlert {
        PriorAlert {
            state: AlertState::Open,
            severity,
        }
    }

    /// Alerta previa que ya estaba resuelta con la severidad indicada.
    fn resuelta(severity: Option<Severity>) -> PriorAlert {
        PriorAlert {
            state: AlertState::Resolved,
            severity,
        }
    }

    /// Una alerta que aparece por primera vez es `new`.
    #[test]
    fn aparece_por_primera_vez_es_new() {
        assert_eq!(
            Drift::classify(None, Some(Severity::Medium)),
            Some(Drift::New)
        );
    }

    /// Una alerta presente **sin** severidad aportada también es `new`: la
    /// presencia no depende de que la fuente publique la severidad.
    #[test]
    fn aparece_sin_severidad_es_new() {
        assert_eq!(
            Drift::classify(None, Some(Severity::Unknown)),
            Some(Drift::New)
        );
    }

    /// Una alerta abierta que sigue presente sin cambiar de severidad es
    /// `still_open`.
    #[test]
    fn abierta_que_sigue_presente_es_still_open() {
        assert_eq!(
            Drift::classify(
                Some(abierta(Some(Severity::Medium))),
                Some(Severity::Medium)
            ),
            Some(Drift::StillOpen)
        );
    }

    /// Una alerta abierta que desaparece queda `resolved`.
    #[test]
    fn abierta_que_desaparece_es_resolved() {
        assert_eq!(
            Drift::classify(Some(abierta(Some(Severity::Medium))), None),
            Some(Drift::Resolved)
        );
    }

    /// Una alerta resuelta que reaparece es `reopened`, nunca `new`.
    #[test]
    fn resuelta_que_reaparece_es_reopened() {
        let cambio = Drift::classify(
            Some(resuelta(Some(Severity::Medium))),
            Some(Severity::Medium),
        );
        assert_eq!(cambio, Some(Drift::Reopened));
        assert_ne!(cambio, Some(Drift::New), "una reapertura no es un hallazgo");
    }

    /// Una subida de severidad (`medium` → `critical`) es `severity_up`.
    #[test]
    fn subida_de_severidad_es_severity_up() {
        assert_eq!(
            Drift::classify(
                Some(abierta(Some(Severity::Medium))),
                Some(Severity::Critical)
            ),
            Some(Drift::SeverityUp)
        );
    }

    /// Una bajada de severidad no es una subida: sigue `still_open`.
    #[test]
    fn bajada_de_severidad_es_still_open() {
        assert_eq!(
            Drift::classify(Some(abierta(Some(Severity::High))), Some(Severity::Low)),
            Some(Drift::StillOpen)
        );
    }

    /// Pasar de severidad desconocida a conocida cuenta como subida.
    #[test]
    fn de_desconocida_a_conocida_es_severity_up() {
        assert_eq!(
            Drift::classify(Some(abierta(None)), Some(Severity::Low)),
            Some(Drift::SeverityUp)
        );
    }

    /// Una alerta ya resuelta que sigue ausente no produce cambio: el conjunto
    /// cerrado no tiene categoría para «sigue resuelta».
    #[test]
    fn resuelta_que_sigue_ausente_no_produce_cambio() {
        assert_eq!(Drift::classify(Some(resuelta(None)), None), None);
    }

    /// El conjunto de categorías es **cerrado** y se representa en `snake_case`.
    #[test]
    fn las_categorias_son_cerradas_y_en_snake_case() {
        let esperado = ["new", "still_open", "resolved", "reopened", "severity_up"];
        let obtenido: Vec<&str> = Drift::ALL
            .iter()
            .map(|categoria| categoria.as_str())
            .collect();
        assert_eq!(obtenido, esperado);
        assert_eq!(Drift::ALL.len(), 5);
    }

    /// Cada categoría se puede leer de vuelta desde su representación textual.
    #[test]
    fn las_categorias_se_leen_de_vuelta() {
        for categoria in Drift::ALL {
            assert_eq!(Drift::parse(categoria.as_str()), Some(categoria));
        }
        assert_eq!(Drift::parse("inventada"), None);
    }

    /// El estado del ciclo de vida también se representa en `snake_case`.
    #[test]
    fn los_estados_se_representan_en_snake_case() {
        assert_eq!(AlertState::Open.as_str(), "open");
        assert_eq!(AlertState::Resolved.as_str(), "resolved");
        assert_eq!(AlertState::parse("resolved"), Some(AlertState::Resolved));
        assert_eq!(AlertState::parse("otro"), None);
    }
}
