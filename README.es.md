# ghwatch

**Español** · [English](README.md)

[![CI](https://github.com/atareao/ghwatch/actions/workflows/ci.yml/badge.svg)](https://github.com/atareao/ghwatch/actions/workflows/ci.yml)
[![Licencia: MIT](https://img.shields.io/badge/licencia-MIT-blue.svg)](https://github.com/atareao/ghwatch/blob/main/LICENSE)

`ghwatch` es un CLI en Rust que vigila las vulnerabilidades de **todos** los
repositorios GitHub de una cuenta, para que no tengas que abrirlos uno a uno.

Recoge las **alertas de Dependabot**, las de **secret scanning** y las de **code
scanning**; las enriquece con [OSV.dev](https://osv.dev) (puntuación CVSS, alias
CVE y versión que corrige); mantiene un **histórico en SQLite** entre
ejecuciones; produce un **informe de consola, Markdown y JSON** que siempre se
escribe a disco; y, opcionalmente, **notifica a [Valet](https://github.com/atareao/valet)**
mediante su API de ingesta, en modo *best-effort*.

## Instalación

Necesitas una cadena de herramientas reciente para **Rust edición 2024** (Rust
1.85 o posterior) y un token de GitHub con el scope clásico **`repo`**.

```bash
# Desde una copia local (instala en ~/.local/bin, donde lo espera el unit de
# systemd)
cargo install --path . --locked --root "$HOME/.local"

# Directamente desde git
cargo install --git https://github.com/atareao/ghwatch --locked --root "$HOME/.local"
```

Sin `--root`, `cargo install` deja el binario en `~/.cargo/bin/ghwatch`; si lo
prefieres así, ajusta `ExecStart` en `ghwatch.service` en consecuencia. También
puedes ejecutarlo desde el árbol de fuentes con `cargo run --release -- <subcomando>`.

## Inicio rápido

```bash
# 1. Crea la base de datos y la plantilla de configuración (permisos 600)
ghwatch init

# 2. Añade tu token de GitHub (y, si quieres, la ingest key de Valet)
$EDITOR ~/.config/ghwatch/config.toml

# 3. Escanea la cuenta, persiste la ejecución y escribe el informe
ghwatch scan
```

Por defecto la base de datos vive en `~/.local/share/ghwatch/ghwatch.db` y los
informes en `~/.local/share/ghwatch/reports/ghwatch-YYYY-MM-DD.md` y
`...-YYYY-MM-DD.json` (un par por día). El escaneo termina con un resumen de una
línea; usa `--quiet` para silenciar la barra de progreso y el resumen final.

## Subcomandos

| Comando | Qué hace |
|---|---|
| `ghwatch scan` | Escanea todos los repositorios, persiste la ejecución, escribe el informe y, si procede, notifica a Valet. |
| `ghwatch report` | Reemite el último informe local. **Sin red.** |
| `ghwatch diff` | Muestra las alertas nuevas y resueltas de la última ejecución. **Sin red.** |
| `ghwatch history` | Muestra la serie de ejecuciones almacenada (fecha, total, nuevas, resueltas). **Sin red.** |
| `ghwatch repos` | Lista cada repositorio con alertas abiertas, su número de alertas y su desglose por severidad. **Sin red.** |
| `ghwatch notify` | Reenvía a Valet el último informe local sin reescribirlo. |
| `ghwatch init` | Crea la base de datos y la plantilla de configuración, sin sobrescribir las existentes. |

```bash
# Escanear incluyendo forks y repos archivados
ghwatch scan --include-forks --include-archived

# Escanear sin enriquecimiento OSV (más rápido, sin llamadas a terceros)
ghwatch scan --no-enrich

# Reemitir el último informe como JSON por stdout (los logs quedan en stderr)
ghwatch report --format json > informe.json

# Cambios desde la última ejecución, y las últimas 10 ejecuciones
ghwatch diff
ghwatch history --limit 10

# Repositorios con alertas abiertas, de mayor a menor
ghwatch repos --limit 5

# Reenviar el último informe aunque no haya novedades
ghwatch notify --force
```

`report`, `diff`, `history` y `repos` **no tocan GitHub ni Valet**: leen solo la
base de datos local y los ficheros de informe, así que puedes repetirlos sin
conexión. Solo `scan` escanea, y solo `scan`/`notify` pueden contactar con Valet.

Flags de `scan`: `--include-forks`, `--include-archived`, `--no-enrich`,
`--force`, `--quiet`, `-v/--verbose` (repetible), `--db`, `--report-dir`,
`--config`, `--concurrency`, `--github-base-url`, `--osv-base-url`,
`--valet-base-url`. Ejecuta `ghwatch <subcomando> --help` para la lista completa.

`repos` lee solo la base de datos local (ni la crea ni toca la red) e imprime en
**stdout**:

- `--limit <N>`: acota cuántos repositorios se muestran; por defecto `20`, y `0`
  los muestra todos.
- `--all`: los muestra todos (equivale a `--limit 0`); si se combinan, `--all`
  prevalece.
- `--min-severity <critical|high|medium|low|unknown>`: conserva solo los
  repositorios con al menos una alerta de esa severidad **o superior**, sin
  recortar el recuento ni el desglose mostrados (el repositorio aparece entero);
  `unknown` deja pasar todos los repositorios con alertas.
- `--format <console|json|markdown>`: formato de salida, `console` por defecto,
  siempre por stdout (no escribe ningún fichero).

```
$ ghwatch repos --limit 5
ejecución #3 · generado 2026-11-07T08:25:50.008680277+00:00 · repositorios con alertas: 88 de 363 · alertas abiertas: 1330
atareao/reto-python  112  (critical=8 high=63 medium=41 low=0 unknown=0)
atareao/bloc  84  (critical=0 high=46 medium=29 low=9 unknown=0)
atareao/presu  79  (critical=1 high=45 medium=29 low=4 unknown=0)
atareao/tareas  59  (critical=0 high=32 medium=22 low=5 unknown=0)
atareao/asistentes  55  (critical=3 high=19 medium=30 low=3 unknown=0)
```

## Configuración

La configuración de ejecución vive en `~/.config/ghwatch/config.toml` y **debe
conservar permisos `600`**: guarda las credenciales en claro. Si el fichero es
más laxo que `600`, `ghwatch` lo rechaza con un aviso y **no lo usa** (cae al
entorno y a los valores por defecto).

`ghwatch init` escribe esta plantilla (todas las claves comentadas) solo si el
fichero no existe todavía:

```toml
# Configuración de ghwatch.
#
# Este fichero contiene credenciales en claro: debe conservar permisos 600.
# Las variables de entorno tienen prioridad sobre estos valores.
#
# github_token = "ghp_..."
# valet_base_url = "http://localhost:3000"
# valet_ingest_token = "val_..."
# db_path = "~/.local/share/ghwatch/ghwatch.db"
# report_dir = "~/.local/share/ghwatch/reports"
# concurrency = 6
# include_forks = false
# include_archived = false
```

Claves:

| Clave | Tipo | Defecto | Significado |
|---|---|---|---|
| `github_token` | string | — | Credencial de GitHub. Mejor `GITHUB_TOKEN`. |
| `valet_base_url` | string | `http://localhost:3000` | URL base de Valet. |
| `valet_ingest_token` | string | — | Ingest key de Valet (`val_…`). |
| `db_path` | ruta | `~/.local/share/ghwatch/ghwatch.db` | Base de datos SQLite. |
| `report_dir` | ruta | `~/.local/share/ghwatch/reports` | Directorio de informes. |
| `concurrency` | entero | `6` | Concurrencia del cliente de GitHub. |
| `include_forks` | bool | `false` | Incluye los forks en el inventario. |
| `include_archived` | bool | `false` | Incluye los repositorios archivados. |
| `github_base_url` | string | `https://api.github.com` | URL base de la API de GitHub (para pruebas). |
| `osv_base_url` | string | `https://api.osv.dev` | URL base de OSV (para pruebas). |

`valet_base_url`/`valet_ingest_token` también se aceptan con los alias
`base_url`/`ingest_token`. Las claves desconocidas se ignoran. Las rutas que
empiezan por `~` se expanden contra `$HOME`.

Variables de entorno:

| Variable | Sustituye a la clave |
|---|---|
| `GITHUB_TOKEN` | `github_token` |
| `VALET_BASE_URL` | `valet_base_url` |
| `VALET_INGEST_TOKEN` | `valet_ingest_token` |
| `GHWATCH_DB` | `db_path` |
| `GHWATCH_REPORT_DIR` | `report_dir` |
| `GHWATCH_CONCURRENCY` | `concurrency` |
| `GHWATCH_INCLUDE_FORKS` | `include_forks` |
| `GHWATCH_INCLUDE_ARCHIVED` | `include_archived` |
| `GHWATCH_GITHUB_BASE_URL` | `github_base_url` |
| `GHWATCH_OSV_BASE_URL` | `osv_base_url` |
| `RUST_LOG` | Filtro de logs (tiene prioridad sobre `--quiet`/`-v`). |

Los booleanos aceptan `1/true/yes/on` y `0/false/no/off`.

**Precedencia:** flags de línea de comandos > variables de entorno > fichero de
configuración > valores por defecto.

## Credencial de GitHub

El token se resuelve en este orden:

1. La variable de entorno **`GITHUB_TOKEN`**.
2. Si no está, la CLI **`gh`**: `gh auth token`.
3. Si ninguna funciona, un error explicativo y el código de salida `3`.

El scope necesario es el clásico **`repo`**. Los tokens *fine-grained* también
sirven si conceden «Dependabot alerts: read» y «Secret scanning alerts: read»
sobre los repositorios que quieras vigilar. El token no se registra ni se imprime
nunca: se redacta en `Debug`/`Display` y no viaja dentro de ningún error.

## Notificación a Valet

`ghwatch` envía a Valet un resumen *best-effort* mediante su API de ingesta. **No**
modifica Valet en absoluto.

**1. Crea una ingest key.** Las rutas de gestión de claves requieren una sesión
OIDC de Valet; el token se muestra **una sola vez**, así que cópialo enseguida:

```bash
curl -i -X POST http://localhost:3000/api/ingest-keys \
  --cookie "valet_session=<tu-cookie-de-sesión>" \
  -H "Content-Type: application/json" \
  -d '{ "name": "ghwatch" }'
# 201 -> { "id": "…", "name": "ghwatch", "prefix": "val_XXXXXXXX", "token": "val_…" }
```

**2. Guárdala.** Pon el token `val_…` en `~/.config/ghwatch/config.toml`
(`valet_ingest_token`, siempre `600`) o exporta `VALET_INGEST_TOKEN`.

**3. Envía.** `scan` notifica por su cuenta; `notify` reenvía el último informe
local:

```bash
ghwatch scan          # escanea, escribe el informe y notifica si hay novedades
ghwatch scan --force  # notifica aunque no haya novedades
ghwatch notify        # reenvía el último informe, sin escanear
```

Por defecto la notificación sigue la regla **solo-si-hay-cambios**: se envía solo
si `new > 0` o `resolved > 0` (evita el mensaje diario de «no ha pasado nada»).
`--force` fuerza el envío saltándose esa regla. Un fallo de Valet es *best-effort*:
nunca altera el código de salida y el informe en disco ya está escrito. El
`message` se recorta a 4096 bytes (por frontera UTF-8, sin partir caracteres) y el
`metadata` a 8192 bytes, siempre referenciando la ruta del informe.

**Verifica** que la notificación llegó con el buzón (requiere sesión OIDC):

```bash
curl -i "http://localhost:3000/api/notifications/unread?limit=5" \
  --cookie "valet_session=<tu-cookie-de-sesión>"
```

## Códigos de salida

| Código | Significado |
|---|---|
| `0` | Éxito. Incluye «Valet caído» y «sin novedades». |
| `2` | Error de uso (lo emite `clap`). |
| `3` | No hay credencial de GitHub utilizable. |
| `4` | Fallo fatal de la API de GitHub. |
| `5` | Fallo local fatal: base de datos, informe o E/S. |

## Operación

Un **timer de usuario de systemd** ejecuta un escaneo diario a las 07:00
(`Persistent=true`, de modo que una ejecución perdida se recupera al arrancar).
Las unidades viven en `systemd/`:

```bash
mkdir -p ~/.config/systemd/user
cp systemd/ghwatch.service systemd/ghwatch.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ghwatch.timer

# Que corra aunque no tengas la sesión abierta
loginctl enable-linger "$USER"
```

Si el binario no está en `~/.local/bin/ghwatch`, ajusta `ExecStart` en
`ghwatch.service` (por ejemplo `%h/.cargo/bin/ghwatch scan`). Comprueba el timer
y sus trazas con:

```bash
systemctl --user list-timers ghwatch.timer
journalctl --user -u ghwatch
```

El [`justfile`](justfile) envuelve las operaciones habituales:

```bash
just --list        # lista las recetas
just scan          # cargo run --release -- scan
just report --format markdown
just check         # fmt-check + clippy + tests
just build-release # cargo build --release --locked
just install-timer # copia las unidades y activa el timer
just validate      # openspec validate --specs --strict
```

Las recetas de CLI ejecutan `cargo run --release -- …` por defecto; tras
`just install` puedes apuntarlas al binario instalado con
`export GHWATCH_BIN=$(command -v ghwatch)`.

## Desarrollo

La puerta de calidad local es `just check` (o su equivalente directo):

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- **Los tests nunca usan red ni credenciales.** GitHub, OSV y Valet se simulan
  con [`wiremock`](https://crates.io/crates/wiremock); SQLite corre **en memoria**
  (`sqlite::memory:`, una conexión) y las migraciones se embeben con
  `include_str!` y se aplican en runtime (sin `DATABASE_URL` ni macros `query!`).
- **Convención de tests:** cada fichero de `tests/` va envuelto en un módulo con
  su mismo nombre (por ejemplo `mod github { … }` en `tests/github.rs`), de modo
  que `cargo test github` selecciona solo ese grupo.
- **Idioma:** los comentarios del código y los textos de usuario están en
  español; los encabezados estructurales y `SHALL`/`MUST` de la spec se mantienen
  en inglés.

El proyecto se construye con dos puertas de calidad y una disciplina de auditoría:

- **SDD con OpenSpec** — no se escribe código ni tests nuevos antes de que exista
  y esté aprobado un *change* en `openspec/changes/`. `openspec/` es la
  especificación viva: contratos, escenarios *Given/When/Then* y un `tasks.md`.
  `PLAN.md` es el checkpoint arquitectónico y `AGENTS.md` describe el estado
  actual y las invariantes.
- **TDD estricto** — cada tarea se desarrolla en rojo → verde → refactor,
  ejecutando la suite por CLI en cada paso.
- **Auditoría independiente** — cada fase se revisa antes de darse por cerrada, y
  los hallazgos se aplican también con tests.
- **Gitflow con gitmoji** — `main` y `development` solo reciben cambios por
  *pull request*; el trabajo diario va en ramas `feature/*`. Consulta
  [`GIT_FLOW.md`](GIT_FLOW.md) para las ramas, los commits y el flujo de release.

## Arquitectura

```
src/
  main.rs   # entrypoint: configura tracing (stderr) y delega en cli::run
  cli.rs    # clap: scan | report | diff | history | repos | notify | init
  config.rs # config.toml + entorno + precedencia y comprobación de permisos
  auth.rs   # GITHUB_TOKEN -> `gh auth token` -> error tipado
  github/   # cliente HTTP, paginación por Link, cuota y las tres fuentes de alertas
  domain/   # tipos puros: Alert, Severity, ScanRun, Drift, Priority
  scan.rs   # orquestación con concurrencia acotada y progreso
  osv/      # cliente OSV.dev querybatch, mapeo de ecosistemas y fusión
  store/    # migraciones sqlx, upsert idempotente, ciclo de vida y deriva
  report/   # consola, Markdown y JSON versionado, más el listado de repositorios
  notify/   # cliente de ingesta de Valet y coordinación informe/notificación
```

`domain` es puro y testeable sin red ni base de datos, y `store` depende de
`domain` (nunca al revés). El informe se escribe **antes** de notificar; un fallo
de Valet nunca deja un informe a medias. El reconocimiento medido contra la
cuenta del autor encontró **367 repositorios activos** con unas **1.333 alertas
abiertas repartidas en 90 repositorios**.

## Licencia

MIT. Autor: **Lorenzo Carbonell** (`atareao`) · <https://atareao.es>.

Consulta el texto completo en [`LICENSE`](LICENSE).
