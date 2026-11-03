//! Interpretación de la cabecera `Link` para seguir la paginación de la API de
//! GitHub.
//!
//! La API devuelve la página siguiente como una URL absoluta en la cabecera
//! `Link`, con varias relaciones separadas por comas, por ejemplo:
//!
//! ```text
//! <https://api.github.com/repos/o/r/alerts?page=2>; rel="next", <...>; rel="last"
//! ```
//!
//! La extracción es una función **pura**: recibe el valor de la cabecera y
//! decide, sin IO, si hay continuación.

/// Extrae la URL asociada a `rel="next"` de una cabecera `Link`.
///
/// Devuelve `None` si la cabecera no está, si no contiene una relación `next`
/// o si la entrada de `next` no trae una URL entre `<` y `>`.
///
/// Tolera varias relaciones en la misma cabecera, espacios variables alrededor
/// de la puntuación y la forma sin comillas (`rel=next`) o con varias
/// relaciones dentro del mismo `rel` (`rel="next last"`).
#[must_use]
pub fn next_link(link_header: Option<&str>) -> Option<String> {
    let cabecera = link_header?;

    for entrada in cabecera.split(',') {
        let mut url: Option<String> = None;
        let mut es_next = false;

        for parametro in entrada.split(';') {
            let parametro = parametro.trim();
            if let Some(resto) = parametro
                .strip_prefix('<')
                .and_then(|sin_inicio| sin_inicio.strip_suffix('>'))
            {
                url = Some(resto.to_string());
            } else if let Some(valor) = parametro.strip_prefix("rel=") {
                let valor = valor.trim().trim_matches('"');
                es_next = valor.split_whitespace().any(|rel| rel == "next");
            }
        }

        // Solo es una continuación si la relación `next` va acompañada de URL.
        if es_next {
            return url;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una cabecera con `rel="next"` entre otras relaciones devuelve su URL.
    #[test]
    fn next_link_extrae_la_url_de_next() {
        let cabecera = "<https://api.github.com/items?page=2>; rel=\"next\", \
                        <https://api.github.com/items?page=5>; rel=\"last\"";
        assert_eq!(
            next_link(Some(cabecera)).as_deref(),
            Some("https://api.github.com/items?page=2")
        );
    }

    /// Tolera espacios variables alrededor de la puntuación.
    #[test]
    fn next_link_tolera_espacios_variables() {
        let cabecera = "<https://example.com/p2>;rel=next";
        assert_eq!(
            next_link(Some(cabecera)).as_deref(),
            Some("https://example.com/p2")
        );
    }

    /// `next` puede venir dentro de un `rel` con varias relaciones.
    #[test]
    fn next_link_acepta_next_entre_varias_relaciones() {
        let cabecera = "<https://example.com/p3>; rel=\"next last\"";
        assert_eq!(
            next_link(Some(cabecera)).as_deref(),
            Some("https://example.com/p3")
        );
    }

    /// Sin relación `next` no hay continuación.
    #[test]
    fn next_link_sin_next_devuelve_none() {
        let cabecera = "<https://api.github.com/items?page=1>; rel=\"prev\", \
                        <https://api.github.com/items?page=4>; rel=\"last\"";
        assert_eq!(next_link(Some(cabecera)), None);
    }

    /// Una cabecera ausente no tiene continuación.
    #[test]
    fn next_link_sin_cabecera_devuelve_none() {
        assert_eq!(next_link(None), None);
    }

    /// Una entrada `next` sin URL entre `<` y `>` no es una continuación válida.
    #[test]
    fn next_link_malformada_devuelve_none() {
        assert_eq!(next_link(Some("rel=\"next\"")), None);
    }

    /// Una cabecera vacía no tiene continuación.
    #[test]
    fn next_link_vacia_devuelve_none() {
        assert_eq!(next_link(Some("")), None);
    }
}
