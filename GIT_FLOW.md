# Git Flow

`ghwatch` sigue **Git Flow** con versionado semántico gestionado por
[`vampus`](https://github.com/atareao/vampus) y changelog generado con
[`git-cliff`](https://git-cliff.org). `main` y `development` **solo** reciben
cambios por *pull request*; nunca se hace commit directo en ellas.

## Ramas

| Rama | Propósito | Base |
|---|---|---|
| `main` | Producción. Cada tag `vX.Y.Z` aquí dispara la GitHub Release. | — |
| `development` | Integración de las features en curso. | `main` |
| `feature/*` | Nuevas funcionalidades. | `development` |
| `hotfix/*` | Correcciones urgentes a producción. | `main` |
| `release/*` | Preparación de una release (bump + changelog). | `development` |

## Flujo diario

### Features

```bash
git checkout development && git pull origin development
git checkout -b feature/mi-feature
git commit -m "✨ feat: add dark mode toggle"
git push -u origin feature/mi-feature
# Abrir Pull Request en GitHub contra `development`
```

### Hotfixes

```bash
git checkout main && git pull origin main
git checkout -b hotfix/arreglo-critico
git commit -m "🚑️ hotfix: crash on empty input"
git push -u origin hotfix/arreglo-critico
# PR contra `main`; tras el merge, sincronizar `development`:
git checkout development && git merge main && git push origin development
```

### Releases

El release se **prepara en local** (no hay workflow que haga el bump en CI: el
repositorio no tiene el secreto `GH_PAT`). La CI de release solo construye y
publica los binarios a partir del tag.

```bash
# 1. Rama de release desde development
git checkout development && git pull origin development
git checkout -b release/vX.Y.Z

# 2. Subir la versión (elige patch | minor | major según los commits)
vampus upgrade --minor          # actualiza `version` en Cargo.toml y .vampus.yml

# 3. Generar el changelog para el tag nuevo
git-cliff -o CHANGELOG.md --tag vX.Y.Z

# 4. Commit de release
git add Cargo.toml Cargo.lock .vampus.yml CHANGELOG.md
git commit -m "🔧 chore(release): vX.Y.Z"
git push -u origin release/vX.Y.Z

# 5. PR de release/vX.Y.Z -> main y merge
# 6. Ya en main (tras el merge), crear y empujar el tag:
git checkout main && git pull origin main
git tag vX.Y.Z
git push origin vX.Y.Z          # dispara .github/workflows/release.yml

# 7. Sincronizar development con main
git checkout development && git merge main && git push origin development
```

Al empujar el tag, `release.yml` compila `ghwatch` para `x86_64-unknown-linux-gnu`
y `aarch64-unknown-linux-gnu`, empaqueta `ghwatch-<target>.tar.gz` y publica la
GitHub Release con notas generadas automáticamente.

También hay recetas de `just` que automatizan parte del flujo:

```bash
just bump --minor            # vampus upgrade --minor
just changelog               # git-cliff -o CHANGELOG.md
just release-prepare v0.2.0  # rama release/v0.2.0 + changelog + commit de release
```

## Conventional Commits con Gitmoji

El formato determina el bump de versión:

| Mensaje | Bump |
|---|---|
| `✨ feat: ...` | minor (`0.Y.0`) |
| `🐛 fix: ...` | patch (`0.0.Z`) |
| `♻️ refactor: ...` | patch (`0.0.Z`) |
| `📝 docs: ...` | patch (`0.0.Z`) |
| `💥 feat!: ...` o `BREAKING CHANGE` | major (`X.0.0`) |

### Gitmoji de referencia

| Tipo | Emoji |
|---|---|
| feat | ✨ |
| fix | 🐛 |
| hotfix | 🚑️ |
| docs | 📝 |
| refactor | ♻️ |
| perf | ⚡ |
| style | 💄 |
| test | ✅ |
| chore | 🔧 |
| ci | 👷 |
| revert | ⏪️ |
| deps | ⬆️ |
| breaking | 💥 |

## CI

| Workflow | Disparador | Qué hace |
|---|---|---|
| `ci.yml` | push/PR a `main` y `development` | `fmt` → `clippy` → `test` → build release `--locked` |
| `release.yml` | tag `v*` | compila los dos targets nativos y publica la GitHub Release |

## Secretos de GitHub

**Este repositorio no necesita ningún secreto para su CI actual.** `ci.yml` y
`release.yml` usan el `GITHUB_TOKEN` que GitHub inyecta automáticamente
(`permissions: contents: write` basta para crear la release).

- `GH_PAT` solo haría falta si algún día se automatizara el bump de versión en
  CI (un `release-prepare.yml` que empujara el commit y el tag). Hoy el release
  se prepara en local, así que no se usa.
- `CARGO_REGISTRY_TOKEN` solo haría falta si algún día se publicara el crate en
  crates.io. Hoy no se publica y no existe el job `publish`.

## Resumen visual

```
main        ──hotfix──●────────────●────────────────●
                       \            /                /
development ──────────●──●──●────●──●──●──●────────●
                        \ /        \   /
feature     ──────────●  feature ──●
                      feature/foo   feature/bar

● = merge vía PR
● = tag vX.Y.Z en main (release)
```
