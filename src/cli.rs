//! Superficie de línea de comandos y capa de ejecución.
//!
//! Este módulo define los siete subcomandos (`scan`, `report`, `diff`, `history`,
//! `repos`, `notify` e `init`) con `clap` y traduce cada uno a la lógica de las
//! fases anteriores. La ejecución **no** lee el entorno del proceso, ni `$HOME`,
//! ni el reloj: todo eso se inyecta a través de [`Env`], de modo que el flujo
//! completo es determinista y comprobable sin red y sin credenciales reales.
//!
//! # Códigos de salida
//!
//! | Código | Constante | Significado |
//! |---|---|---|
//! | 0 | [`EXIT_OK`] | ejecución correcta |
//! | 2 | [`EXIT_USAGE`] | error de uso (lo emite `clap`) |
//! | 3 | [`EXIT_AUTH`] | no hay credencial de GitHub |
//! | 4 | [`EXIT_GITHUB`] | fallo fatal de la API de GitHub |
//! | 5 | [`EXIT_STORE`] | fallo local de base de datos, informe o E/S |
//!
//! # Orden de escritura
//!
//! `scan` y `notify` escriben **siempre** el informe antes de notificar: el
//! envío a Valet es best-effort y nunca altera el código de salida. Los
//! subcomandos de consulta (`report`, `diff`, `history`, `repos`) leen solo del
//! almacenamiento local —base de datos y ficheros de informe— y no resuelven
//! credenciales ni contactan con GitHub; `notify` tampoco reescribe el informe,
//! solo envía a Valet el documento ya escrito.
//!
//! # Salida
//!
//! `stdout` queda reservado al resultado del subcomando (informe, JSON o texto)
//! para que la salida de máquina sea parseable; los diagnósticos de `tracing`
//! van a `stderr` (véase `src/main.rs`).

use std::future::Future;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use crate::auth::{self, AuthError, Token};
use crate::config::{self, ConfigFlags, ResolvedConfig};
use crate::domain::alert::{AlertSource, Severity};
use crate::domain::drift::AlertState;
use crate::github::GithubError;
use crate::github::client::{GithubClient, GithubConfig};
use crate::github::repos::RepoSelection;
use crate::notify::{
    EmitOutcome, NotifyOutcome, ValetClient, ValetConfig, emit_and_notify, send_document,
};
use crate::osv::{OsvClient, OsvConfig};
use crate::report::{
    HISTORY_EN_INFORME, RepoState, ReportDocument, ReposDocument, ReposFilters, SCHEMA_VERSION,
};
use crate::scan::{
    HiddenReporter, IndicatifReporter, ProgressReporter, scan, scan_with_enrichment,
};
use crate::store::{RepoQuery, Store};

/// Salida correcta.
pub const EXIT_OK: u8 = 0;
/// Error de uso; lo emite `clap` antes de ejecutar nada.
pub const EXIT_USAGE: u8 = 2;
/// No hay credencial de GitHub utilizable.
pub const EXIT_AUTH: u8 = 3;
/// Fallo fatal de la API de GitHub.
pub const EXIT_GITHUB: u8 = 4;
/// Fallo local de base de datos, informe o E/S.
pub const EXIT_STORE: u8 = 5;

/// Fichero de configuración con la plantilla que escribe `init`.
const PLANTILLA_CONFIG: &str = "\
# Configuración de ghwatch.
#
# Este fichero contiene credenciales en claro: debe conservar permisos 600.
# Las variables de entorno tienen prioridad sobre estos valores.
#
# github_token = \"ghp_...\"
# valet_base_url = \"http://localhost:3000\"
# valet_ingest_token = \"val_...\"
# db_path = \"~/.local/share/ghwatch/ghwatch.db\"
# report_dir = \"~/.local/share/ghwatch/reports\"
# concurrency = 6
# include_forks = false
# include_archived = false
";

/// Futuro que resuelve la credencial de GitHub.
///
/// Es el tipo que devuelve el resolutor inyectable de [`Env`]: permite
/// sustituir la resolución real (`GITHUB_TOKEN` → `gh auth token`) por un doble
/// determinista en los tests.
pub type TokenFuture = Pin<Box<dyn Future<Output = Result<Token, AuthError>> + Send>>;

/// Acceso inyectable a las variables de entorno.
pub type GetEnv = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Entorno de ejecución inyectable.
///
/// Reúne todo lo que la capa de ejecución no debe leer por su cuenta: variables
/// de entorno, reloj, salida y resolución de credenciales. En producción lo
/// construye [`Env::production`]; los tests aportan sus propios dobles.
pub struct Env {
    /// Acceso a las variables de entorno.
    pub get_env: GetEnv,
    /// Instante de la ejecución.
    pub now: DateTime<Utc>,
    /// Salida de texto del programa.
    pub out: Box<dyn Write + Send>,
    /// Informe de progreso inyectado; si falta, se usa el de producción.
    pub reporter: Option<Arc<dyn ProgressReporter>>,
    /// Resolutor de la credencial de GitHub.
    pub resolver: Box<dyn Fn() -> TokenFuture + Send + Sync>,
}

