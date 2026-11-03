# ghwatch

Vigilancia de vulnerabilidades en los repositorios GitHub de una cuenta: alertas de
Dependabot, secret scanning y enriquecimiento con OSV.dev, con histórico entre
ejecuciones y un informe de situación actual y evolución.

[![CI](https://github.com/atareao/ghwatch/actions/workflows/ci.yml/badge.svg)](https://github.com/atareao/ghwatch/actions/workflows/ci.yml)
[![Licencia: MIT](https://img.shields.io/badge/licencia-MIT-blue.svg)](https://github.com/atareao/ghwatch/blob/main/LICENSE)

> [!WARNING]
> **Proyecto en desarrollo. El binario todavía no hace nada útil.**
>
> De las secciones de trabajo 0 a 7, están completadas la 0 (aprobación de la
> especificación) y las fases 1 y 2. Lo que funciona hoy es **solo la
> biblioteca**:
>
> - `cargo install ghwatch` **no** da un CLI usable: instalaría un binario cuyo
>   `main` es un stub que imprime `Hello, world!`. No hay `clap`, ni subcomandos,
>   ni banderas, ni base de datos, ni informes.
> - No hay todavía escaneo, ni persistencia, ni informes, ni notificación.
> - Lo implementado y auditado es el núcleo reutilizable: resolución de la
>   credencial (`auth`) y cliente HTTP de GitHub (`github`), con **78 tests** que
>   pasan sin red y sin token.
>
> Este documento describe el **objetivo** del producto y, cuando corresponde, lo
> que ya existe. Cada apartado indica a qué se refiere.

## Qué hará

El producto final es un CLI en Rust, `ghwatch`, que recorrerá **todos** los
repositorios GitHub de una cuenta (para la cuenta `atareao` se midieron **433**,
de los que **367** están activos) y:

- recogerá las **alertas de Dependabot** (se midieron ~**1.333** alertas abiertas
  repartidas en 90 repositorios) paginando de forma completa —nunca truncando en
  el tamaño de página—;
- recogerá las **alertas de secret scanning** (se midieron 3 abiertas ya en el
  reconocimiento) y las de **code scanning** cuando CodeQL esté configurado;
- **enriquecerá** cada alerta con OSV.dev (CVSS, alias CVE y versión que corrige)
  en lotes y sin sobreescribir los datos que ya aporta GitHub;
- guardará un **histórico en SQLite** por ejecución y calculará la **evolución**
  entre escaneos (nuevas, resueltas, reabiertas, subidas de severidad);
- producirá un **informe de situación actual y evolución** (consola, Markdown y
  JSON versionado) escrito siempre a disco;
- **notificará opcionalmente a Valet**, el asistente self-hosted del autor,
  mediante su API de ingesta ya existente, en modo *best-effort*.

## Estado por fases

La especificación viva está en `openspec/changes/ghwatch-scanner/` y el
checkpoint arquitectónico en `PLAN.md`.

| Sección | Estado | Contenido |
|---|---|---|
| 0 · Aprobación | ✅ Hecha | `proposal.md` + `design.md` + `tasks.md` aprobados antes de escribir código |
| 1 · Andamiaje y autenticación | ✅ Hecha | crate `ghwatch`, dependencias, higiene, y el módulo `auth` (más el endurecimiento 1.9) |
| 2 · Cliente GitHub | ✅ Hecha | `github/`: cabeceras, paginación por `Link`, control de *rate limit*, errores tipados (más el endurecimiento 2.9) |
| 3 · Inventario y recolección | ⬜ Pendiente | descubrimiento y filtrado de repositorios, alertas de las tres fuentes, orquestación con concurrencia acotada |
| 4 · Persistencia e histórico | ⬜ Pendiente | migraciones sqlx, upsert idempotente, cálculo de deriva, `history` y `diff` |
| 5 · Enriquecimiento OSV y priorización | ⬜ Pendiente | cliente OSV por lotes, mapeo de ecosistemas, fusión sin sobreescritura, priorización determinista |
| 6 · Informes y notificación a Valet | ⬜ Pendiente | salida de consola, Markdown, JSON versionado, notificador a Valet |
| 7 · Operación | ⬜ Pendiente | unidades systemd de usuario, `justfile`, CI, gitflow y verificación final |

## Arquitectura

### Módulos (lo que existe hoy)

```
src/
  main.rs          # stub actual: imprime "Hello, world!"
  lib.rs           # raíz de la biblioteca (expone `auth` y `github`)
  auth.rs          # resolución del token: GITHUB_TOKEN → `gh auth token` → error tipado
  github/
    mod.rs         # reexporta el cliente, la configuración y los errores
    client.rs      # cliente reqwest: `get_json` y `get_paginated`, cabeceras y control de cuota
    error.rs       # `GithubError` tipado (incluye `Disabled` y `NotConfigured`)
    pagination.rs  # lectura pura de la cabecera `Link` (`rel="next"`)
    ratelimit.rs   # aritmética pura de `X-RateLimit-*`, `Retry-After` y retroceso exponencial
tests/
  auth.rs          # integración de `auth` con dobles (sin procesos externos)
  github.rs        # integración del cliente con `wiremock` (sin red)
```

Una frase por módulo:

- **`auth`** — obtiene la credencial en el orden `GITHUB_TOKEN` → `gh auth token`
  → error explicativo, y redacta el token tanto en `Debug`/`Display` como en los
  mensajes de error; el valor solo se obtiene por `Token::expose_secret()`.
- **`github::client`** — cliente HTTP sobre `reqwest` que añade las cabeceras
  exigidas por la API, pagina siguiendo el `Link`, aplica pausa preventiva y
  reintentos, y acota la concurrencia con un semáforo.
- **`github::error`** — errores tipados; **nunca** transportan el token.
- **`github::pagination`** — extracción pura (sin IO) de la URL `rel="next"` de
  la cabecera `Link`.
- **`github::ratelimit`** — aritmética pura del límite de peticiones: cuota
  restante, instante de reposición, `Retry-After` y retroceso exponencial.

### Módulos previstos (aún no escritos)

`src/cli.rs`, `src/config.rs`, y los módulos `github/{repos,dependabot,secret_scanning,code_scanning}`,
`osv/`, `store/`, `domain/`, `report/` y `notify/`. Están planificados en
`PLAN.md`, no implementados.

### Stack

`Rust` (edición 2024) · `tokio` · `reqwest` con `rustls` · `sqlx` sobre SQLite
(consultas en tiempo de ejecución, sin `DATABASE_URL` en compilación) · `clap`
(derive) · `comfy-table` · `indicatif` · `serde` / `serde_json` · `thiserror`
(con `anyhow` para el binario) · `tracing` / `tracing-subscriber` · `chrono` ·
`wiremock` solo para los tests.

## Cómo usarlo hoy

Hoy **no** hay un binario útil, así que no hay nada que instalar. Lo que se puede
ejecutar es la biblioteca y su suite de tests, que no necesitan red ni token:

```bash
# 78 tests (53 unitarios + 10 de integración de `auth` + 15 de integración de `github`)
cargo test

# Higiene: sin warnings de Clippy y formato comprobado
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

> **No** ejecutes `cargo install ghwatch` esperando un escáner: instalaría un
> binario que solo imprime `Hello, world!`. El CLI llegará en fases posteriores;
> hoy esta biblioteca es la base sobre la que se construirá.

## Credenciales

Cuando exista el CLI, resolverá la credencial de GitHub en este orden (es lo que
ya implementa `auth`):

1. La variable de entorno **`GITHUB_TOKEN`**.
2. Si no está, la CLI **`gh`**: `gh auth token`.
3. Si ninguna funciona, un error que explica las dos vías.

El permiso que necesita lo implementado es el scope clásico **`repo`** (el token
usado durante el reconocimiento tenía `admin:public_key, gist, read:org, repo`).
Las cabeceras de autenticación se construyen en un único punto, y el **token nunca
se registra ni se imprime**: `Token` redacta su valor en `Debug` y `Display`, y
ningún error (`AuthError`, `GithubError`) puede contenerlo. Hay tests específicos
que verifican que ni el `Debug` del cliente, ni el de las estructuras que lo
envuelven, ni los mensajes de error filtran un secreto.

## Notas de implementación

Detalles medidos contra la API real y ya resueltos en la biblioteca:

- **La paginación es por cursor.** El `Link: rel="next"` de Dependabot **no** es
  la misma ruta con `page=2`: cambia el prefijo
  (`/repos/{owner}/{repo}/...` → `/repositories/{id}/...`) y añade un cursor
  opaco `after=<base64>`. Por eso el cliente sigue la URL del `Link` **tal cual**,
  sin recomponerla; recomponerla perdería la página siguiente.
- **Un `403` puede ser «capacidad no configurada», no un fallo.** Un repositorio
  con Dependabot deshabilitado responde `403` con
  `{"message":"Dependabot alerts are disabled for this repository."}`; se
  clasifica como `GithubError::Disabled` (por el cuerpo) y no aborta el escaneo.
  Un `403` de límite sigue trayendo `Retry-After` y se trata distinto.
- **Un `404` puede significar «no configurado».** `code-scanning/alerts`
  responde `404` con `{"message":"no analysis found"}` cuando no hay CodeQL; se
  clasifica como `GithubError::NotConfigured`, no como error.

## Metodología

El proyecto se construye con dos puertas de calidad y una disciplina de ramas:

- **SDD con OpenSpec.** No se escribe código ni tests nuevos antes de que exista
  y esté aprobado un *change* en `openspec/changes/`. `openspec/` es la
  especificación viva: contratos, escenarios *Given/When/Then* y un `tasks.md`.
- **TDD estricto.** Cada tarea se desarrolla en rojo → verde → refactor,
  ejecutando la suite por CLI en cada paso.
- **Sin red en los tests.** GitHub se simula con `wiremock`; la suite es
  determinista y ejecutable sin credenciales.
- **Auditoría independiente.** Cada fase la revisa un revisor `@rust-reviewer`
  antes de darse por cerrada; los hallazgos se aplican también con TDD.
- **Gitflow con gitmoji.** `main` (producción) y `development` (integración) solo
  reciben cambios por *pull request*; el trabajo diario va en ramas `feature/*`
  con *conventional commits* y emoji.

## Roadmap

- **Fase 3 · Inventario y recolección** — descubrir y filtrar los 367 repositorios
  activos y recolectar las alertas de Dependabot, secret scanning y code scanning
  normalizadas a un modelo único, con orquestación concurrente y progreso.
- **Fase 4 · Persistencia e histórico** — esquema SQLite (4 tablas), *upsert*
  idempotente, ciclo de vida de cada alerta y cálculo de deriva; subcomandos
  `history` y `diff`.
- **Fase 5 · Enriquecimiento OSV y priorización** — cliente OSV por lotes, mapeo
  de ecosistemas (Dependabot → OSV), fusión sin sobreescritura y priorización
  determinista.
- **Fase 6 · Informes y notificación a Valet** — salida de consola, Markdown
  (situación actual + evolución + acciones) y JSON versionado, y entrega
  *best-effort* a Valet.
- **Fase 7 · Operación** — unidades systemd de usuario, `justfile`, la
  verificación de la CI (el workflow básico ya está en el repositorio; la fase
  cubre su validación) y la verificación final contra los criterios de
  aceptación.

## Licencia

MIT. Autor: **Lorenzo Carbonell** (`atareao`) · <https://atareao.es>.

Consulta el texto completo en [`LICENSE`](LICENSE).
