# justfile — atajos de desarrollo y operación de ghwatch.
#
# `just` (https://github.com/casey/just) ejecuta las recetas de este fichero.
# `just` a secas, o `just --list`, lista las recetas disponibles.

# Binario que invocan las recetas de CLI. Por defecto compila y ejecuta en modo
# release (`cargo run --release --`). Tras `just install`, exporta
# `GHWATCH_BIN=$(command -v ghwatch)` para usar el binario instalado sin
# recompilar en cada invocación.
bin := env_var_or_default("GHWATCH_BIN", "cargo run --release --")

# Lista las recetas disponibles.
default:
    @just --list

# Compila en modo debug.
build:
    cargo build

# Compila el binario optimizado respetando `Cargo.lock`.
build-release:
    cargo build --release --locked

# Ejecuta la suite completa (sin red ni credenciales).
test:
    cargo test

# Clippy sin warnings.
lint:
    cargo clippy --all-targets -- -D warnings

# Formatea el código.
fmt:
    cargo fmt --all

# Comprueba el formato sin reescribir ficheros.
fmt-check:
    cargo fmt --all --check

# Puerta de calidad local: formato + lint + tests (lo que corre la CI).
check: fmt-check lint test

# Escanea la cuenta y persiste el resultado (admite los flags de `scan`).
scan *ARGS:
    {{bin}} scan {{ARGS}}

# Reemite el último informe local en el formato pedido (sin red).
report *ARGS:
    {{bin}} report {{ARGS}}

# Muestra las alertas nuevas y resueltas del último run (sin red).
diff *ARGS:
    {{bin}} diff {{ARGS}}

# Muestra la serie histórica almacenada (sin red).
history *ARGS:
    {{bin}} history {{ARGS}}

# Crea la base de datos y la plantilla de configuración.
init *ARGS:
    {{bin}} init {{ARGS}}

# Ayuda del binario.
help:
    {{bin}} --help

# Valida las especificaciones OpenSpec archivadas (`openspec/specs/`).
validate:
    openspec validate --specs --strict

# Instala el timer y el service de usuario de systemd.
install-timer:
    mkdir -p "$HOME/.config/systemd/user"
    cp systemd/ghwatch.service systemd/ghwatch.timer "$HOME/.config/systemd/user/"
    systemctl --user daemon-reload
    systemctl --user enable --now ghwatch.timer
    @echo 'Instalado. Recuerda habilitar el linger: loginctl enable-linger "$USER"'

# Desinstala el timer y el service de usuario.
uninstall-timer:
    -systemctl --user disable --now ghwatch.timer
    rm -f "$HOME/.config/systemd/user/ghwatch.service" "$HOME/.config/systemd/user/ghwatch.timer"
    -systemctl --user daemon-reload

# Instala el binario en `~/.local/bin` (lo que espera el unit de systemd).
install:
    cargo install --path . --locked --root "$HOME/.local"

# ── Releases (ver GIT_FLOW.md) ──────────────────────────────────
# El release se prepara en local: bump con `vampus`, changelog con `git-cliff`
# y commit de release. El push del tag se hace tras mergear a `main`.

# Sube la versión con vampus (`just bump --minor`, `--patch` o `--major`).
bump *ARGS:
    vampus upgrade {{ARGS}}

# Muestra la versión actual declarada en `.vampus.yml`/`Cargo.toml`.
version:
    vampus show

# Genera `CHANGELOG.md` con lo aún no publicado (sin cerrar versión).
changelog:
    git-cliff -o CHANGELOG.md

# Genera `CHANGELOG.md` cerrando la versión `VERSION` (p. ej. `v0.2.0`).
changelog-tag VERSION:
    git-cliff -o CHANGELOG.md --tag {{VERSION}}

# Prepara la release: rama `release/VERSION`, changelog y commit (sin push ni tag).
release-prepare VERSION:
    git checkout -b release/{{VERSION}}
    git-cliff -o CHANGELOG.md --tag {{VERSION}}
    git add Cargo.toml Cargo.lock .vampus.yml CHANGELOG.md
    git commit -m "🔧 chore(release): {{VERSION}}"