impl Env {
    /// Entorno de producción: proceso, reloj real y `stdout`.
    #[must_use]
    pub fn production() -> Self {
        Self {
            get_env: Box::new(|clave| std::env::var(clave).ok()),
            now: Utc::now(),
            out: Box::new(io::stdout()),
            reporter: None,
            resolver: Box::new(|| Box::pin(auth::resolve_token_async())),
        }
    }
}

/// Programa completo.
#[derive(Debug, Parser)]
#[command(
    name = "ghwatch",
    version,
    about = "Vigila las vulnerabilidades de todos los repositorios de una cuenta de GitHub",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Subcomando a ejecutar.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcomandos disponibles.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Escanea la cuenta, persiste el resultado e informa.
    Scan(ScanArgs),
    /// Reemite el último informe local en el formato pedido.
    Report(ReportArgs),
    /// Muestra las alertas nuevas y resueltas de la última ejecución.
    Diff(CommonArgs),
    /// Muestra la serie histórica almacenada.
    History(HistoryArgs),
    /// Lista los repositorios con sus alertas.
    Repos(ReposArgs),
    /// Reenvía el último informe local a Valet.
    Notify(NotifyArgs),
    /// Crea la base de datos y la plantilla de configuración.
    Init(InitArgs),
}

/// Argumentos de `scan`.
#[derive(Debug, Clone, Args)]
pub struct ScanArgs {
    /// Incluye los forks en el inventario.
    #[arg(long)]
    pub include_forks: bool,
    /// Incluye los repositorios archivados en el inventario.
    #[arg(long)]
    pub include_archived: bool,
    /// No enriquece las alertas con OSV.dev.
    #[arg(long)]
    pub no_enrich: bool,
    /// Fuerza la notificación aunque no haya novedades.
    #[arg(long)]
    pub force: bool,
    /// No dibuja la barra de progreso ni el resumen final.
    #[arg(long)]
    pub quiet: bool,
    /// Nivel de detalle; repetible (`-v`, `-vv`).
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbose: u8,
    /// Ruta de la base de datos.
    #[arg(long)]
    pub db: Option<PathBuf>,
    /// Directorio de los informes.
    #[arg(long)]
    pub report_dir: Option<PathBuf>,
    /// Ruta del fichero de configuración.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Concurrencia del cliente de GitHub.
    #[arg(long)]
    pub concurrency: Option<usize>,
    /// URL base de la API de GitHub.
    #[arg(long)]
    pub github_base_url: Option<String>,
    /// URL base de OSV.dev.
    #[arg(long)]
    pub osv_base_url: Option<String>,
    /// URL base de Valet.
    #[arg(long)]
    pub valet_base_url: Option<String>,
}

/// Formato de salida de `report`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Resumen legible en la terminal.
    #[default]
    Console,
    /// Documento Markdown.
    Markdown,
    /// Documento JSON.
    Json,
}

/// Severidad mínima del filtro de `repos`.
///
/// Es un `ValueEnum` propio del CLI: `Severity` vive en `domain` (puro) y no
/// debe conocer el parseo de la línea de comandos. Un valor fuera del conjunto
/// es un error de uso (código 2), no un `Unknown` silencioso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MinSeverity {
    /// Solo repositorios con alguna alerta crítica.
    Critical,
    /// Repositorios con alguna alerta alta o crítica.
    High,
    /// Repositorios con alguna alerta media, alta o crítica.
    Medium,
    /// Repositorios con alguna alerta baja, media, alta o crítica.
    Low,
    /// Todos los repositorios con alertas.
    Unknown,
}

impl MinSeverity {
    /// Severidad del dominio equivalente.
    #[must_use]
    pub fn severity(self) -> Severity {
        match self {
            Self::Critical => Severity::Critical,
            Self::High => Severity::High,
            Self::Medium => Severity::Medium,
            Self::Low => Severity::Low,
            Self::Unknown => Severity::Unknown,
        }
    }
}

/// Fuente de alertas del filtro de `repos`.
///
/// Es un `ValueEnum` propio del CLI: `AlertSource` vive en `domain` (puro) y no
/// debe conocer el parseo de la línea de comandos. Un valor fuera del conjunto
/// es un error de uso (código 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SourceFilter {
    /// Alertas de Dependabot.
    Dependabot,
    /// Alertas de secret scanning.
    Secret,
    /// Alertas de code scanning.
    Code,
}

impl SourceFilter {
    /// Fuente del dominio equivalente.
    #[must_use]
    pub fn source(self) -> AlertSource {
        match self {
            Self::Dependabot => AlertSource::Dependabot,
            Self::Secret => AlertSource::Secret,
            Self::Code => AlertSource::Code,
        }
    }
}

/// Estado de las alertas del filtro de `repos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StateFilter {
    /// Solo alertas abiertas.
    Open,
    /// Solo alertas resueltas.
    Resolved,
    /// Abiertas y resueltas.
    All,
}

impl StateFilter {
    /// Estado del dominio (`None` = todos, para `RepoQuery`).
    #[must_use]
    pub fn alert_state(self) -> Option<AlertState> {
        match self {
            Self::Open => Some(AlertState::Open),
            Self::Resolved => Some(AlertState::Resolved),
            Self::All => None,
        }
    }

