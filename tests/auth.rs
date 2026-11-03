//! Tests de integración del módulo `ghwatch::auth`.
//!
//! Se inyectan dobles (closures) tanto para la lectura del entorno como para la
//! ejecución de `gh`, de modo que los tests no tocan el entorno del proceso ni
//! lanzan procesos externos. Los tests viven en el módulo `auth` para que el
//! filtro `cargo test auth` los seleccione por nombre.

mod auth {
    use std::cell::Cell;

    use ghwatch::auth::{AuthError, Token, resolve_token_from};

    /// Escenario 1: con `GITHUB_TOKEN` presente se usa ese valor y **no** se invoca `gh`.
    #[test]
    fn usa_github_token_del_entorno_sin_invocar_gh() {
        let gh_invocado = Cell::new(false);

        let token = resolve_token_from(
            |clave| (clave == "GITHUB_TOKEN").then(|| "ghp_valor_del_entorno".to_string()),
            || {
                gh_invocado.set(true);
                Ok("ghp_valor_de_gh".to_string())
            },
        )
        .expect("la resolución debería tener éxito con GITHUB_TOKEN presente");

        assert_eq!(token.expose_secret(), "ghp_valor_del_entorno");
        assert!(
            !gh_invocado.get(),
            "no se debe invocar `gh` si GITHUB_TOKEN existe"
        );
    }

    /// Escenario 2: sin `GITHUB_TOKEN`, se obtiene el token de `gh auth token`.
    #[test]
    fn obtiene_token_de_gh_cuando_no_hay_entorno() {
        let token = resolve_token_from(|_| None, || Ok("ghp_valor_de_gh".to_string()))
            .expect("la resolución debería tener éxito con `gh` disponible");

        assert_eq!(token.expose_secret(), "ghp_valor_de_gh");
    }

    /// Escenario 3: sin credencial alguna, el error explica las dos vías y no filtra token.
    #[test]
    fn error_sin_credenciales_menciona_las_dos_vias_y_no_filtra_token() {
        let err = resolve_token_from(|_| None, || Err(AuthError::GhNotInstalled))
            .expect_err("sin credenciales la resolución debe fallar");

        let mensaje = err.to_string();
        assert!(
            mensaje.contains("GITHUB_TOKEN"),
            "el error debe mencionar GITHUB_TOKEN: {mensaje}"
        );
        assert!(
            mensaje.contains("gh auth token"),
            "el error debe mencionar `gh auth token`: {mensaje}"
        );
        assert!(
            !mensaje.contains("ghp_"),
            "el error no debe contener ningún valor de token: {mensaje}"
        );
    }

    /// Escenario 3 (variante): `gh` falla; el error sigue explicando las dos vías.
    #[test]
    fn error_cuando_gh_falla_menciona_las_dos_vias() {
        let err = resolve_token_from(|_| None, || Err(AuthError::GhFailed { code: Some(1) }))
            .expect_err("un fallo de `gh` debe producir error");

        let mensaje = err.to_string();
        assert!(
            mensaje.contains("GITHUB_TOKEN"),
            "falta GITHUB_TOKEN en: {mensaje}"
        );
        assert!(
            mensaje.contains("gh auth token"),
            "falta `gh auth token` en: {mensaje}"
        );
    }

    /// Escenario 3 (variante): `gh` devuelve vacío; el error sigue explicando las dos vías.
    #[test]
    fn error_cuando_gh_devuelve_vacio_menciona_las_dos_vias() {
        let err = resolve_token_from(|_| None, || Err(AuthError::GhEmpty))
            .expect_err("una salida vacía de `gh` debe producir error");

        let mensaje = err.to_string();
        assert!(
            mensaje.contains("GITHUB_TOKEN"),
            "falta GITHUB_TOKEN en: {mensaje}"
        );
        assert!(
            mensaje.contains("gh auth token"),
            "falta `gh auth token` en: {mensaje}"
        );
    }

    /// El tipo `Token` redacta su valor tanto en `Debug` como en `Display`.
    #[test]
    fn token_redacta_su_valor_en_debug_y_display() {
        let token = Token::new("ghp_secreto_super_secreto").expect("token válido");

        let debug = format!("{token:?}");
        let display = format!("{token}");

        assert!(
            !debug.contains("ghp_secreto_super_secreto"),
            "Debug no debe filtrar el token: {debug}"
        );
        assert!(
            !display.contains("ghp_secreto_super_secreto"),
            "Display no debe filtrar el token: {display}"
        );
        assert_eq!(debug, "Token(***)");
        assert_eq!(display, "***");
    }

    /// Valor secreto reconocible para comprobar la redacción transitiva.
    const SECRETO: &str = "ghp_SECRETO_IRRECONOCIBLE_1234567890";

    /// Estructura local que envuelve un `Token` y deriva `Debug`.
    ///
    /// Sirve para comprobar que la redacción del token se propaga: si `Token`
    /// delegara en su `String` interno, este `Debug` filtraría el secreto.
    #[derive(Debug)]
    struct Envoltorio {
        token: Token,
    }

    /// La redacción es transitiva y ni el `Debug` de una estructura ni los
    /// mensajes de error exponen el token.
    ///
    /// `AuthError` no puede transportar un token porque es un enum cerrado sin
    /// ninguna variante que contenga un `String` con la credencial: las
    /// variantes de fallo de `gh` guardan como mucho un código de salida.
    #[test]
    fn la_redaccion_del_token_no_se_filtra_en_estructuras_ni_errores() {
        let envoltorio = Envoltorio {
            token: Token::new(SECRETO).expect("token válido"),
        };

        let debug_estructura = format!("{envoltorio:?}");
        assert!(
            !debug_estructura.contains(SECRETO),
            "el Debug de la estructura filtra el secreto: {debug_estructura}"
        );

        let display_token = format!("{}", envoltorio.token);
        assert!(
            !display_token.contains(SECRETO),
            "el Display del token filtra el secreto: {display_token}"
        );

        let err = resolve_token_from(|_| None, || Err(AuthError::GhNotInstalled))
            .expect_err("sin credenciales la resolución debe fallar");
        let debug_error = format!("{err:?}");
        assert!(
            !debug_error.contains(SECRETO),
            "el Debug del error filtra el secreto: {debug_error}"
        );
        assert!(
            !err.to_string().contains(SECRETO),
            "el Display del error filtra el secreto: {err}"
        );
    }
}
