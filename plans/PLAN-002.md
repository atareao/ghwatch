# PLAN-002 — CI: actualizar las acciones al runtime Node.js 24

> Plan de la tarea de mantenimiento (chore) de la CI. **No hay change OpenSpec**:
> ningún capability de `openspec/specs/` cambia; es configuración operativa
> (`.github/workflows/`), como el `justfile` o las unidades de systemd.

## Objetivo

GitHub Actions deprecó el runtime Node.js 20. Las acciones fijadas en los
workflows lo usan y emiten avisos de deprecación; hay que subirlas a versiones
que corran sobre Node.js 24.

## Diagnóstico (runtime real de cada pin, leído del `action.yml` del tag)

| Acción | Pin actual | Runtime | Destino |
|---|---|---|---|
| `actions/checkout` | `@v4` | node20 | `@v7` (node24) |
| `actions/upload-artifact` | `@v4` | node20 | `@v7` (node24) |
| `actions/download-artifact` | `@v4` | node20 | `@v8` (node24) |
| `softprops/action-gh-release` | `@v2` | node20 | `@v3` (node24) |
| `Swatinem/rust-cache` | `@v2` | node24 | sin cambios |
| `dtolnay/rust-toolchain` | `@stable` | composite | sin cambios |

Notas:

- `softprops/action-gh-release@v2.6.2` es la **última** de la línea v2 y su
  README la declara sin mantenimiento por usar Node 20.
- Las versiones destino conservan los mismos `inputs` que usamos.
- Node 24 exige un runner ≥ `2.327.1`; los runners hosted lo cumplen.

## Alcance

- `.github/workflows/ci.yml`: `actions/checkout@v4` → `@v7`.
- `.github/workflows/release.yml`: `actions/checkout@v4` → `@v7`,
  `actions/upload-artifact@v4` → `@v7`, `actions/download-artifact@v4` → `@v8`,
  `softprops/action-gh-release@v2` → `@v3`.

## Verificación

- YAML válido (`yq e '.' <fichero> >/dev/null`).
- No queda ninguna acción fijada a un runtime Node 20.
- La primera prueba real del `release.yml` es la release v0.2.0 (item 1).

## Cierre

Al terminar: renombrar este plan a `plans/PLAN-002-ARCHIVED.md`.