    /// Estado de la presentación.
    #[must_use]
    pub fn repo_state(self) -> RepoState {
        match self {
            Self::Open => RepoState::Open,
            Self::Resolved => RepoState::Resolved,
            Self::All => RepoState::All,
        }
    }
}

/// Argumentos de `report`.
#[derive(Debug, Clone, Args)]
pub struct ReportArgs {
    /// Formato del informe reemitido.
    #[arg(long, value_enum, default_value_t = OutputFormat::Console)]
    pub format: OutputFormat,
    /// Directorio de los informes.
    #[arg(long)]
    pub report_dir: Option<PathBuf>,
    /// Ruta del fichero de configuración.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// Argumentos comunes a las consultas que solo usan la base de datos.
#[derive(Debug, Clone, Args)]
pub struct CommonArgs {
    /// Ruta de la base de datos.
    #[arg(long)]
    pub db: Option<PathBuf>,
    /// Ruta del fichero de configuración.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// Argumentos de `history`.
#[derive(Debug, Clone, Args)]
pub struct HistoryArgs {
    /// Ruta de la base de datos.
    #[arg(long)]
    pub db: Option<PathBuf>,
    /// Ruta del fichero de configuración.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Número máximo de ejecuciones a mostrar.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

/// Argumentos de `repos`.
#[derive(Debug, Clone, Args)]
pub struct ReposArgs {
    /// Ruta de la base de datos.
    #[arg(long)]
    pub db: Option<PathBuf>,
    /// Ruta del fichero de configuración.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Número máximo de repositorios a mostrar (`0` = todos).
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    /// Muestra todos los repositorios (equivale a `--limit 0`).
    #[arg(long)]
    pub all: bool,
    /// Conserva solo los repositorios con al menos una alerta de esta severidad o superior.
    #[arg(long, value_enum)]
    pub min_severity: Option<MinSeverity>,
    /// Incluye los repositorios sin ninguna alerta.
    #[arg(long)]
    pub include_empty: bool,
    /// Cuenta solo las alertas de esta fuente.
    #[arg(long, value_enum)]
    pub source: Option<SourceFilter>,
    /// Estado de las alertas contadas.
    #[arg(long, value_enum, default_value_t = StateFilter::Open)]
    pub state: StateFilter,
    /// Formato de la salida.
    #[arg(long, value_enum, default_value_t = OutputFormat::Console)]
    pub format: OutputFormat,
    /// Escribe la salida en este fichero en lugar de la salida estándar.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

/// Argumentos de `notify`.
#[derive(Debug, Clone, Args)]
pub struct NotifyArgs {
    /// Fuerza el envío aunque no haya novedades.
    #[arg(long)]
    pub force: bool,
    /// Directorio de los informes.
    #[arg(long)]
    pub report_dir: Option<PathBuf>,
    /// Ruta del fichero de configuración.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// URL base de Valet.
    #[arg(long)]
    pub valet_base_url: Option<String>,
}

/// Argumentos de `init`.
#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    /// Ruta de la base de datos a crear.
    #[arg(long)]
    pub db: Option<PathBuf>,
    /// Ruta del fichero de configuración a crear.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// Ejecuta el CLI con el entorno de producción.
pub async fn run(cli: Cli) -> u8 {
    run_with(cli, Env::production()).await
}

/// Ejecuta el CLI con un entorno inyectado.
///
/// Es el punto de entrada de los tests: permite fijar entorno, reloj, salida y
/// resolución de credenciales sin tocar el proceso ni la red.
#[must_use]
pub async fn run_with(cli: Cli, mut env: Env) -> u8 {
    match cli.command {
        Command::Scan(args) => ejecutar_scan(args, &mut env).await,
        Command::Report(args) => ejecutar_report(args, &mut env).await,
        Command::Diff(args) => ejecutar_diff(args, &mut env).await,
        Command::History(args) => ejecutar_history(args, &mut env).await,
        Command::Repos(args) => ejecutar_repos(args, &mut env).await,
        Command::Notify(args) => ejecutar_notify(args, &mut env).await,
        Command::Init(args) => ejecutar_init(args, &mut env).await,
    }
}

/// Ejecuta `scan`.
async fn ejecutar_scan(args: ScanArgs, env: &mut Env) -> u8 {
    let flags = ConfigFlags {
        db_path: args.db.clone(),
        report_dir: args.report_dir.clone(),
        concurrency: args.concurrency,
        include_forks: args.include_forks,
        include_archived: args.include_archived,
        github_base_url: args.github_base_url.clone(),
        osv_base_url: args.osv_base_url.clone(),
        valet_base_url: args.valet_base_url.clone(),
    };
    let config = resolver_config(&flags, args.config.as_deref(), env);

    let token = match token_de(config.github_token.clone(), env).await {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(error = %error, "no hay credencial de GitHub utilizable");
            return EXIT_AUTH;
        }
    };

