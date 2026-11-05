//! Tests de integración de la resolución de configuración del CLI.
//!
//! Cubren la precedencia **flags > entorno > fichero > defecto**, el rechazo de
//! un `config.toml` con permisos más laxos que `600` y las rutas por defecto
//! bajo `$HOME`. El entorno real del proceso no se toca: la lectura se inyecta
//! como clausura, de modo que cada test es determinista y aislado.
//!
//! Los tests viven en el módulo `config` para que `cargo test config` los
//! seleccione por nombre.

mod config {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use ghwatch::auth::ENV_VAR as GITHUB_TOKEN_ENV;
    use ghwatch::config::{
        self, ConfigFlags, DEFAULT_CONCURRENCY, ENV_CONCURRENCY, ENV_DB, ENV_INCLUDE_ARCHIVED,
        ENV_INCLUDE_FORKS, ENV_REPORT_DIR,
    };
    use ghwatch::notify::ValetSettings;
    use ghwatch::notify::config::{DEFAULT_BASE_URL, ENV_BASE_URL, ENV_INGEST_TOKEN};

    /// Variable de entorno con el `HOME` inyectado en los tests.
    const HOME: &str = "HOME";

    /// Escribe un `config.toml` con el modo indicado y devuelve su ruta.
    ///
    /// Solo en unix: los permisos de fichero solo existen ahí.
    #[cfg(unix)]
    fn escribir_config(directorio: &tempfile::TempDir, modo: u32, contenido: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let ruta = directorio.path().join("config.toml");
        let mut fichero = std::fs::File::create(&ruta).expect("crear config.toml");
        fichero
            .write_all(contenido.as_bytes())
            .expect("escribir config.toml");
        std::fs::set_permissions(&ruta, std::fs::Permissions::from_mode(modo))
            .expect("fijar permisos");
        ruta
    }

    /// Acceso al entorno a partir de pares clave/valor.
    fn entorno(pares: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let mapa: std::collections::HashMap<String, String> = pares
            .iter()
            .map(|(clave, valor)| ((*clave).to_string(), (*valor).to_string()))
            .collect();
        move |clave| mapa.get(clave).cloned()
    }

    /// Un fichero inexistente cuenta como ausente.
    fn ruta_inexistente(directorio: &tempfile::TempDir) -> PathBuf {
        directorio.path().join("no-existe.toml")
    }

    // --- Precedencia flags > entorno > fichero > defecto ---

    /// El flag gana al entorno y al fichero.
    #[cfg(unix)]
    #[test]
    fn el_flag_gana_al_entorno_y_al_fichero() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(
            &directorio,
            0o600,
            "valet_base_url = \"http://fichero:1111\"\n\
             concurrency = 3\n\
             db_path = \"/fichero.db\"\n",
        );

        let flags = ConfigFlags {
            valet_base_url: Some("http://flag:2222".to_string()),
            concurrency: Some(9),
            db_path: Some(PathBuf::from("/flag.db")),
            ..ConfigFlags::default()
        };
        let get_env = entorno(&[
            (ENV_BASE_URL, "http://entorno:3333"),
            (ENV_CONCURRENCY, "7"),
            (ENV_DB, "/entorno.db"),
        ]);

        let config = config::resolve(&flags, &get_env, Some(&ruta));

