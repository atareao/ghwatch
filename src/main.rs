//! Punto de entrada del binario `ghwatch`.
//!
//! Solo se encarga de tres cosas: configurar el registro de logs, parsear los
//! argumentos con [`clap`] y delegar en [`ghwatch::cli::run`]. No contiene lógica
//! de negocio: toda la ejecución vive en la biblioteca y recibe su entorno por
//! inyección.

use std::io::IsTerminal;

use clap::Parser;
use ghwatch::cli::{Cli, Command, run};
use tracing_subscriber::EnvFilter;

/// Arranca el CLI.
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    configurar_logs(&cli);

    let codigo = run(cli).await;
    std::process::exit(i32::from(codigo));
}

/// Configura `tracing` a partir de `RUST_LOG` y de la verbosidad de `scan`.
///
/// `RUST_LOG` tiene prioridad; si no está definida, el nivel se deriva de
/// `--quiet` y de `-v` (0 = `warn`, 1 = `info`, 2 = `debug`, 3+ = `trace`).
///
/// **Los registros van siempre a `stderr`**, nunca a `stdout`: la salida
/// estándar queda reservada en exclusiva para el resultado del comando
/// (`report --format json|markdown`, `diff`, `history`) y así un consumidor
/// puede redirigirla a un fichero sin que un aviso la corrompa. El color solo
/// se activa si `stderr` es una terminal, para no ensuciar los ficheros.
fn configurar_logs(cli: &Cli) {
    let (quiet, verbose) = match &cli.command {
        Command::Scan(args) => (args.quiet, args.verbose),
        _ => (false, 0),
    };
    let por_defecto = if quiet {
        "error".to_string()
    } else {
        match verbose {
            0 => "warn".to_string(),
            1 => "info".to_string(),
            2 => "debug".to_string(),
            _ => "trace".to_string(),
        }
    };
    let filtro = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(por_defecto));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filtro)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .try_init();
}