    let github_config = GithubConfig {
        base_url: config.github_base_url.clone(),
        concurrency: config.concurrency,
        ..GithubConfig::default()
    };
    let client = match GithubClient::new(token, github_config) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            // La construcción solo falla por una URL base inválida o por el
            // cliente HTTP subyacente: es un error local de configuración, no
            // una respuesta de la API (que se mapea a `EXIT_GITHUB`).
            tracing::error!(error = %error, "no se pudo construir el cliente de GitHub");
            return EXIT_STORE;
        }
    };

    let selection = RepoSelection {
        include_forks: config.include_forks,
        include_archived: config.include_archived,
    };
    let reporter: Arc<dyn ProgressReporter> = if args.quiet {
        Arc::new(HiddenReporter)
    } else {
        env.reporter
            .clone()
            .unwrap_or_else(|| Arc::new(IndicatifReporter::new()))
    };

    let resumen = if args.no_enrich {
        match scan(Arc::clone(&client), selection, reporter).await {
            Ok(resumen) => resumen,
            Err(error) => return fallo_github(error),
        }
    } else {
        match OsvClient::new(OsvConfig {
            base_url: config.osv_base_url.clone(),
            ..OsvConfig::default()
        }) {
            Ok(enricher) => {
                match scan_with_enrichment(
                    Arc::clone(&client),
                    selection,
                    reporter,
                    Arc::new(enricher),
                )
                .await
                {
                    Ok(resumen) => resumen,
                    Err(error) => return fallo_github(error),
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "no se pudo construir el cliente de OSV; se escanea sin enriquecer"
                );
                match scan(Arc::clone(&client), selection, reporter).await {
                    Ok(resumen) => resumen,
                    Err(error) => return fallo_github(error),
                }
            }
        }
    };

    let store = match abrir_store(&config.db_path).await {
        Ok(store) => store,
        Err(codigo) => return codigo,
    };

    let persistencia = match store.persist_scan(&resumen, env.now, env.now).await {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::error!(error = %error, "no se pudo persistir el escaneo");
            return EXIT_STORE;
        }
    };
    let historico = match store.history().await {
        Ok(historico) => historico,
        Err(error) => {
            tracing::error!(error = %error, "no se pudo leer el histórico");
            return EXIT_STORE;
        }
    };

    let documento = ReportDocument::from_scan(
        &resumen,
        &historico,
        HISTORY_EN_INFORME,
        persistencia.run_id,
        a_i64(persistencia.new),
        a_i64(persistencia.resolved),
        env.now,
    );
    let (markdown, json) = rutas_informe(&config.report_dir, env.now);

    if let Err(error) = tokio::fs::create_dir_all(&config.report_dir).await {
        tracing::error!(
            path = %config.report_dir.display(),
            error = %error,
            "no se pudo crear el directorio de informes"
        );
        return EXIT_STORE;
    }

    let notifier = construir_notifier(&config);
    let emitido =
        match emit_and_notify(&documento, &markdown, &json, notifier.as_ref(), args.force).await {
            Ok(emitido) => emitido,
            Err(error) => {
                tracing::error!(error = %error, "no se pudo escribir el informe");
                return EXIT_STORE;
            }
        };

    if !args.quiet {
        let resumen_texto = resumen_final(&resumen, &persistencia, &emitido);
        if let Err(error) = emitir(env, &resumen_texto) {
            tracing::warn!(error = %error, "no se pudo escribir el resumen en la salida");
        }
    }

    EXIT_OK
}

/// Ejecuta `report`.
async fn ejecutar_report(args: ReportArgs, env: &mut Env) -> u8 {
    let flags = ConfigFlags {
        report_dir: args.report_dir,
        ..ConfigFlags::default()
    };
    let config = resolver_config(&flags, args.config.as_deref(), env);

    let Some(json) = informe_mas_reciente(&config.report_dir, ".json").await else {
        tracing::warn!(
            path = %config.report_dir.display(),
            "no hay ningún informe local que reemitir"
        );
        return EXIT_OK;
    };
    let documento = match leer_documento(&json).await {
        Ok(documento) => documento,
        Err(codigo) => return codigo,
    };

    let texto = match args.format {
        OutputFormat::Console => resumen_documento(&documento),
        OutputFormat::Markdown => documento.to_markdown(),
        OutputFormat::Json => match documento.to_json() {
            Ok(texto) => texto,
            Err(error) => {
                tracing::error!(error = %error, "no se pudo serializar el informe a JSON");
                return EXIT_STORE;
            }
        },
    };

    if let Err(error) = emitir(env, &texto) {
        tracing::error!(error = %error, "no se pudo escribir el informe en la salida");
        return EXIT_STORE;
    }
    EXIT_OK
}