        assert_eq!(config.valet_base_url, "http://flag:2222");
        assert_eq!(config.concurrency, 9);
        assert_eq!(config.db_path, PathBuf::from("/flag.db"));
    }

    /// Sin flags, el entorno gana al fichero.
    #[cfg(unix)]
    #[test]
    fn el_entorno_gana_al_fichero() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(
            &directorio,
            0o600,
            "valet_base_url = \"http://fichero:1111\"\n\
             concurrency = 3\n\
             db_path = \"/fichero.db\"\n\
             report_dir = \"/fichero-reports\"\n",
        );

        let get_env = entorno(&[
            (ENV_BASE_URL, "http://entorno:3333"),
            (ENV_CONCURRENCY, "7"),
            (ENV_DB, "/entorno.db"),
            (ENV_REPORT_DIR, "/entorno-reports"),
        ]);

        let config = config::resolve(&ConfigFlags::default(), &get_env, Some(&ruta));

        assert_eq!(config.valet_base_url, "http://entorno:3333");
        assert_eq!(config.concurrency, 7);
        assert_eq!(config.db_path, PathBuf::from("/entorno.db"));
        assert_eq!(config.report_dir, PathBuf::from("/entorno-reports"));
    }

    /// El fichero completa lo que ni el flag ni el entorno aportan.
    #[cfg(unix)]
    #[test]
    fn el_fichero_completa_lo_que_falta() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(
            &directorio,
            0o600,
            "valet_base_url = \"http://fichero:1111\"\n\
             valet_ingest_token = \"val_del_fichero\"\n\
             github_token = \"ghp_del_fichero\"\n\
             concurrency = 3\n\
             db_path = \"/fichero.db\"\n\
             report_dir = \"/fichero-reports\"\n\
             include_forks = true\n\
             include_archived = true\n",
        );

        let get_env = entorno(&[(HOME, "/home/pruebas")]);
        let config = config::resolve(&ConfigFlags::default(), &get_env, Some(&ruta));

        assert_eq!(config.valet_base_url, "http://fichero:1111");
        assert_eq!(
            config
                .valet_ingest_token
                .as_ref()
                .map(ghwatch::notify::IngestToken::expose_secret),
            Some("val_del_fichero")
        );
        assert_eq!(
            config
                .github_token
                .as_ref()
                .map(ghwatch::auth::Token::expose_secret),
            Some("ghp_del_fichero")
        );
        assert_eq!(config.concurrency, 3);
        assert_eq!(config.db_path, PathBuf::from("/fichero.db"));
        assert_eq!(config.report_dir, PathBuf::from("/fichero-reports"));
        assert!(config.include_forks);
        assert!(config.include_archived);
    }

    // --- Permisos del fichero ---

    /// Un fichero con permisos `644` se rechaza con aviso y su contenido no se
    /// usa: ni la URL ni, sobre todo, los tokens.
    #[cfg(unix)]
    #[test]
    fn un_fichero_con_permisos_laxos_se_rechaza() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(
            &directorio,
            0o644,
            "valet_base_url = \"http://fichero:1111\"\n\
             github_token = \"ghp_del_fichero\"\n\
             valet_ingest_token = \"val_del_fichero\"\n",
        );

        let get_env = entorno(&[(HOME, "/home/pruebas")]);
        let config = config::resolve(&ConfigFlags::default(), &get_env, Some(&ruta));

        assert_eq!(
            config.valet_base_url, DEFAULT_BASE_URL,
            "la URL de un fichero laxo no se usa"
        );
        assert!(
            config.github_token.is_none(),
            "el token de GitHub de un fichero laxo no se usa"
        );
        assert!(
            config.valet_ingest_token.is_none(),
            "el token de ingesta de un fichero laxo no se usa"
        );
    }

    /// Un fichero ausente o con TOML inválido se ignora sin propagar error.
    #[cfg(unix)]
    #[test]
    fn un_fichero_ausente_o_invalido_se_ignora() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let get_env = entorno(&[(HOME, "/home/pruebas")]);

        let ausente = config::resolve(
            &ConfigFlags::default(),
            &get_env,
            Some(&ruta_inexistente(&directorio)),
        );
        assert_eq!(ausente.valet_base_url, DEFAULT_BASE_URL);
        assert!(ausente.github_token.is_none());

        let invalido = escribir_config(&directorio, 0o600, "esto no es = = toml válido\n");
        let config = config::resolve(&ConfigFlags::default(), &get_env, Some(&invalido));
        assert_eq!(config.valet_base_url, DEFAULT_BASE_URL);
        assert!(config.github_token.is_none());
    }

    // --- Valores por defecto ---

    /// Sin flags, entorno ni fichero, la base de datos y los informes cuelgan
    /// de `$HOME/.local/share/ghwatch/`.
    #[test]
    fn los_valores_por_defecto_cuelgan_de_home() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let get_env = entorno(&[(HOME, "/home/pruebas")]);

        let config = config::resolve(
            &ConfigFlags::default(),
            &get_env,
            Some(&ruta_inexistente(&directorio)),
        );

        assert_eq!(
            config.db_path,
            PathBuf::from("/home/pruebas/.local/share/ghwatch/ghwatch.db")
        );
        assert_eq!(
            config.report_dir,
            PathBuf::from("/home/pruebas/.local/share/ghwatch/reports")
        );
        assert_eq!(config.valet_base_url, DEFAULT_BASE_URL);
        assert_eq!(config.concurrency, DEFAULT_CONCURRENCY);
        assert!(!config.include_forks);
        assert!(!config.include_archived);
        assert_eq!(
            config.github_base_url,
            ghwatch::github::client::DEFAULT_BASE_URL
        );
        assert_eq!(config.osv_base_url, "https://api.osv.dev");
    }

    // --- Tokens ---

    /// Los tokens se resuelven del entorno y, si faltan, del fichero.
    #[test]
    fn los_tokens_se_resuelven_de_entorno_o_fichero() {
        let directorio = tempfile::tempdir().expect("directorio temporal");

        let get_env = entorno(&[
            (HOME, "/home/pruebas"),
            (GITHUB_TOKEN_ENV, "ghp_del_entorno"),
            (ENV_INGEST_TOKEN, "val_del_entorno"),
        ]);
        let config = config::resolve(
            &ConfigFlags::default(),
            &get_env,
            Some(&ruta_inexistente(&directorio)),
        );
        assert_eq!(
            config
                .github_token
                .as_ref()
                .map(ghwatch::auth::Token::expose_secret),
            Some("ghp_del_entorno")
        );
        assert_eq!(
            config
                .valet_ingest_token
                .as_ref()
                .map(ghwatch::notify::IngestToken::expose_secret),
            Some("val_del_entorno")
        );

        let vacio = entorno(&[(HOME, "/home/pruebas")]);
        let config = config::resolve(
            &ConfigFlags::default(),
            &vacio,
            Some(&ruta_inexistente(&directorio)),
        );
        assert!(config.github_token.is_none());
        assert!(config.valet_ingest_token.is_none());
    }

    // --- Booleanos ---

    /// Los booleanos se combinan: el fichero y el entorno los activan y el flag
    /// también, sin que un `false` implícito los desactive.
    #[cfg(unix)]
    #[test]
    fn los_booleanos_se_combinan() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(&directorio, 0o600, "include_forks = true\n");

        let get_env = entorno(&[(HOME, "/home/pruebas"), (ENV_INCLUDE_ARCHIVED, "true")]);
        let config = config::resolve(&ConfigFlags::default(), &get_env, Some(&ruta));

        assert!(config.include_forks, "lo activa el fichero");
        assert!(config.include_archived, "lo activa el entorno");

        let flags = ConfigFlags {
            include_forks: true,
            ..ConfigFlags::default()
        };
        let get_env = entorno(&[(HOME, "/home/pruebas"), (ENV_INCLUDE_FORKS, "false")]);
        let config = config::resolve(&flags, &get_env, Some(&ruta_inexistente(&directorio)));
        assert!(config.include_forks, "el flag sigue ganando");
        assert!(!config.include_archived);
    }

    /// `default_db_path`/`default_report_dir` calculan la ruta a partir del
    /// `HOME` inyectado, sin leer el entorno del proceso.
    #[test]
    fn las_rutas_por_defecto_se_calculan_desde_el_entorno_inyectado() {
        let get_env = entorno(&[(HOME, "/home/otro")]);
        assert_eq!(
            config::default_db_path(&get_env),
            Path::new("/home/otro/.local/share/ghwatch/ghwatch.db")
        );
        assert_eq!(
            config::default_report_dir(&get_env),
            Path::new("/home/otro/.local/share/ghwatch/reports")
        );
        assert_eq!(
            config::default_config_path(&get_env),
            Some(PathBuf::from("/home/otro/.config/ghwatch/config.toml"))
        );
    }

    // --- Endurecimiento tras la auditoría ---

    /// I4: las dos grafías de las claves de Valet (`valet_base_url` y
    /// `base_url`) resuelven lo mismo tanto en `ValetSettings` como en
    /// `config::resolve`. Así un usuario que escriba la grafía documentada por
    /// `notify` no ve cómo el CLI la ignora en silencio.
    #[cfg(unix)]
    #[test]
    fn las_dos_grafias_de_valet_resuelven_lo_mismo() {
        let canonica = tempfile::tempdir().expect("directorio temporal");
        let ruta_canonica = escribir_config(
            &canonica,
            0o600,
            "valet_base_url = \"http://uno:1111\"\n\
             valet_ingest_token = \"val_del_fichero\"\n",
        );
        let alias = tempfile::tempdir().expect("directorio temporal");
        let ruta_alias = escribir_config(
            &alias,
            0o600,
            "base_url = \"http://uno:1111\"\n\
             ingest_token = \"val_del_fichero\"\n",
        );

        let entorno_vacio = entorno(&[(HOME, "/home/pruebas")]);
        for ruta in [&ruta_canonica, &ruta_alias] {
            let settings = ValetSettings::from_env_or_file(|_| None, ruta);
            assert_eq!(settings.base_url, "http://uno:1111", "ruta {ruta:?}");
            assert_eq!(
                settings
                    .token
                    .as_ref()
                    .map(ghwatch::notify::IngestToken::expose_secret),
                Some("val_del_fichero"),
                "ruta {ruta:?}"
            );

            let config = config::resolve(&ConfigFlags::default(), &entorno_vacio, Some(ruta));
            assert_eq!(config.valet_base_url, "http://uno:1111", "ruta {ruta:?}");
            assert_eq!(
                config
                    .valet_ingest_token
                    .as_ref()
                    .map(ghwatch::notify::IngestToken::expose_secret),
                Some("val_del_fichero"),
                "ruta {ruta:?}"
            );
        }
    }

    /// I5: `~` y `~/…` se expanden contra el `HOME` inyectado al resolver las
    /// rutas, en lugar de crear un directorio literal `~` bajo el CWD.
    #[cfg(unix)]
    #[test]
    fn la_tilde_se_expande_contra_el_home_inyectado() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(
            &directorio,
            0o600,
            "db_path = \"~/datos/ghwatch.db\"\n\
             report_dir = \"~\"\n",
        );

        let get_env = entorno(&[(HOME, "/home/pruebas")]);
        let config = config::resolve(&ConfigFlags::default(), &get_env, Some(&ruta));

        assert_eq!(
            config.db_path,
            PathBuf::from("/home/pruebas/datos/ghwatch.db")
        );
        assert_eq!(config.report_dir, PathBuf::from("/home/pruebas"));

        // Un flag con `~` también se expande.
        let flags = ConfigFlags {
            db_path: Some(PathBuf::from("~/otra.db")),
            ..ConfigFlags::default()
        };
        let config = config::resolve(&flags, &get_env, None);
        assert_eq!(config.db_path, PathBuf::from("/home/pruebas/otra.db"));
    }

    /// M13: un `ENV=false` explícito gana al `include_forks = true` del fichero
    /// (precedencia entorno > fichero).
    #[cfg(unix)]
    #[test]
    fn el_entorno_en_false_gana_al_fichero_en_true() {
        let directorio = tempfile::tempdir().expect("directorio temporal");
        let ruta = escribir_config(&directorio, 0o600, "include_forks = true\n");
        let get_env = entorno(&[(HOME, "/home/pruebas"), (ENV_INCLUDE_FORKS, "false")]);
        let config = config::resolve(&ConfigFlags::default(), &get_env, Some(&ruta));
        assert!(!config.include_forks, "el entorno false tiene prioridad");
    }
}
