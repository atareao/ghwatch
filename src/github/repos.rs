//! Descubrimiento y filtrado del inventario de repositorios.
//!
//! El descubrimiento reutiliza [`GithubClient::get_paginated`], de modo que
//! recorre **todas** las páginas siguiendo `Link` con `per_page=100` sin truncar
//! el inventario. El filtrado es una función pura sobre el inventario ya
//! descargado: no toca la red.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::domain::repo::{DependabotState, Repo};

use super::client::GithubClient;
use super::error::GithubError;

/// Ruta del endpoint que enumera los repositorios de la cuenta autenticada.
const REPOS_PATH: &str = "/user/repos";

/// Política de selección del inventario.
///
/// Por defecto se excluyen los forks y los archivados; cada bandera reincorpora
/// una de esas categorías. La bandera CLI (`--include-forks`,
/// `--include-archived`) se traduce a este struct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RepoSelection {
    /// Reincorpora los forks (excluidos por defecto).
    pub include_forks: bool,
    /// Reincorpora los repositorios archivados (excluidos por defecto).
    pub include_archived: bool,
}

impl RepoSelection {
    /// Indica si `repo` entra en la selección.
    #[must_use]
    pub fn includes(&self, repo: &Repo) -> bool {
        (self.include_forks || !repo.fork) && (self.include_archived || !repo.archived)
    }
}

/// Filtra `repos` conforme a `selection`, preservando el orden de entrada.
#[must_use]
pub fn select(repos: Vec<Repo>, selection: RepoSelection) -> Vec<Repo> {
    repos
        .into_iter()
        .filter(|repo| selection.includes(repo))
        .collect()
}

/// Descubre **todos** los repositorios de la cuenta, sin truncar.
///
/// Cada repositorio se registra con su metadato de identidad y el estado de
/// Dependabot a [`DependabotState::Unknown`]: averiguarlo exige consultar sus
/// alertas, algo que ocurre en la fase de recolección.
///
/// # Errores
///
/// Propaga el [`GithubError`] de cualquier página del inventario.
pub async fn discover(client: &GithubClient) -> Result<Vec<Repo>, GithubError> {
    let crudos: Vec<RawRepo> = client.get_paginated(REPOS_PATH).await?;
    Ok(crudos.into_iter().map(RawRepo::into_domain).collect())
}

/// Forma cruda de un repositorio tal y como lo devuelve la API de GitHub.
///
/// Solo se deserializan los campos que el modelo de dominio necesita; el resto
/// se ignora. `serde(default)` mantiene la tolerancia ante campos ausentes.
#[derive(Debug, Deserialize)]
struct RawRepo {
    /// Nombre completo (`owner/name`); GitHub lo llama `full_name`.
    full_name: String,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    pushed_at: Option<String>,
}

impl RawRepo {
    /// Convierte la forma cruda en el metadato de dominio.
    fn into_domain(self) -> Repo {
        Repo {
            name_with_owner: self.full_name,
            private: self.private,
            archived: self.archived,
            fork: self.fork,
            language: self.language,
            pushed_at: parse_datetime(self.pushed_at.as_deref()),
            dependabot: DependabotState::Unknown,
        }
    }
}

/// Interpreta una fecha ISO-8601 (`RFC 3339`) en UTC.
///
/// Una fecha ausente o ilegible se traduce a `None` en lugar de fallar: el
/// metadato de identidad no debe hacer fracasar el descubrimiento.
fn parse_datetime(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|texto| DateTime::parse_from_rfc3339(texto).ok())
        .map(|fecha| fecha.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construye un repositorio de prueba con las categorías indicadas.
    fn repo(fork: bool, archived: bool) -> Repo {
        Repo {
            name_with_owner: "atareao/demo".to_string(),
            private: false,
            archived,
            fork,
            language: None,
            pushed_at: None,
            dependabot: DependabotState::Unknown,
        }
    }

    /// La selección por defecto excluye forks y archivados.
    #[test]
    fn seleccion_por_defecto_excluye_forks_y_archivados() {
        let inventario = vec![
            repo(false, false),
            repo(true, false),
            repo(false, true),
            repo(true, true),
        ];
        let seleccionados = select(inventario, RepoSelection::default());
        assert_eq!(seleccionados.len(), 1);
        assert!(!seleccionados[0].fork && !seleccionados[0].archived);
    }

    /// `include_forks` reincorpora los forks y suma `include_archived` los
    /// archivados.
    #[test]
    fn las_banderas_reincorporan_cada_categoria() {
        let inventario = vec![repo(false, false), repo(true, false), repo(false, true)];
        let con_forks = select(
            inventario.clone(),
            RepoSelection {
                include_forks: true,
                include_archived: false,
            },
        );
        assert_eq!(con_forks.len(), 2);

        let con_archivados = select(
            inventario.clone(),
            RepoSelection {
                include_forks: false,
                include_archived: true,
            },
        );
        assert_eq!(con_archivados.len(), 2);

        let con_todos = select(
            inventario,
            RepoSelection {
                include_forks: true,
                include_archived: true,
            },
        );
        assert_eq!(con_todos.len(), 3);
    }

    /// `parse_datetime` interpreta una fecha RFC 3339 y tolera lo ilegible.
    #[test]
    fn parse_datetime_interpreta_y_tolera() {
        assert!(parse_datetime(Some("2026-11-03T09:00:00Z")).is_some());
        assert!(parse_datetime(Some("no es una fecha")).is_none());
        assert!(parse_datetime(None).is_none());
    }
}