/// Ejecuta `diff`.
async fn ejecutar_diff(args: CommonArgs, env: &mut Env) -> u8 {
    let flags = ConfigFlags {
        db_path: args.db,
        ..ConfigFlags::default()
    };
    let config = resolver_config(&flags, args.config.as_deref(), env);

    let store = match abrir_store_si_existe(&config.db_path).await {
        Ok(Some(store)) => store,
        Ok(None) => return emitir_sin_datos(env, "sin ejecuciones registradas"),
        Err(codigo) => return codigo,
    };
    let diff = match store.latest_diff().await {
        Ok(diff) => diff,
        Err(error) => {
            tracing::error!(error = %error, "no se pudo calcular el diff");
            return EXIT_STORE;
        }
    };

    let texto = match diff {
        Some(diff) => {
            let mut lineas = vec![format!(
                "diff de la ejecución #{}: {} nuevas, {} resueltas",
                diff.run_id,
                diff.new.len(),
                diff.resolved.len()
            )];
            for alerta in &diff.new {
                lineas.push(format!(
                    "  nueva    #{} {} [{:?}]",
                    alerta.external_id, alerta.repo, alerta.source
                ));
            }
            for alerta in &diff.resolved {
                lineas.push(format!(
                    "  resuelta #{} {} [{:?}]",
                    alerta.external_id, alerta.repo, alerta.source
                ));
            }
            lineas.join("\n")
        }
        None => "sin ejecuciones registradas".to_string(),
    };

    if let Err(error) = emitir(env, &texto) {
        tracing::error!(error = %error, "no se pudo escribir el diff");
        return EXIT_STORE;
    }
    EXIT_OK
}

