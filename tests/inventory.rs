//! Tests de integración del descubrimiento y filtrado del inventario.
//!
//! Se usa `wiremock` como servidor falso: la suite no hace red real. Los tests
//! viven en el módulo `inventory` para que el filtro `cargo test inventory` los
//! seleccione por nombre.

mod inventory {
    use ghwatch::auth::Token;
    use ghwatch::domain::repo::DependabotState;
    use ghwatch::github::repos::{self, RepoSelection};
    use ghwatch::github::{GithubClient, GithubConfig};
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

    /// Construye el JSON de un repositorio del inventario.
    fn repo_json(indice: usize, private: bool, fork: bool, archived: bool) -> Value {
        serde_json::json!({
            "full_name": format!("atareao/repo-{indice:03}"),
            "private": private,
            "archived": archived,
            "fork": fork,
            "language": "Rust",
            "pushed_at": "2026-11-03T09:00:00Z",
        })
    }

    /// Inventario de 433 repositorios de los que 66 son forks o archivados:
    /// 40 forks (0..40) y 26 archivados (40..66), categorías disjuntas. Por
    /// defecto deben quedar 367.
    fn inventario_433() -> Vec<Value> {
        (0..433)
            .map(|indice| {
                let fork = indice < 40;
                let archived = (40..66).contains(&indice);
                repo_json(indice, false, fork, archived)
            })
            .collect()
    }

    /// Monta el inventario repartido en páginas de 100, encadenadas por `Link`
    /// con `rel="next"`; la última página no incluye enlace de continuación.
    async fn montar_inventario(server: &MockServer, repos: &[Value]) {
        let paginas: Vec<&[Value]> = repos.chunks(100).collect();
        let total = paginas.len();

        for (indice, pagina) in paginas.iter().enumerate() {
            let numero = indice + 1;
            let mut mock = Mock::given(method("GET")).and(path("/user/repos"));
            mock = if numero == 1 {
                mock.and(query_param_is_missing("page"))
            } else {
                mock.and(query_param("page", numero.to_string()))
            };

            let mut respuesta = ResponseTemplate::new(200);
            if numero < total {
                let siguiente = format!(
                    "<{uri}/user/repos?per_page=100&page={page}>; rel=\"next\"",
                    uri = server.uri(),
                    page = numero + 1
                );
                respuesta = respuesta.insert_header("Link", siguiente.as_str());
            }
            mock.respond_with(respuesta.set_body_json(pagina))
                .mount(server)
                .await;
        }
    }

    /// Cuenta las peticiones recibidas por el servidor falso.
    async fn peticiones(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .expect("peticiones recibidas")
            .len()
    }

    // --- 3.1 El descubrimiento enumera el inventario completo ---

    /// 433 repositorios en 5 páginas: el inventario contiene exactamente 433
    /// entradas y se solicitan las 5 páginas (no se trunca en la primera).
    #[tokio::test]
    async fn descubrimiento_enumera_433_repositorios_en_cinco_paginas() {
        let server = MockServer::start().await;
        montar_inventario(&server, &inventario_433()).await;

        let client = cliente(&server);
        let repos = repos::discover(&client)
            .await
            .expect("el descubrimiento debería completarse");

        assert_eq!(repos.len(), 433, "no debe truncarse el inventario");
        assert_eq!(peticiones(&server).await, 5, "deben pedirse las 5 páginas");
    }

    /// Una última página sin `rel="next"` termina la enumeración y no dispara
    /// ninguna petición adicional.
    #[tokio::test]
    async fn pagina_final_sin_rel_next_termina_la_enumeracion() {
        let server = MockServer::start().await;
        let repos = vec![
            repo_json(0, false, false, false),
            repo_json(1, false, false, false),
            repo_json(2, false, false, false),
        ];
        montar_inventario(&server, &repos).await;

        let client = cliente(&server);
        let descubiertos = repos::discover(&client)
            .await
            .expect("el descubrimiento debería completarse");

        assert_eq!(descubiertos.len(), 3);
        assert_eq!(
            peticiones(&server).await,
            1,
            "no debe pedir una página de más"
        );
    }

    // --- 3.2 La selección por defecto excluye forks y archivados ---

    /// Sin flags, el inventario de 433 repositorios queda en 367 (excluye los
    /// 66 forks o archivados).
    #[tokio::test]
    async fn seleccion_por_defecto_excluye_forks_y_archivados() {
        let server = MockServer::start().await;
        montar_inventario(&server, &inventario_433()).await;

        let client = cliente(&server);
        let todos = repos::discover(&client).await.expect("descubrimiento");
        assert_eq!(todos.len(), 433);

        let seleccionados = repos::select(todos, RepoSelection::default());
        assert_eq!(seleccionados.len(), 367, "deben quedar 367 repositorios");
        assert!(
            seleccionados
                .iter()
                .all(|repo| !repo.fork && !repo.archived),
            "la selección por defecto no debe incluir forks ni archivados"
        );
    }

    /// `--include-forks` reincorpora los 40 forks excluidos (367 → 407).
    #[tokio::test]
    async fn include_forks_reincorpora_los_forks() {
        let server = MockServer::start().await;
        montar_inventario(&server, &inventario_433()).await;

        let client = cliente(&server);
        let todos = repos::discover(&client).await.expect("descubrimiento");

        let seleccionados = repos::select(
            todos,
            RepoSelection {
                include_forks: true,
                include_archived: false,
            },
        );

        assert_eq!(seleccionados.len(), 407, "367 + 40 forks");
        assert!(
            seleccionados.iter().any(|repo| repo.fork),
            "los forks deben reincorporarse"
        );
        assert!(
            seleccionados.iter().all(|repo| !repo.archived),
            "los archivados siguen excluidos"
        );
    }

    /// `--include-archived` reincorpora los 26 archivados excluidos (367 → 393).
    #[tokio::test]
    async fn include_archived_reincorpora_los_archivados() {
        let server = MockServer::start().await;
        montar_inventario(&server, &inventario_433()).await;

        let client = cliente(&server);
        let todos = repos::discover(&client).await.expect("descubrimiento");

        let seleccionados = repos::select(
            todos,
            RepoSelection {
                include_forks: false,
                include_archived: true,
            },
        );

        assert_eq!(seleccionados.len(), 393, "367 + 26 archivados");
        assert!(
            seleccionados.iter().any(|repo| repo.archived),
            "los archivados deben reincorporarse"
        );
        assert!(
            seleccionados.iter().all(|repo| !repo.fork),
            "los forks siguen excluidos"
        );
    }

    // --- 3.2bis Metadato de identidad y estado de Dependabot ---

    /// Un repositorio privado conserva su metadato, su `pushed_at` no es nulo y
    /// su estado de Dependabot arranca como `unknown`.
    #[tokio::test]
    async fn repositorio_privado_conserva_metadato_y_dependabot_desconocido() {
        let server = MockServer::start().await;
        let repos = vec![repo_json(0, true, false, false)];
        montar_inventario(&server, &repos).await;

        let client = cliente(&server);
        let descubiertos = repos::discover(&client).await.expect("descubrimiento");

        assert_eq!(descubiertos.len(), 1);
        let repo = &descubiertos[0];
        assert_eq!(repo.name_with_owner, "atareao/repo-000");
        assert!(!repo.name_with_owner.is_empty());
        assert!(repo.private, "el repositorio es privado");
        assert!(repo.pushed_at.is_some(), "pushed_at no debe ser nulo");
        assert_eq!(
            repo.dependabot,
            DependabotState::Unknown,
            "el estado de Dependabot aún no se ha consultado"
        );
    }
}
