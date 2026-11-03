//! Tests de integración del módulo `ghwatch::auth`.
//!
//! Se inyectan dobles (closures) tanto para la lectura del entorno como para la
//! ejecución de `gh`, de modo que los tests no tocan el entorno del proceso ni
//! lanzan procesos externos.

use std::cell::Cell;

use ghwatch::auth::resolve_token_from;

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
    assert!(!gh_invocado.get(), "no se debe invocar `gh` si GITHUB_TOKEN existe");
}