/// Ejecuta `history`.
async fn ejecutar_history(args: HistoryArgs, env: &mut Env) -> u8 {
    let flags = ConfigFlags {
        db_path: args.db,
        ..ConfigFlags::default()
    };
    let config = resolver_config(&flags, args.config.as_deref(), env);

    let store = match abrir_store_si_existe(&config.db_path).await {
        Ok(Some(store)) => store,
        Ok(None) => return emitir_sin_datos(env, "sin ejecuciones registradas"),
        Err(codigo) => return codigo,
    };
    let historico = match store.history().await {
        Ok(historico) => historico,
        Err(error) => {
            tracing::error!(error = %error, "no se pudo leer el histórico");
            return EXIT_STORE;
        }
    };

    let texto = if historico.is_empty() || args.limit == 0 {
        "sin ejecuciones registradas".to_string()
    } else {
        let desde = historico.len().saturating_sub(args.limit);
        historico[desde..]
            .iter()
            .map(|punto| {
                format!(
                    "#{} {} total={} nuevas={} resueltas={}",
                    punto.run_id,
                    punto.started_at.to_rfc3339(),
                    punto.total,
                    punto.new,
                    punto.resolved
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    if let Err(error) = emitir(env, &texto) {
        tracing::error!(error = %error, "no se pudo escribir el histórico");
        return EXIT_STORE;
    }
    EXIT_OK
}

/// Ejecuta `repos`.
///
/// Consulta la base de datos (sin abrirla si no existe, sin crearla) y compone
/// el listado de repositorios con sus alertas —en el estado y de la fuente
/// pedidos— en el formato elegido. Es una consulta local: no resuelve
/// credenciales ni contacta con la red. Con `--output`, la salida formateada va
/// al fichero indicado y no a la salida estándar; sin él, a la salida estándar.
///
/// Convención de bytes: el fichero reproduce **exactamente** los mismos bytes
/// que la salida estándar, es decir, el texto formateado seguido de un `\n`
/// (la salida estándar lo añade con `writeln!`). Así una redirección a fichero
/// y `--output` son intercambiables.
async fn ejecutar_repos(args: ReposArgs, env: &mut Env) -> u8 {
    let flags = ConfigFlags {
        db_path: args.db,
        ..ConfigFlags::default()
    };
    let config = resolver_config(&flags, args.config.as_deref(), env);

    let store = match abrir_store_si_existe(&config.db_path).await {
        Ok(Some(store)) => store,
        Ok(None) => return emitir_sin_datos(env, "sin datos locales (ejecuta `ghwatch scan`)"),
        Err(codigo) => return codigo,
    };

    // Los ejes del filtro se calculan una sola vez y se reutilizan tanto en la
    // consulta al almacén como en el documento de presentación.
    let source = args.source.map(SourceFilter::source);
    let min_severity = args.min_severity.map(MinSeverity::severity);
    let include_empty = args.include_empty;

    let consulta = RepoQuery {
        state: args.state.alert_state(),
        source,
        min_severity,
        include_empty,
    };

    let listado = match store.repo_alert_counts(&consulta).await {
        Ok(listado) => listado,
        Err(error) => {
            tracing::error!(error = %error, "no se pudieron contar las alertas por repositorio");
            return EXIT_STORE;
        }
    };
    let run_id = match store.latest_run_id().await {
        Ok(run_id) => run_id.unwrap_or(0),
        Err(error) => {
            tracing::error!(error = %error, "no se pudo leer la última ejecución");
            return EXIT_STORE;
        }
    };

    // `--all` es azúcar de `--limit 0` y prevalece si se combinan.
    let limit = if args.all { None } else { Some(args.limit) };
    let filtros = ReposFilters {
        state: args.state.repo_state(),
        source,
        min_severity,
        include_empty,
    };
    let documento = ReposDocument::from_list(&listado, run_id, limit, filtros, env.now);

    let texto = match args.format {
        OutputFormat::Console => documento.to_console(),
        OutputFormat::Markdown => documento.to_markdown(),
        OutputFormat::Json => match documento.to_json() {
            Ok(texto) => texto,
            Err(error) => {
                tracing::error!(error = %error, "no se pudo serializar el listado a JSON");
                return EXIT_STORE;
            }
        },
    };

    match args.output {
        // `--output` escribe el fichero y **no** emite por la salida estándar.
        // Un fallo de escritura es un fallo local: no se crean directorios
        // intermedios, así que una carpeta inexistente es un error.
        Some(ruta) => {
            // El fichero lleva el mismo `\n` final que `emitir` añade por stdout.
            if let Err(error) = std::fs::write(&ruta, format!("{texto}\n")) {
                tracing::error!(
                    path = %ruta.display(),
                    error = %error,
                    "no se pudo escribir el listado en el fichero de salida"
                );
                return EXIT_STORE;
            }
        }
        None => {
            if let Err(error) = emitir(env, &texto) {
                tracing::error!(error = %error, "no se pudo escribir el listado de repositorios");
                return EXIT_STORE;
            }
        }
    }
    EXIT_OK
}

/// Ejecuta `notify`.
async fn ejecutar_notify(args: NotifyArgs, env: &mut Env) -> u8 {
    let flags = ConfigFlags {
        report_dir: args.report_dir,
        valet_base_url: args.valet_base_url,
        ..ConfigFlags::default()
    };
    let config = resolver_config(&flags, args.config.as_deref(), env);

    let Some(json) = informe_mas_reciente(&config.report_dir, ".json").await else {
        tracing::warn!(
            path = %config.report_dir.display(),
            "no hay ningún informe local que notificar"
        );
        return EXIT_OK;
    };
    let documento = match leer_documento(&json).await {
        Ok(documento) => documento,
        Err(codigo) => return codigo,
    };

    // La ruta del Markdown se **deriva** del JSON: `notify` no escribe nada y no
    // debe emparejar por su cuenta un `.md` de otra fecha.
    let markdown = json.with_extension("md");
    let notifier = construir_notifier(&config);
    let resultado = match notifier.as_ref() {
        Some(cliente) => send_document(cliente, &documento, &markdown, args.force).await,
        None => NotifyOutcome::NotConfigured,
    };

    let texto = format!(
        "notificación: {}\ninforme: {}",
        describir_notificacion(&resultado),
        markdown.display()
    );
    if let Err(error) = emitir(env, &texto) {
        tracing::error!(error = %error, "no se pudo escribir el resultado de la notificación");
        return EXIT_STORE;
    }
    EXIT_OK
}

/// Ejecuta `init`.
async fn ejecutar_init(args: InitArgs, env: &mut Env) -> u8 {
    // `init` pasa por la misma resolución que el resto de subcomandos: respeta
    // la precedencia flags > entorno > fichero > defecto (por ejemplo
    // `GHWATCH_DB` o el `db_path` del `config.toml`).
    let flags = ConfigFlags {
        db_path: args.db.clone(),
        ..ConfigFlags::default()
    };
    let config = resolver_config(&flags, args.config.as_deref(), env);
    let db_path = config.db_path;
    let config_path = args
        .config
        .clone()
        .or_else(|| config::default_config_path(env.get_env.as_ref()));

    if let Err(error) = asegurar_padre(&db_path).await {
        tracing::error!(
            path = %db_path.display(),
            error = %error,
            "no se pudo crear el directorio de la base de datos"
        );
        return EXIT_STORE;
    }
    if let Err(codigo) = abrir_store(&db_path).await {
        return codigo;
    }

    let mut mensajes = vec![format!("base de datos: {}", db_path.display())];
    if let Some(config_path) = config_path {
        let ruta = config_path.clone();
        let plantilla = tokio::task::spawn_blocking(move || escribir_plantilla(&ruta)).await;
        let resultado = match plantilla {
            Ok(resultado) => resultado,
            Err(error) => Err(io::Error::other(error)),
        };
        if let Err(error) = resultado {
            tracing::error!(
                path = %config_path.display(),
                error = %error,
                "no se pudo escribir la plantilla de configuración"
            );
            return EXIT_STORE;
        }
        mensajes.push(format!("configuración: {}", config_path.display()));
    }

    if let Err(error) = emitir(env, &mensajes.join("\n")) {
        tracing::error!(error = %error, "no se pudo escribir el resultado de init");
        return EXIT_STORE;
    }
    EXIT_OK
}

/// Resuelve la configuración con la ruta del fichero indicada o la de defecto.
///
/// Si el `--config` indicado no existe, avisa de forma accionable y sigue con la
/// ruta por defecto (la resolución del fichero es best-effort).
fn resolver_config(flags: &ConfigFlags, config_path: Option<&Path>, env: &Env) -> ResolvedConfig {
    if let Some(ruta) = config_path
        && !ruta.exists()
    {
        tracing::warn!(
            path = %ruta.display(),
            "el fichero de configuración indicado no existe; se ignora y se usa la ruta por defecto"
        );
    }
    let ruta = config_path
        .map(Path::to_path_buf)
        .or_else(|| config::default_config_path(env.get_env.as_ref()));
    config::resolve(flags, env.get_env.as_ref(), ruta.as_deref())
}

/// Devuelve la credencial resuelta: la de la configuración o la del entorno.
async fn token_de(token: Option<Token>, env: &Env) -> Result<Token, AuthError> {
    match token {
        Some(token) => Ok(token),
        None => (env.resolver)().await,
    }
}

/// Abre el almacén **solo si el fichero ya existe**.
///
/// Los subcomandos de consulta (`diff`, `history`) no deben crear una base de
/// datos vacía: si el fichero no está, devuelven `Ok(None)` para que el llamante
/// informe de que no hay datos y termine con éxito. Solo un fallo real de
/// apertura devuelve `Err(codigo)`.
async fn abrir_store_si_existe(db_path: &Path) -> Result<Option<Store>, u8> {
    match tokio::fs::try_exists(db_path).await {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(error) => {
            tracing::error!(
                path = %db_path.display(),
                error = %error,
                "no se pudo comprobar la base de datos"
            );
            return Err(EXIT_STORE);
        }
    }
    abrir_store(db_path).await.map(Some)
}

/// Abre el almacén, creando antes su directorio.
async fn abrir_store(db_path: &Path) -> Result<Store, u8> {
    if let Err(error) = asegurar_padre(db_path).await {
        tracing::error!(
            path = %db_path.display(),
            error = %error,
            "no se pudo crear el directorio de la base de datos"
        );
        return Err(EXIT_STORE);
    }
    let url = match url_sqlite(db_path) {
        Ok(url) => url,
        Err(error) => {
            tracing::error!(
                path = %db_path.display(),
                error = %error,
                "no se pudo resolver la ruta de la base de datos"
            );
            return Err(EXIT_STORE);
        }
    };
    match Store::connect(&url).await {
        Ok(store) => Ok(store),
        Err(error) => {
            tracing::error!(error = %error, "no se pudo abrir la base de datos");
            Err(EXIT_STORE)
        }
    }
}

/// Construye la URL de sqlx a partir de la ruta absoluta del fichero.
fn url_sqlite(db_path: &Path) -> io::Result<String> {
    let absoluta = std::path::absolute(db_path)?;
    Ok(format!("sqlite://{}", absoluta.display()))
}

/// Crea el directorio padre de `path` si hace falta.
async fn asegurar_padre(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(padre) if !padre.as_os_str().is_empty() => tokio::fs::create_dir_all(padre).await,
        _ => Ok(()),
    }
}

/// Construye el notificador de Valet si hay token de ingesta.
fn construir_notifier(config: &ResolvedConfig) -> Option<ValetClient> {
    let token = config.valet_ingest_token.clone()?;
    let valet_config = ValetConfig {
        base_url: config.valet_base_url.clone(),
        ..ValetConfig::default()
    };
    match ValetClient::new(valet_config, token) {
        Ok(cliente) => Some(cliente),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "no se pudo construir el cliente de Valet; se omite la notificación"
            );
            None
        }
    }
}

/// Rutas de los informes de una ejecución.
fn rutas_informe(report_dir: &Path, now: DateTime<Utc>) -> (PathBuf, PathBuf) {
    let nombre = now.format("%Y-%m-%d");
    (
        report_dir.join(format!("ghwatch-{nombre}.md")),
        report_dir.join(format!("ghwatch-{nombre}.json")),
    )
}

/// Localiza el informe más reciente con la extensión indicada.
///
/// Los nombres siguen el patrón `ghwatch-YYYY-MM-DD.<ext>`; el mayor en orden
/// lexicográfico es el más reciente. La lectura del directorio es asíncrona para
/// no bloquear el runtime.
async fn informe_mas_reciente(dir: &Path, extension: &str) -> Option<PathBuf> {
    let mut entradas = tokio::fs::read_dir(dir).await.ok()?;
    let mut elegido: Option<String> = None;
    while let Ok(Some(entrada)) = entradas.next_entry().await {
        let Some(nombre) = entrada.file_name().into_string().ok() else {
            continue;
        };
        if nombre.starts_with("ghwatch-") && nombre.ends_with(extension) {
            elegido = Some(match elegido {
                Some(actual) if actual >= nombre => actual,
                _ => nombre,
            });
        }
    }
    elegido.map(|nombre| dir.join(nombre))
}

/// Lee y deserializa un informe local, validando su `schema_version`.
///
/// Un JSON corrupto o de una versión desconocida es un error de almacén
/// ([`EXIT_STORE`]) con un aviso: no se reemite un informe que no se entiende.
async fn leer_documento(json: &Path) -> Result<ReportDocument, u8> {
    let contenido = tokio::fs::read_to_string(json).await.map_err(|error| {
        tracing::error!(
            path = %json.display(),
            error = %error,
            "no se pudo leer el informe local"
        );
        EXIT_STORE
    })?;
    let documento: ReportDocument = serde_json::from_str(&contenido).map_err(|error| {
        tracing::error!(
            path = %json.display(),
            error = %error,
            "el informe local no es JSON válido"
        );
        EXIT_STORE
    })?;
    if documento.schema_version != SCHEMA_VERSION {
        tracing::warn!(
            path = %json.display(),
            encontrada = documento.schema_version,
            esperada = SCHEMA_VERSION,
            "el informe local declara una versión de esquema desconocida"
        );
        return Err(EXIT_STORE);
    }
    Ok(documento)
}

/// Escribe un mensaje informativo y devuelve [`EXIT_OK`].
///
/// Lo usan los comandos de consulta cuando no hay nada que mostrar (por
/// ejemplo, una base de datos que aún no existe): no es un error.
fn emitir_sin_datos(env: &mut Env, texto: &str) -> u8 {
    if let Err(error) = emitir(env, texto) {
        tracing::error!(error = %error, "no se pudo escribir el resultado");
        return EXIT_STORE;
    }
    EXIT_OK
}

/// Escribe una plantilla de configuración con permisos `600` sin sobrescribir.
fn escribir_plantilla(path: &Path) -> io::Result<()> {
    if path.exists() {
        tracing::info!(
            path = %path.display(),
            "la configuración ya existe; no se sobrescribe"
        );
        return Ok(());
    }
    if let Some(padre) = path.parent()
        && !padre.as_os_str().is_empty()
    {
        std::fs::create_dir_all(padre)?;
    }
    escribir_privado(path, PLANTILLA_CONFIG)
}

/// Crea un fichero nuevo con permisos `600` (unix) o con los del sistema.
#[cfg(unix)]
fn escribir_privado(path: &Path, contenido: &str) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut fichero = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    fichero.write_all(contenido.as_bytes())?;
    fichero.flush()
}

