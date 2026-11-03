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

    /// Escenario 3: sin credencial alguna, el error explica las dos vías.
    #[test]
    fn error_sin_credenciales_menciona_las_dos_vias() {
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
    }

    /// Escenario 3 (variante): `gh` falla con un código; el error se envuelve en
    /// `NoCredentials` y sigue explicando las dos vías.
    #[test]
    fn error_cuando_gh_falla_menciona_las_dos_vias() {
        let err = resolve_token_from(|_| None, || Err(AuthError::GhFailed { code: Some(1) }))
            .expect_err("un fallo de `gh` debe producir error");

        assert!(
            matches!(err, AuthError::NoCredentials { .. }),
            "se esperaba NoCredentials, no: {err:?}"
        );

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

    /// Escenario 3 (variante): el runner devuelve `GhEmpty`; el error sigue
    /// explicando las dos vías.
    ///
    /// Este caso cubre el error del runner, no una salida vacía.
    #[test]
    fn error_cuando_el_runner_devuelve_gh_empty_menciona_las_dos_vias() {
        let err = resolve_token_from(|_| None, || Err(AuthError::GhEmpty))
            .expect_err("un error del runner debe producir error");

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

    /// Escenario 3 (variante): `gh` termina bien pero devuelve una salida vacía.
    ///
    /// A diferencia del test anterior, aquí el runner devuelve `Ok` con una
    /// cadena en blanco, de modo que ejercita la rama que convierte una salida
    /// vacía en `NoCredentials` con causa `GhEmpty`.
    #[test]
    fn error_cuando_gh_devuelve_salida_vacia_menciona_las_dos_vias() {
        let err = resolve_token_from(|_| None, || Ok("   \n\t".to_string()))
            .expect_err("una salida vacía debe producir error");

        assert!(
            matches!(err, AuthError::NoCredentials { .. }),
            "se esperaba NoCredentials, no: {err:?}"
        );

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

    /// Un `GITHUB_TOKEN` presente pero vacío o en blanco cae a la vía de `gh`.
    #[test]
    fn entorno_vacio_o_en_blanco_cae_a_gh() {
        let gh_invocado = Cell::new(false);

        let token = resolve_token_from(
            |_| Some("   ".to_string()),
            || {
                gh_invocado.set(true);
                Ok("ghp_desde_gh".to_string())
            },
        )
        .expect("con el entorno en blanco debe usarse `gh`");

        assert!(gh_invocado.get(), "el entorno en blanco debe caer a `gh`");
        assert_eq!(token.expose_secret(), "ghp_desde_gh");
    }

    /// El valor del entorno se recorta antes de usarse.
    #[test]
    fn recorta_el_valor_del_entorno() {
        let gh_invocado = Cell::new(false);

        let token = resolve_token_from(
            |_| Some("  ghp_recortado\n".to_string()),
            || {
                gh_invocado.set(true);
                Ok("no debería usarse".to_string())
            },
        )
        .expect("con el entorno presente debe usarse ese token");

        assert_eq!(token.expose_secret(), "ghp_recortado");
        assert!(
            !gh_invocado.get(),
            "no debe invocarse `gh` si el entorno vale"
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
