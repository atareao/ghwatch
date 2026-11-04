//! Mapeo de ecosistemas de GitHub a ecosistemas de OSV.
//!
//! El conjunto es **cerrado**: solo los ecosistemas que GitHub publica y OSV
//! reconoce con un nombre distinto (o no) están en la tabla. Un ecosistema sin
//! equivalencia no se traduce: el enriquecimiento lo omite sin error y lo
//! registra en modo depuración.
//!
//! # Riesgo de grafía en `github-actions`
//!
//! GitHub nombra el ecosistema de acciones de dos formas según el contexto
//! (`github-actions` en la configuración de Dependabot, `github_actions` en
//! algunos cuerpos de la API). Para no perder el enriquecimiento por una
//! diferencia de separador, se aceptan **ambas** grafías y se traducen al mismo
//! ecosistema de OSV.

/// Traduce el ecosistema que publica GitHub al que espera OSV.
///
/// La comparación ignora mayúsculas y espacios sobrantes. Devuelve `None` para
/// un ecosistema sin equivalencia declarada.
#[must_use]
pub fn map_ecosystem(github: &str) -> Option<&'static str> {
    match github.trim().to_ascii_lowercase().as_str() {
        "pip" => Some("PyPI"),
        "npm" => Some("npm"),
        "cargo" => Some("crates.io"),
        "go" => Some("Go"),
        "composer" => Some("Packagist"),
        "maven" => Some("Maven"),
        "nuget" => Some("NuGet"),
        "bundler" => Some("RubyGems"),
        "github-actions" | "github_actions" => Some("GitHub Actions"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La tabla cubre los nueve ecosistemas declarados.
    #[test]
    fn cubre_la_tabla_cerrada() {
        assert_eq!(map_ecosystem("pip"), Some("PyPI"));
        assert_eq!(map_ecosystem("npm"), Some("npm"));
        assert_eq!(map_ecosystem("cargo"), Some("crates.io"));
        assert_eq!(map_ecosystem("go"), Some("Go"));
        assert_eq!(map_ecosystem("composer"), Some("Packagist"));
        assert_eq!(map_ecosystem("maven"), Some("Maven"));
        assert_eq!(map_ecosystem("nuget"), Some("NuGet"));
        assert_eq!(map_ecosystem("bundler"), Some("RubyGems"));
        assert_eq!(map_ecosystem("github-actions"), Some("GitHub Actions"));
    }

    /// La comparación ignora mayúsculas y espacios sobrantes.
    #[test]
    fn ignora_mayusculas_y_espacios() {
        assert_eq!(map_ecosystem("  pip "), Some("PyPI"));
        assert_eq!(map_ecosystem("CARGO"), Some("crates.io"));
    }

    /// M5 — Las dos grafías de GitHub Actions se traducen igual.
    #[test]
    fn acepta_las_dos_graficas_de_github_actions() {
        assert_eq!(map_ecosystem("github-actions"), Some("GitHub Actions"));
        assert_eq!(map_ecosystem("github_actions"), Some("GitHub Actions"));
        assert_eq!(map_ecosystem("GitHub_Actions"), Some("GitHub Actions"));
    }

    /// Un ecosistema sin equivalencia no se traduce.
    #[test]
    fn un_ecosistema_desconocido_no_se_traduce() {
        assert_eq!(map_ecosystem("swift"), None);
        assert_eq!(map_ecosystem(""), None);
    }
}