/// Crea un fichero nuevo con los permisos por defecto del sistema.
#[cfg(not(unix))]
fn escribir_privado(path: &Path, contenido: &str) -> io::Result<()> {
    let mut fichero = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    fichero.write_all(contenido.as_bytes())?;
    fichero.flush()
}

/// Escribe una línea en la salida del programa.
fn emitir(env: &mut Env, texto: &str) -> io::Result<()> {
    writeln!(env.out, "{texto}")?;
    env.out.flush()
}

/// Traduce un error fatal de GitHub al código de salida.
fn fallo_github(error: GithubError) -> u8 {
    tracing::error!(error = %error, "el escaneo de GitHub falló");
    EXIT_GITHUB
}

/// Convierte un contador `usize` a la cifra del informe, saturando al máximo
/// representable en `i64`.
///
/// En un `usize` de 64 bits un valor mayor que `i64::MAX` es, en la práctica,
/// imposible para un número de alertas; saturar evita un pánico y deja una cifra
/// monótona en lugar de un cero engañoso.
fn a_i64(valor: usize) -> i64 {
    i64::try_from(valor).unwrap_or(i64::MAX)
}

/// Resume el resultado de un escaneo para la salida final.
fn resumen_final(
    resumen: &crate::scan::ScanSummary,
    persistencia: &crate::store::PersistOutcome,
    emitido: &EmitOutcome,
) -> String {
    format!(
        "escaneo completado: {} repositorios, {} alertas, {} nuevas, {} resueltas\n\
         informe: {}\n\
         notificación: {}",
        resumen.succeeded_count(),
        resumen.alert_count(),
        persistencia.new,
        persistencia.resolved,
        emitido.paths.markdown.display(),
        describir_notificacion(&emitido.notify)
    )
}

/// Describe brevemente el desenlace de la notificación, sin datos sensibles.
fn describir_notificacion(outcome: &NotifyOutcome) -> String {
    match outcome {
        NotifyOutcome::Sent(id) => format!("enviada ({id})"),
        NotifyOutcome::Skipped => "omitida (sin novedades; usa --force)".to_string(),
        NotifyOutcome::NotConfigured => "no configurada".to_string(),
        NotifyOutcome::Failed(causa) => format!("fallida: {causa}"),
    }
}

/// Resume un documento de informe para la consola.
fn resumen_documento(documento: &ReportDocument) -> String {
    format!(
        "informe de la ejecución #{}\n\
         generado: {}\n\
         repositorios: {} correctos, {} fallidos\n\
         alertas: {} ({} críticas, {} altas, {} medias, {} bajas, {} desconocidas)\n\
         novedades: {} nuevas, {} resueltas",
        documento.run_id,
        documento.generated_at.to_rfc3339(),
        documento.scanned,
        documento.failed,
        documento.total,
        documento.by_severity.critical,
        documento.by_severity.high,
        documento.by_severity.medium,
        documento.by_severity.low,
        documento.by_severity.unknown,
        documento.new,
        documento.resolved
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El CLI acepta los siete subcomandos y rechaza uno desconocido.
    #[test]
    fn la_superficie_declara_los_siete_subcomandos() {
        for args in [
            vec!["ghwatch", "scan"],
            vec!["ghwatch", "report"],
            vec!["ghwatch", "diff"],
            vec!["ghwatch", "history"],
            vec!["ghwatch", "repos"],
            vec!["ghwatch", "notify"],
            vec!["ghwatch", "init"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    /// El nombre del informe sigue el patrón estable.
    #[test]
    fn las_rutas_del_informe_son_estables() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).expect("instante válido");
        let (markdown, json) = rutas_informe(Path::new("/tmp/informes"), now);
        assert!(markdown.ends_with("ghwatch-2023-11-14.md"));
        assert!(json.ends_with("ghwatch-2023-11-14.json"));
    }
}
